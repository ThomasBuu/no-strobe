package dev.nostrobe

import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraMetadata
import android.hardware.camera2.CaptureRequest
import android.util.Range

/** Camera2 side: read the sensor limits, then apply the shutter command per frame. */
object Camera2Sync {
    private const val NS_PER_US = 1000L

    fun supportsManualSensor(chars: CameraCharacteristics): Boolean {
        val caps = chars.get(CameraCharacteristics.REQUEST_AVAILABLE_CAPABILITIES) ?: return false
        return caps.contains(CameraMetadata.REQUEST_AVAILABLE_CAPABILITIES_MANUAL_SENSOR)
    }

    /**
     * Sensor limits in microseconds. Rolling-shutter sustain is only reported on API 31+;
     * devices without it get 0, which makes the solver treat the readout as instantaneous.
     */
    fun limitsFromCharacteristics(chars: CameraCharacteristics): CameraLimits {
        val exposureRange = chars.get(CameraCharacteristics.SENSOR_INFO_EXPOSURE_TIME)
            ?: return CameraLimits(100.0, 30000.0, 20000.0, doubleArrayOf(30.0), 100.0, 6400.0)
        val sensitivity = chars.get(CameraCharacteristics.SENSOR_INFO_SENSITIVITY_RANGE)
        val fpsRanges = chars.get(CameraCharacteristics.CONTROL_AE_TARGET_FPS_RANGE)
        val sustain = chars.get(CameraCharacteristics.SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION)

        val fps = fpsRanges?.map { it.toDouble() }?.toDoubleArray()
            ?: doubleArrayOf(30.0)

        return CameraLimits(
            minExposureUs = exposureRange.getLower() / NS_PER_US.toDouble(),
            maxExposureUs = exposureRange.getUpper() / NS_PER_US.toDouble(),
            sustainUs = (sustain?.getUpper() ?: 0L) / NS_PER_US.toDouble(),
            supportedFps = fps,
            minIso = (sensitivity?.getLower() ?: 100).toDouble(),
            maxIso = (sensitivity?.getUpper() ?: 6400).toDouble(),
        )
    }

    /**
     * Manual exposure for one frame. AE must be off, otherwise the 3A routine overwrites
     * these values. Frame duration is set so the frame period stays an integer multiple of
     * the strobe period; fps is kept in the AE range key for devices that ignore
     * SENSOR_FRAME_DURATION.
     */
    fun apply(builder: CaptureRequest.Builder, cmd: NsCommand) {
        val fps = cmd.fps.toInt()
        builder.set(CaptureRequest.CONTROL_AE_MODE, CameraMetadata.CONTROL_AE_MODE_OFF)
        builder.set(CaptureRequest.CONTROL_CAPTURE_INTENT, CameraMetadata.CONTROL_CAPTURE_INTENT_MANUAL)
        builder.set(
            CaptureRequest.SENSOR_EXPOSURE_TIME,
            (cmd.exposureUs * NS_PER_US).toLong(),
        )
        builder.set(CaptureRequest.SENSOR_SENSITIVITY, cmd.iso.toInt())
        builder.set(
            CaptureRequest.SENSOR_FRAME_DURATION,
            (1_000_000.0 / cmd.fps * NS_PER_US).toLong(),
        )
        builder.set(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, Range(fps, fps))
    }
}
