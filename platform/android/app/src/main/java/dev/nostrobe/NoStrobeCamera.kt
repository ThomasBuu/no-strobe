package dev.nostrobe

import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.CaptureResult
import android.hardware.camera2.TotalCaptureResult
import android.media.MediaRecorder
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
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
    @Volatile private var stopped = false

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
        val chars = manager.getCameraCharacteristics(cameraId)
        val limits = Camera2Sync.limitsFromCharacteristics(chars)
        sustainUs = limits.sustainUs
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

        val surfaces = listOf(previewSurface, r.surface) +
            (recorder?.surface?.let { listOf(it) } ?: emptyList())

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

    private fun createSession(d: CameraDevice, surfaces: List<Surface>) {
        val b = d.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW)
        for (s in surfaces) b.addTarget(s)
        builder = b
        d.createCaptureSession(surfaces, object : CameraCaptureSession.StateCallback() {
            override fun onConfigured(s: CameraCaptureSession) {
                session = s
                s.setRepeatingRequest(b.build(), resultCallback, handler)
            }

            override fun onConfigureFailed(s: CameraCaptureSession) {}
        }, null)
    }

    private fun onFrame(reader: ImageReader) {
        val image = try {
            reader.acquireLatestImage()
        } catch (e: IllegalStateException) {
            return  // reader closed during shutdown
        } ?: return
        try {
            if (pwmMode && pwm == null) {
                val limits = Camera2Sync.limitsFromCharacteristics(
                    manager.getCameraCharacteristics(cameraId)
                )
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
                recordFrame(image.timestamp, actualExposureNs(image.timestamp), last, -1.0, true)
                if (staleFrames == 60) {
                    Log.w(TAG, "sensor ignores the requested exposure (asked $requestedExposureNs ns, " +
                        "got ${actualExposureNs(image.timestamp)} ns): manual exposure unsupported in this mode?")
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
                cmd.copy(iso = (budgetUsIso / cmd.exposureUs).coerceIn(isoMin, isoMax))
            } else {
                cmd
            }
            onStatus(applied)
            recordFrame(image.timestamp, actualExposureNs(image.timestamp), applied, lumaMean, false)
            if (needsUpdate(last, applied)) {
                last = applied
                val b = builder!!
                Camera2Sync.apply(b, applied)
                requestedExposureNs = (cmd.exposureUs * 1000.0).toLong()
                session?.setRepeatingRequest(b.build(), resultCallback, handler)
            }
        } catch (e: IllegalStateException) {
            Log.w(TAG, "frame dropped: ${e.message}")
        } finally {
            image.close()
        }
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
    }

    private companion object {
        const val TAG = "nostrobe"
    }
}
