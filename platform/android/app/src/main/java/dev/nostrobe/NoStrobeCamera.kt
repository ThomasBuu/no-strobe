package dev.nostrobe

import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.media.MediaRecorder
import android.media.ImageReader
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

    fun start() {
        val chars = manager.getCameraCharacteristics(cameraId)
        val limits = Camera2Sync.limitsFromCharacteristics(chars)
        sustainUs = limits.sustainUs

        if (pwmMode) {
            // The controller is built on the first frame: the line rate needs the stream
            // height, which is only known once an image arrives.
            pwm = null
        } else {
            strobe = NsJni.create(limits)
        }

        val map = chars.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
        val size = map?.getOutputSizes(ImageFormat.YUV_420_888)?.maxBy { it.width * it.height }
            ?: Size(1920, 1080)

        val r = ImageReader.newInstance(size.width, size.height, ImageFormat.YUV_420_888, 3)
        r.setOnImageAvailableListener({ reader -> onFrame(reader) }, null)
        reader = r

        val surfaces = listOf(previewSurface, r.surface) +
            (recorder?.surface?.let { listOf(it) } ?: emptyList())

        manager.openCamera(cameraId, object : CameraDevice.StateCallback() {
            override fun onOpened(d: CameraDevice) {
                device = d
                createSession(d, surfaces)
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
                s.setRepeatingRequest(b.build(), null, null)
            }

            override fun onConfigureFailed(s: CameraCaptureSession) {}
        }, null)
    }

    private fun onFrame(reader: ImageReader) {
        val image = reader.acquireLatestImage() ?: return
        try {
            if (pwmMode && pwm == null) {
                val limits = Camera2Sync.limitsFromCharacteristics(
                    manager.getCameraCharacteristics(cameraId)
                )
                lineRateHz = LineProfileExtractor.lineRate(image.height, sustainUs * 1000.0)
                pwm = NsJni.createPwm(limits, lineRateHz)
            }
            val cmd = if (pwmMode) {
                pwm!!.onProfile(LineProfileExtractor.rowProfile(image, 1))
            } else {
                strobe!!.onFrame(image.timestamp / 1000.0, LumaExtractor.meanLuma(image))
            }
            onStatus(cmd)
            if (needsUpdate(last, cmd)) {
                last = cmd
                val b = builder!!
                Camera2Sync.apply(b, cmd)
                session?.setRepeatingRequest(b.build(), null, null)
            }
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
        session?.close(); session = null
        device?.close(); device = null
        reader?.close(); reader = null
        strobe?.close(); strobe = null
        pwm?.close(); pwm = null
    }
}
