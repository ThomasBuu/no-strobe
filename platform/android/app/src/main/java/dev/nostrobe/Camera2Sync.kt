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
        val exposureRange: Range<Long> = chars.get(CameraCharacteristics.SENSOR_INFO_EXPOSURE_TIME_RANGE)
            ?: return CameraLimits(100.0, 30000.0, 20000.0, doubleArrayOf(30.0), 100.0, 6400.0)
        val sensitivity: Range<Int>? = chars.get(CameraCharacteristics.SENSOR_INFO_SENSITIVITY_RANGE)
        // The list of supported fps, not the current AE target.
        val fpsRanges: Array<Range<Int>>? =
            chars.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
        // API 31+ key. The symbol is not in android.jar at compileSdk 34, so the key is
        // built from its name. Devices that do not define the tag throw
        // IllegalArgumentException from CameraCharacteristics.get, so the read is guarded;
        // they get sustain = 0 and the PWM path hill-climbs.
        val sustainKey = CameraCharacteristics.Key(
            "android.sensor.info.rolling_shutter_sustain_duration",
            java.lang.Long::class.java,
        )
        val sustain = runCatching { chars.get(sustainKey) }.getOrNull()

        val fps = fpsRanges?.map { it.upper.toDouble() }?.toDoubleArray()
            ?: doubleArrayOf(30.0)

        return CameraLimits(
            minExposureUs = exposureRange.lower / NS_PER_US.toDouble(),
            maxExposureUs = exposureRange.upper / NS_PER_US.toDouble(),
            sustainUs = (sustain?.toLong() ?: 0L) / NS_PER_US.toDouble(),
            supportedFps = fps,
            minIso = (sensitivity?.lower ?: 100).toDouble(),
            maxIso = (sensitivity?.upper ?: 6400).toDouble(),
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
