package dev.nostrobe.strobe

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.CaptureResult
import android.hardware.camera2.TotalCaptureResult
import android.hardware.camera2.params.OutputConfiguration
import android.hardware.camera2.params.SessionConfiguration
import android.media.Image
import android.media.ImageReader
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.util.Log
import android.view.Surface
import java.util.concurrent.Executor
import kotlin.math.abs

/**
 * Caméra arrière en réglage manuel (pose courte, ISO fixe) dont la durée de chaque image est pilotée par [StrobeLock]
 * pour tomber lampe éteinte. Tout se passe sur un seul thread (callbacks caméra, images, enregistrement).
 */
class StrobeCamera(private val context: Context, private val onStatus: (Status) -> Unit) {
    data class Status(
        val frequency: Double? = null, val locked: Boolean = false, val cleanRatio: Double = 0.0, val fps: Double = 0.0,
        val recording: Boolean = false, val skewMs: Double? = null, val isoRange: IntRange = 100..3200,
        val iso: Int = 800, val exposureMs: Double = 2.0, val metering: Boolean = true,
        val timingIgnored: Boolean = false, // le téléphone n'applique pas la durée d'image demandée : calage impossible
    )

    val width = 1280; val height = 720
    private val thread = HandlerThread("camera").apply { start() }
    private val handler = Handler(thread.looper)
    private val main = Handler(Looper.getMainLooper())
    private val executor = Executor { handler.post(it) }
    private val manager = context.getSystemService(CameraManager::class.java)

    private var device: CameraDevice? = null
    private var opening = false
    private var session: CameraCaptureSession? = null
    private var reader: ImageReader? = null
    private var preview: Surface? = null
    private var chars: CameraCharacteristics? = null
    private var fpsRange: android.util.Range<Int>? = null

    private val lock = StrobeLock(bands = 16)
    private var exposureNs = 2_000_000L
    private var iso = 800
    private var frameDurationNs = 33_333_333L
    private var meterFrames = 0 // au démarrage : exposition auto ~1 s pour trouver l'ISO qui va avec la pose courte
    private val results = HashMap<Long, TotalCaptureResult>()
    private val images = HashMap<Long, Image>()
    private var recording: Recording? = null
    private var status = Status()
    private var seen = 0; private var kept = 0; private var windowStart = 0L
    private var lastLum = DoubleArray(16)
    private var lastTs = 0L; private val intervals = ArrayList<Double>()
    private var requestedNs = 0L; private var mismatches = 0

    fun open(surface: Surface) = handler.post {
        preview = surface
        if (opening || device != null) return@post // surface signalée deux fois : une seule ouverture
        opening = true
        runCatching { openCamera() }.onFailure { opening = false; Log.e(TAG, "ouverture caméra impossible", it) }
    }

    fun close() = handler.post {
        recording?.finish(); recording = null
        session?.close(); session = null
        device?.close(); device = null
        reader?.close(); reader = null
        images.values.forEach { it.close() }; images.clear(); results.clear()
        status = status.copy(recording = false); publish()
    }

    fun setExposure(exposureMs: Double, iso: Int) = handler.post {
        exposureNs = (exposureMs * 1e6).toLong(); this.iso = iso
        status = status.copy(iso = iso, exposureMs = exposureMs)
        repeat()
    }

    fun toggleRecording() = handler.post {
        val r = recording
        if (r != null) { r.finish(); recording = null }
        else {
            val orientation = chars?.get(CameraCharacteristics.SENSOR_ORIENTATION) ?: 90
            val realtime = chars?.get(CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE) == CameraCharacteristics.SENSOR_INFO_TIMESTAMP_SOURCE_REALTIME
            recording = runCatching { Recording(context, width, height, orientation, realtime) }
                .onFailure { Log.e(TAG, "enregistrement impossible", it) }.getOrNull()
        }
        status = status.copy(recording = recording != null); publish()
    }

