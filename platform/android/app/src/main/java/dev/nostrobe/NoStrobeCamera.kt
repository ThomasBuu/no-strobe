package dev.nostrobe

import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.graphics.Rect
import android.hardware.camera2.CameraMetadata
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.params.MeteringRectangle
import android.hardware.camera2.CaptureResult
import android.hardware.camera2.TotalCaptureResult
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.SessionConfiguration
import android.os.Build
import android.media.MediaRecorder
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.os.SystemClock
import android.util.Log
import java.io.Writer
import java.util.Locale
import android.util.Size
import android.view.Surface
import kotlin.math.abs

/**
 * Camera2 pipeline: preview + YUV analysis + recording, with the shutter driven by the
 * Rust controller.
 *
 * The repeating request is reissued only when the controller moves the shutter, so the
 * new exposure takes effect on the next frame without a request per frame.
 */
class NoStrobeCamera(
    private val manager: CameraManager,
    private val cameraId: String,
    private val previewSurface: Surface,
    private val recorder: MediaRecorder?,
    private val pwmMode: Boolean,
    /** Pins one lens of a logical multi-camera; null leaves the lens choice to the phone. */
    private val physicalId: String? = null,
    private var zoomRatio: Float = 1f,
    /** Exposure compensation in EV (stops), from the exposure bar. */
    private var evBias: Float = 0f,
    /** Called once the capture session is configured and the repeating request runs. */
    private val onReady: () -> Unit = {},
    private val onStatus: (NsCommand) -> Unit,
) {
    private var device: CameraDevice? = null
    private var session: CameraCaptureSession? = null
    private var reader: ImageReader? = null
    private var strobe: SyncController? = null
    private var pwm: PwmSyncController? = null
    private var builder: android.hardware.camera2.CaptureRequest.Builder? = null
    private var last: NsCommand? = null
    private var sustainUs = 0.0
    private var lineRateHz = 0.0
    private var handler: Handler? = null
    private var analysisThread: HandlerThread? = null

    // The sensor pipeline is a few frames deep: after a new exposure is requested, the next
    // images still carry the old one. The controller measures residual banding per trial
    // exposure, so a frame is only fed to it if the exposure the sensor really used
    // (CaptureResult, matched by SENSOR_TIMESTAMP) is the one last requested.
    private var requestedExposureNs = 0L
    private var latestResultExposureNs = 0L
    private val resultExposure = HashMap<Long, Long>()
    private var staleFrames = 0
    private var lastAfState: Int? = null
    @Volatile private var stopped = false
    // The device cannot (or does not) do manual exposure: the camera stays on auto-exposure
    // and the app only records; the clip is fixed afterwards.
    private var autoOnly = false

    // No flicker found: do not leave the sensor on short exposures and a maxed ISO (a dark,
    // noisy clip). After PROBE_MS without a lock the camera goes back to auto-exposure for
    // REST_MS, then probes again (the lights may have changed).
    private var surfaceList: List<Surface> = emptyList()
    // The lens actually pinned by the session that is running (null: the logical camera).
    private var activePhysicalId: String? = null
    private var analysisSurface: Surface? = null
    private var probeSinceMs = 0L
    private var restUntilMs = 0L

    // Per-frame log written while recording (one line per analysed frame, plus the frames
    // dropped as stale). Live correction is not possible with MediaRecorder, so the clip is
    // fixed afterwards (tools/postprocess.sh) and this log says how much of it needs fixing.
    @Volatile var meta: Writer? = null
    private val statsLock = Any()
    private var recFrames = 0
    private var recLocked = 0

    fun recordingStats(): Pair<Int, Int> = synchronized(statsLock) { recFrames to recLocked }

    fun closeMeta() = synchronized(statsLock) {
        try { meta?.flush(); meta?.close() } catch (_: java.io.IOException) {}
        meta = null
    }

    private fun recordFrame(ts: Long, expNs: Long, c: NsCommand?, luma: Double, stale: Boolean) {
        val w = meta ?: return
        synchronized(statsLock) {
            if (!stale) {
                recFrames += 1
                if (c?.state == NsJni.STATE_LOCKED) recLocked += 1
            }
            val hz = if (c != null && c.periodUs > 0) 1e6 / c.periodUs else 0.0
            try {
                w.write(String.format(Locale.US, "%d,%d,%d,%.4f,%d,%.1f,%d\n",
                    ts, expNs, c?.iso?.toInt() ?: 0, luma, c?.state ?: -1, hz, if (stale) 1 else 0))
            } catch (_: java.io.IOException) {}
        }
    }

    // Brightness budget = exposure (us) * ISO, metered by the camera's own auto-exposure
    // before the shutter goes manual. A shorter exposure then gets a proportionally higher
    // ISO, so every trial of the closed loop sees the same image brightness (the residual
    // banding amplitude is only comparable between trials at equal brightness).
    private var budgetUsIso = 0.0
    private var latestAutoBudget = 0.0
    private var autoWait = 0
    private var isoMin = 100.0
    private var isoMax = 6400.0
    private val resultCallback = object : CameraCaptureSession.CaptureCallback() {
        override fun onCaptureCompleted(
            session: CameraCaptureSession,
            request: CaptureRequest,
            result: TotalCaptureResult,
        ) {
            val af = result.get(CaptureResult.CONTROL_AF_STATE)
            if (af != lastAfState) {
                lastAfState = af
                when (af) {
                    CaptureResult.CONTROL_AF_STATE_FOCUSED_LOCKED -> Log.i(TAG, "focus: locked")
                    CaptureResult.CONTROL_AF_STATE_NOT_FOCUSED_LOCKED -> Log.i(TAG, "focus: could not lock")
                    else -> Log.d(TAG, "af state $af")
                }
            }
            val ts = result.get(CaptureResult.SENSOR_TIMESTAMP) ?: return
            val e = result.get(CaptureResult.SENSOR_EXPOSURE_TIME) ?: return
            if (requestedExposureNs == 0L) {
                val iso = result.get(CaptureResult.SENSOR_SENSITIVITY)
                val ae = result.get(CaptureResult.CONTROL_AE_STATE)
                if (iso != null) {
                    latestAutoBudget = (e / 1000.0) * iso
                    if (ae == CaptureResult.CONTROL_AE_STATE_CONVERGED) {
                        budgetUsIso = latestAutoBudget
                    }
                }
            }
            synchronized(resultExposure) {
                latestResultExposureNs = e
                resultExposure[ts] = e
                if (resultExposure.size > 16) {
                    val oldest = resultExposure.keys.minOrNull()
                    if (oldest != null) resultExposure.remove(oldest)
                }
            }
        }
    }

    /** Exposure the sensor used for the frame with this timestamp, or the latest known. */
    private fun actualExposureNs(timestamp: Long): Long = synchronized(resultExposure) {
        resultExposure[timestamp] ?: latestResultExposureNs
    }

    private fun exposureMatches(actualNs: Long, requestedNs: Long): Boolean {
        if (requestedNs <= 0L) return true
        // Sensors quantise exposure to a multiple of the row time: allow 3 % or 60 us.
        val tol = maxOf(requestedNs * 3 / 100, 60_000L)
        return abs(actualNs - requestedNs) <= tol
    }

    fun start() {
        stopped = false
        autoOnly = pwmMode && !Camera2Sync.supportsManualSensor(manager.getCameraCharacteristics(cameraId))
        if (autoOnly) {
            Log.w(TAG, "camera $cameraId has no MANUAL_SENSOR capability: flicker sync unavailable")
            onStatus(unsupportedStatus())
        }
        // A pinned lens has its own sensor: exposure range, readout time and stream sizes.
        val chars = manager.getCameraCharacteristics(physicalId ?: cameraId)
        val limits = Camera2Sync.limitsFromCharacteristics(chars)
        sustainUs = limits.sustainUs
        Log.i(TAG, "limits: exposure %.0f..%.0f us, sustain %.0f us, iso %.0f..%.0f, fps %s".format(
            limits.minExposureUs, limits.maxExposureUs, limits.sustainUs, limits.minIso, limits.maxIso,
            limits.supportedFps.joinToString(",")))
        isoMin = limits.minIso
        isoMax = limits.maxIso

        if (pwmMode) {
            // The controller is built on the first frame: the line rate needs the stream
            // height, which is only known once an image arrives.
            pwm = null
        } else {
            strobe = NsJni.create(limits)
        }

        val map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        // A modest analysis size keeps the per-frame row profile cheap; the preview is a
        // separate surface and does not need the full resolution.
        val size = map?.getOutputSizes(ImageFormat.YUV_420_888)
            ?.filter { it.height <= 720 }
            ?.maxBy { it.width.toLong() * it.height }
            ?: Size(1280, 720)

        // The listener must not run on the main thread: the row profile and the JNI call
        // per frame freeze the UI, and the preview lags.
        val thread = HandlerThread("analysis").apply { start() }
        analysisThread = thread
        handler = Handler(thread.looper)
        val r = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 3)
        r.setOnImageAvailableListener({ reader -> onFrame(reader) }, handler)
        reader = r
        analysisSurface = r.surface

        val surfaces = listOf(previewSurface, r.surface) +
            (recorder?.surface?.let { listOf(it) } ?: emptyList())
        surfaceList = surfaces

        manager.openCamera(cameraId, object : CameraDevice.StateCallback() {
            override fun onOpened(d: CameraDevice) {
                // The camera opens asynchronously: the activity may have been paused (screen
                // off) and the preview surface destroyed meanwhile.
                if (stopped || surfaces.any { !it.isValid }) {
                    d.close()
                    return
                }
                device = d
                try {
                    createSession(d, surfaces)
                } catch (e: IllegalArgumentException) {
                    Log.w(TAG, "session not created: ${e.message}")
                    d.close(); device = null
                }
            }

            override fun onDisconnected(d: CameraDevice) {
                d.close()
                device = null
            }

            override fun onError(d: CameraDevice, e: Int) {
                d.close()
                device = null
            }
        }, null)
    }

    private fun applyBias(b: CaptureRequest.Builder) {
        // Only used while the camera's auto-exposure runs (before probing, and in the rest
        // periods). In manual exposure the bias scales the ISO budget instead.
        val chars = manager.getCameraCharacteristics(cameraId)
        val step = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_STEP)?.toFloat() ?: return
        val range = chars.get(CameraCharacteristics.CONTROL_AE_COMPENSATION_RANGE) ?: return
        if (step <= 0f) return
        b.set(CaptureRequest.CONTROL_AE_EXPOSURE_COMPENSATION, Math.round(evBias / step).coerceIn(range.lower, range.upper))
    }

    /** Exposure bar: takes effect on the next frame, without touching the flicker lock. */
    fun setExposureBias(ev: Float) {
        evBias = ev
        last = null  // the next analysed frame re-applies the ISO with the new bias
        val b = builder ?: return
        applyBias(b)
        try {
            session?.setRepeatingRequest(b.build(), resultCallback, handler)
        } catch (e: Exception) {
            Log.w(TAG, "exposure bias not applied: ${e.message}")
        }
    }

    private fun applyZoom(b: CaptureRequest.Builder) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            b.set(CaptureRequest.CONTROL_ZOOM_RATIO, zoomRatio)
        }
    }

    /**
     * Tap to focus at a point of the preview (0..1 in the upright view). The point is turned
     * into sensor coordinates (rotation, then the zoom crop), autofocus is triggered once
     * and then held; continuous autofocus resumes after FOCUS_HOLD_MS.
     */
    fun focusAt(nx: Float, ny: Float) {
        val b = builder ?: return
        val sess = session ?: return
        val chars = manager.getCameraCharacteristics(cameraId)
        val active = chars.get(CameraCharacteristics.SENSOR_INFO_ACTIVE_ARRAY_SIZE) ?: return
        val orientation = chars.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
        val (sx, sy) = when (orientation) {
            90 -> ny to (1f - nx)
            270 -> (1f - ny) to nx
            180 -> (1f - nx) to (1f - ny)
            else -> nx to ny
        }
        // What the user sees is the zoom crop, centred on the active array.
        val z = zoomRatio.coerceAtLeast(0.1f)
        val cw = active.width() / z
        val ch = active.height() / z
        val px = active.exactCenterX() - cw / 2f + sx * cw
        val py = active.exactCenterY() - ch / 2f + sy * ch
        val half = (minOf(active.width(), active.height()) / 10f).toInt()
        val rect = Rect(
            (px - half).toInt().coerceIn(active.left, active.right - 1),
            (py - half).toInt().coerceIn(active.top, active.bottom - 1),
            (px + half).toInt().coerceIn(active.left + 1, active.right),
            (py + half).toInt().coerceIn(active.top + 1, active.bottom),
        )
        b.set(CaptureRequest.CONTROL_AF_REGIONS, arrayOf(MeteringRectangle(rect, MeteringRectangle.METERING_WEIGHT_MAX - 1)))
        b.set(CaptureRequest.CONTROL_AF_MODE, CameraMetadata.CONTROL_AF_MODE_AUTO)
        b.set(CaptureRequest.CONTROL_AF_TRIGGER, CameraMetadata.CONTROL_AF_TRIGGER_START)
        try {
            sess.capture(b.build(), focusCallback, handler)
            // The trigger is one-shot: the held request must not carry it again.
            b.set(CaptureRequest.CONTROL_AF_TRIGGER, CameraMetadata.CONTROL_AF_TRIGGER_IDLE)
            sess.setRepeatingRequest(b.build(), resultCallback, handler)
            handler?.removeCallbacks(resumeContinuousFocus)
            handler?.postDelayed(resumeContinuousFocus, FOCUS_HOLD_MS)
        } catch (e: Exception) {
            Log.w(TAG, "focus not applied: ${e.message}")
        }
    }

    private val resumeContinuousFocus = Runnable {
        val b = builder ?: return@Runnable
        b.set(CaptureRequest.CONTROL_AF_MODE, CameraMetadata.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
        b.set(CaptureRequest.CONTROL_AF_REGIONS, null)
        try { session?.setRepeatingRequest(b.build(), resultCallback, handler) } catch (_: Exception) {}
    }

    private val focusCallback = object : CameraCaptureSession.CaptureCallback() {
        override fun onCaptureCompleted(s: CameraCaptureSession, r: CaptureRequest, result: TotalCaptureResult) {
            when (result.get(CaptureResult.CONTROL_AF_STATE)) {
                CaptureResult.CONTROL_AF_STATE_FOCUSED_LOCKED -> Log.i(TAG, "focus: locked")
                CaptureResult.CONTROL_AF_STATE_NOT_FOCUSED_LOCKED -> Log.i(TAG, "focus: could not lock")
                else -> {}
            }
        }
    }

    /** Zoom without restarting the session: the repeating request is reissued. */
    fun setZoom(ratio: Float) {
        zoomRatio = ratio
        val b = builder ?: return
        applyZoom(b)
        try {
            session?.setRepeatingRequest(b.build(), resultCallback, handler)
        } catch (e: Exception) {
            Log.w(TAG, "zoom not applied: ${e.message}")
        }
    }

    /**
     * Opens the capture session, trying simpler stream combinations when the device refuses
     * one ("Unsupported set of inputs/outputs"; seen on a Sony Xperia 1 III with preview +
     * analysis + recorder on a pinned lens):
     *   0  the requested lens pinned, every stream
     *   1  the logical camera (the phone picks the lens), every stream
     *   2  no analysis stream: preview + recorder only, no flicker sync (post-processing)
     *   3  preview only: the clip cannot be recorded on this device
     */
    private fun createSession(d: CameraDevice, all: List<Surface>, level: Int = 0) {
        if (stopped) return
        val pin = if (level == 0) physicalId else null
        activePhysicalId = pin
        val surfaces = when {
            level >= 3 -> listOf(previewSurface)
            level == 2 -> all.filter { it !== analysisSurface }
            else -> all
        }
        if (level >= 2) {
            autoOnly = true
            onStatus(unsupportedStatus())
        }
        if (level == 1 && physicalId == null) {
            // Nothing was pinned at level 0, so level 1 would be the same request.
            createSession(d, all, 2); return
        }
        val b = d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW)
        for (s in surfaces) b.addTarget(s)
        applyZoom(b)
        applyBias(b)
        builder = b
        val callback = object : CameraCaptureSession.StateCallback() {
            override fun onConfigured(s: CameraCaptureSession) {
                session = s
                s.setRepeatingRequest(b.build(), resultCallback, handler)
                if (level < 3) onReady()
            }

            override fun onConfigureFailed(s: CameraCaptureSession) {
                Log.w(TAG, "session configuration failed at level $level (lens pin $pin): trying a simpler one")
                if (level < 3 && !stopped) {
                    try { createSession(d, all, level + 1) } catch (e: Exception) {
                        Log.w(TAG, "fallback session failed: ${e.message}")
                    }
                }
            }
        }
        val executor = java.util.concurrent.Executor { r -> (handler ?: Handler(android.os.Looper.getMainLooper())).post(r) }
        if (pin != null) {
            // Streams bound to one physical lens: the phone cannot switch lenses under us.
            val configs = surfaces.map { sf ->
                OutputConfiguration(sf).also { it.setPhysicalCameraId(pin) }
            }
            d.createCaptureSession(
                SessionConfiguration(SessionConfiguration.SESSION_REGULAR, configs, executor, callback)
            )
        } else {
            d.createCaptureSession(surfaces, callback, null)
        }
    }

    // Shortest exposure this sensor really delivers. The advertised range can be shorter
    // (and differs per lens): learned when the sensor keeps returning a longer exposure.
    private var minExposureOverrideUs = 0.0
    private var lastActualNs = 0L
    private var mismatchRun = 0

    private fun currentLimits(): NsJni.CameraLimits {
        val base = Camera2Sync.limitsFromCharacteristics(
            manager.getCameraCharacteristics(activePhysicalId ?: cameraId)
        )
        return if (minExposureOverrideUs > base.minExposureUs) {
            base.copy(minExposureUs = minExposureOverrideUs)
        } else base
    }

    private fun unsupportedStatus() = NsCommand(0.0, 0.0, 0.0, NsJni.MODE_UNSUPPORTED,
        NsJni.STATE_SEARCH, 0.0, 0.0, 0.0)

    private fun onFrame(reader: ImageReader) {
        val image = try {
            reader.acquireLatestImage()
        } catch (e: IllegalStateException) {
            return  // reader closed during shutdown
        } ?: return
        try {
            if (autoOnly) return  // nothing to analyse: no manual exposure on this camera
            if (SystemClock.elapsedRealtime() < restUntilMs) return  // auto-exposure rest
            if (pwmMode && pwm == null) {
                val limits = currentLimits()
                lineRateHz = LineProfileExtractor.lineRate(image.height, sustainUs * 1000.0)
                pwm = NsJni.createPwm(limits, lineRateHz)
            }
            // Let the camera's auto-exposure settle first so the brightness budget is known
            // (about one second at 60 fps; the analysis has room for that).
            if (requestedExposureNs == 0L && budgetUsIso == 0.0) {
                autoWait += 1
                if (autoWait < 60) return
                budgetUsIso = latestAutoBudget  // AE never reported CONVERGED: use what we have
            }
            // Skip frames still carrying the previous exposure; they would pollute the trial.
            if (!exposureMatches(actualExposureNs(image.timestamp), requestedExposureNs)) {
                staleFrames += 1
                val actual = actualExposureNs(image.timestamp)
                mismatchRun = if (actual > requestedExposureNs &&
                    abs(actual - lastActualNs) <= actual / 100) mismatchRun + 1 else 0
                lastActualNs = actual
                if (mismatchRun >= 20) {
                    // Requested below the sensor's real minimum: relearn the range.
                    minExposureOverrideUs = actual / 1000.0 * 1.02
                    Log.i(TAG, "sensor minimum exposure is about ${actual / 1000} us, not the advertised one")
                    mismatchRun = 0
                    staleFrames = 0
                    pwm?.close(); pwm = null
                    last = null
                    requestedExposureNs = 0L
                    return
                }
                recordFrame(image.timestamp, actualExposureNs(image.timestamp), last, -1.0, true)
                if (staleFrames == 60) {
                    Log.w(TAG, "sensor ignores the requested exposure (asked $requestedExposureNs ns, " +
                        "got ${actualExposureNs(image.timestamp)} ns): manual exposure unsupported in this mode, " +
                        "back to auto-exposure for good")
                    autoOnly = true
                    fallBackToAuto(SystemClock.elapsedRealtime(), last ?: unsupportedStatus())
                    onStatus(unsupportedStatus())
                    return
                }
                return
            }
            staleFrames = 0
            var lumaMean = 0.0
            val cmd = if (pwmMode) {
                val prof = LineProfileExtractor.rowProfile(image, 1)
                lumaMean = if (prof.isEmpty()) 0.0 else prof.average()
                pwm!!.onProfile(prof)
            } else {
                lumaMean = LumaExtractor.meanLuma(image).toDouble()
                strobe!!.onFrame(image.timestamp / 1000.0, lumaMean)
            }
            // ISO that keeps the metered brightness at this exposure; the controller's own
            // ISO has no reference for the scene and is ignored once the budget is known.
            val applied = if (budgetUsIso > 0.0) {
                cmd.copy(iso = (budgetUsIso * Math.pow(2.0, evBias.toDouble()) / cmd.exposureUs).coerceIn(isoMin, isoMax))
            } else {
                cmd
            }
            onStatus(applied)
            val now = SystemClock.elapsedRealtime()
            if (applied.state == NsJni.STATE_LOCKED || probeSinceMs == 0L) {
                probeSinceMs = now
            } else if (now - probeSinceMs > PROBE_MS) {
                fallBackToAuto(now, applied)
                return
            }
            recordFrame(image.timestamp, actualExposureNs(image.timestamp), applied, lumaMean, false)
            if (needsUpdate(last, applied)) {
                last = applied
                val b = builder!!
                Camera2Sync.apply(b, applied)
                applyZoom(b)
                requestedExposureNs = (cmd.exposureUs * 1000.0).toLong()
                session?.setRepeatingRequest(b.build(), resultCallback, handler)
            }
        } catch (e: IllegalStateException) {
            Log.w(TAG, "frame dropped: ${e.message}")
        } finally {
            image.close()
        }
    }

    /** Back to the camera's own auto-exposure; probing resumes after REST_MS. */
    private fun fallBackToAuto(now: Long, applied: NsCommand) {
        Log.i(TAG, "no flicker locked in $PROBE_MS ms: auto-exposure for $REST_MS ms")
        restUntilMs = now + REST_MS
        probeSinceMs = 0L
        pwm?.close(); pwm = null
        requestedExposureNs = 0L
        budgetUsIso = 0.0
        latestAutoBudget = 0.0
        autoWait = 0
        staleFrames = 0
        last = null
        val d = device
        val sess = session
        if (d != null && sess != null) {
            try {
                // A fresh template request: AE on, no manual exposure, ISO or frame duration.
                val b = d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW)
                for (t in surfaceList) b.addTarget(t)
                applyZoom(b)
                applyBias(b)
                builder = b
                sess.setRepeatingRequest(b.build(), resultCallback, handler)
            } catch (e: Exception) {
                Log.w(TAG, "could not return to auto-exposure: ${e.message}")
            }
        }
        onStatus(applied.copy(mode = NsJni.MODE_AUTO, state = NsJni.STATE_SEARCH,
            periodUs = 0.0, confidence = 0.0))
    }

    private fun needsUpdate(prev: NsCommand?, cmd: NsCommand): Boolean {
        if (prev == null) return true
        val de = abs(cmd.exposureUs - prev.exposureUs) / prev.exposureUs
        val di = abs(cmd.iso - prev.iso) / prev.iso
        return de > 0.01 || di > 0.05 || cmd.fps != prev.fps
    }

    fun stop() {
        stopped = true
        // Stop the analysis first: closing the reader while the analysis thread is still
        // reading a plane throws "buffer is inaccessible".
        reader?.setOnImageAvailableListener(null, null)
        analysisThread?.quitSafely()
        try { analysisThread?.join(1000) } catch (_: InterruptedException) {}
        analysisThread = null; handler = null
        session?.close(); session = null
        device?.close(); device = null
        reader?.close(); reader = null
        strobe?.close(); strobe = null
        pwm?.close(); pwm = null
        requestedExposureNs = 0L
        latestResultExposureNs = 0L
        staleFrames = 0
        synchronized(resultExposure) { resultExposure.clear() }
        last = null
        budgetUsIso = 0.0
        latestAutoBudget = 0.0
        autoWait = 0
        probeSinceMs = 0L
        restUntilMs = 0L
        mismatchRun = 0
        lastActualNs = 0L
    }

    private companion object {
        const val TAG = "nostrobe"
        const val PROBE_MS = 6_000L
        const val REST_MS = 12_000L
        const val FOCUS_HOLD_MS = 3_000L
    }
}