    @SuppressLint("MissingPermission") // demandée par l'activité avant d'ouvrir
    private fun openCamera() {
        val id = manager.cameraIdList.firstOrNull {
            manager.getCameraCharacteristics(it).get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK
        } ?: return
        val c = manager.getCameraCharacteristics(id); chars = c
        c.get(CameraCharacteristics.SENSOR_INFO_SENSITIVITY_RANGE)?.let { status = status.copy(isoRange = it.lower..it.upper) }
        val map = c.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val minNs = map?.getOutputMinFrameDuration(ImageFormat.YUV_420_888, android.util.Size(width, height)) ?: 0L
        lock.minFrameDuration = maxOf(1.0 / 30, minNs / 1e9)
        fpsRange = c.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)?.minByOrNull { it.lower * 100 - it.upper }
        manager.openCamera(id, object : CameraDevice.StateCallback() {
            override fun onOpened(d: CameraDevice) { opening = false; device = d; startSession(d) }
            override fun onDisconnected(d: CameraDevice) { opening = false; d.close(); device = null }
            override fun onError(d: CameraDevice, error: Int) { opening = false; Log.e(TAG, "caméra : erreur $error"); d.close(); device = null }
        }, handler)
    }

    private fun startSession(d: CameraDevice) {
        val r = ImageReader.newInstance(width, height, ImageFormat.YUV_420_888, 6)
        r.setOnImageAvailableListener({ rd -> rd.acquireNextImage()?.let { onImage(it) } }, handler)
        reader = r
        val outputs = listOfNotNull(preview, r.surface).map { OutputConfiguration(it) }
        d.createCaptureSession(SessionConfiguration(SessionConfiguration.SESSION_REGULAR, outputs, executor,
            object : CameraCaptureSession.StateCallback() {
                override fun onConfigured(s: CameraCaptureSession) { session = s; meterFrames = 30; repeat() }
                override fun onConfigureFailed(s: CameraCaptureSession) { Log.e(TAG, "session caméra refusée") }
            }))
    }

    /** (Re)lance la requête répétée avec la pose, l'ISO et la durée d'image courantes. */
    private fun repeat() {
        val d = device ?: return; val s = session ?: return; val r = reader ?: return
        val b = d.createCaptureRequest(CameraDevice.TEMPLATE_RECORD)
        preview?.let { b.addTarget(it) }
        b.addTarget(r.surface)
        b.set(CaptureRequest.CONTROL_MODE, CaptureRequest.CONTROL_MODE_AUTO)
        b.set(CaptureRequest.CONTROL_AF_MODE, CaptureRequest.CONTROL_AF_MODE_CONTINUOUS_VIDEO)
        if (meterFrames > 0) b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_ON)
        else {
            b.set(CaptureRequest.CONTROL_AE_MODE, CaptureRequest.CONTROL_AE_MODE_OFF) // sinon l'exposition auto pompe à chaque flash
            b.set(CaptureRequest.CONTROL_CAPTURE_INTENT, CaptureRequest.CONTROL_CAPTURE_INTENT_MANUAL) // sinon certains capteurs gardent leur cadence
            b.set(CaptureRequest.SENSOR_EXPOSURE_TIME, exposureNs)
            b.set(CaptureRequest.SENSOR_SENSITIVITY, iso)
            b.set(CaptureRequest.SENSOR_FRAME_DURATION, frameDurationNs)
            // certains capteurs (MediaTek) appliquent la plage de cadence même en manuel : on la laisse descendre
            fpsRange?.let { b.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it) }
        }
        runCatching { s.setRepeatingRequest(b.build(), captureCallback, handler) }.onFailure { Log.w(TAG, "requête refusée", it) }
    }

    private val captureCallback = object : CameraCaptureSession.CaptureCallback() {
        override fun onCaptureCompleted(s: CameraCaptureSession, request: CaptureRequest, result: TotalCaptureResult) {
            val ts = result.get(CaptureResult.SENSOR_TIMESTAMP) ?: return
            results[ts] = result
            pair(ts)
        }
    }

    private fun onImage(image: Image) {
        images[image.timestamp] = image
        if (images.size > 4) images.minByOrNull { it.key }?.let { images.remove(it.key)?.close() } // résultat jamais arrivé
        pair(image.timestamp)
    }

    private fun pair(ts: Long) {
        val result = results[ts] ?: return
        val image = images.remove(ts) ?: return
        results.remove(ts)
        results.keys.removeAll { it < ts - 1_000_000_000L }
        process(image, result, ts)
    }

    private fun process(image: Image, result: TotalCaptureResult, ts: Long) {
        // ponytail: 25 ms par défaut si le capteur ne donne pas son rolling shutter ; réglable si ça tombe faux
        val skewNs = result.get(CaptureResult.SENSOR_ROLLING_SHUTTER_SKEW) ?: 25_000_000L
        val exposure = (result.get(CaptureResult.SENSOR_EXPOSURE_TIME) ?: exposureNs) / 1e9
        if (meterFrames > 0) { // mesure : même luminosité qu'en auto, mais avec la pose courte (on monte l'ISO d'autant)
            image.close()
            if (--meterFrames == 0) {
                val aeIso = result.get(CaptureResult.SENSOR_SENSITIVITY) ?: iso
                val range = status.isoRange
                iso = (aeIso * exposure * 1e9 / exposureNs).toInt().coerceIn(range.first, range.last)
                status = status.copy(iso = iso, metering = false); publish()
                Log.i(TAG, "mesure : auto ${exposure * 1e3} ms ISO $aeIso → ${exposureNs / 1e6} ms ISO $iso")
                repeat()
            }
            return
        }
        val interval = ts - lastTs; lastTs = ts
        intervals.add(interval / 1e6); if (intervals.size > 8) intervals.removeAt(0)
        // durée demandée nettement différente de l'écart réel, longtemps : le capteur ignore la durée d'image
        if (abs(requestedNs - interval) > 3_000_000L && abs(requestedNs - interval) < 500_000_000L) mismatches++ else mismatches = 0
        if (mismatches == 30 && !status.timingIgnored) { status = status.copy(timingIgnored = true); publish(); Log.w(TAG, "durée d'image ignorée par le capteur") }
        requestedNs = frameDurationNs
        lastLum = bandLuminance(image, 16)
        val r = lock.onFrame(ts / 1e9, skewNs / 1e9, exposure, lastLum)
        recording?.let { rec ->
            rec.raw.encode(image, ts)
            if (r.clean) rec.processed.encode(image, ts)
        }
        image.close()

        val next = (r.nextFrameDuration * 1e9).toLong()
        if (abs(next - frameDurationNs) > 20_000) { frameDurationNs = next; repeat() }

        seen++; if (r.clean) kept++
        if (ts - windowStart > 500_000_000L) {
            status = status.copy(frequency = lock.frequency, locked = lock.locked, cleanRatio = kept.toDouble() / maxOf(seen, 1),
                fps = seen / ((ts - windowStart) / 1e9), skewMs = skewNs / 1e6)
            seen = 0; kept = 0; windowStart = ts
            publish()
            Log.i(TAG, "écarts ${intervals.joinToString(" ") { "%.1f".format(it) }} ; appliqué ${(result.get(CaptureResult.SENSOR_FRAME_DURATION) ?: 0) / 1e6} ms, plage $fpsRange ; f=${lock.frequency} calé=${lock.locked} lum=${"%.3f".format(lastLum.average())} skew=${skewNs / 1e6} ms pose=${exposure * 1e3} ms durée=${frameDurationNs / 1e6} ms ${status.fps} i/s")
        }
    }

    private fun publish() { val s = status; main.post { onStatus(s) } }

    companion object {
        private const val TAG = "AntiStrobe"

        /** Luminance moyenne (0…1) de bandes horizontales du capteur (= ordre de lecture), une ligne sur 4, un pixel sur 8. */
        fun bandLuminance(image: Image, bands: Int): DoubleArray {
            val plane = image.planes[0]; val buf = plane.buffer
            val w = image.width; val h = image.height; val stride = plane.rowStride
            return DoubleArray(bands) { b ->
                var sum = 0L; var n = 0
                var y = b * h / bands
                while (y < (b + 1) * h / bands) {
                    var x = 0
                    while (x < w) { sum += buf.get(y * stride + x).toInt() and 0xff; n++; x += 8 }
                    y += 4
                }
                sum.toDouble() / maxOf(n, 1) / 255
            }
        }
    }
}
