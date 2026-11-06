package dev.nostrobe

/**
 * JNI bridge to the Rust strobe-sync controller (libns_ffi.so).
 *
 * Output slots from nativeOnFrame:
 * 0 exposure_us, 1 fps, 2 iso, 3 mode, 4 state, 5 period_us, 6 confidence, 7 drift_us_per_frame.
 */
object NsJni {
    init {
        System.loadLibrary("ns_ffi")
    }

    // internal, not private: SyncController / PwmSyncController are top-level in this file.
    internal external fun nativeCreate(
        minExposureUs: Double,
        maxExposureUs: Double,
        sustainUs: Double,
        fps: DoubleArray,
        minIso: Double,
        maxIso: Double,
    ): Long

    internal external fun nativeOnFrame(handle: Long, tUs: Double, luminance: Double, out: DoubleArray)

    internal external fun nativeDestroy(handle: Long)

    internal external fun nativeCreatePwm(
        minExposureUs: Double,
        maxExposureUs: Double,
        sustainUs: Double,
        fps: DoubleArray,
        minIso: Double,
        maxIso: Double,
        lineRateHz: Double,
    ): Long

    internal external fun nativeOnProfile(handle: Long, profile: DoubleArray, out: DoubleArray)

    internal external fun nativeDestroyPwm(handle: Long)

    const val MODE_INTEGRATE = 0
    const val MODE_GAP = 1
    const val MODE_FLASH = 2
    const val MODE_PWM = 3
    const val STATE_SEARCH = 0
    const val STATE_LOCKED = 1

    fun create(limits: CameraLimits): SyncController {
        val handle = nativeCreate(
            limits.minExposureUs,
            limits.maxExposureUs,
            limits.sustainUs,
            limits.supportedFps,
            limits.minIso,
            limits.maxIso,
        )
        return SyncController(handle)
    }

    /**
     * PWM (LED) mode. `lineRateHz` is the sensor row readout rate in rows per second,
     * computed as sensor height / SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION.
     * Pass 0.0 when the device does not publish it: the controller calibrates by sweeping.
     */
    fun createPwm(limits: CameraLimits, lineRateHz: Double): PwmSyncController {
        val handle = nativeCreatePwm(
            limits.minExposureUs,
            limits.maxExposureUs,
            limits.sustainUs,
            limits.supportedFps,
            limits.minIso,
            limits.maxIso,
            lineRateHz,
        )
        return PwmSyncController(handle)
    }
}

data class CameraLimits(
    val minExposureUs: Double,
    val maxExposureUs: Double,
    val sustainUs: Double,
    val supportedFps: DoubleArray,
    val minIso: Double,
    val maxIso: Double,
)

data class NsCommand(
    val exposureUs: Double,
    val fps: Double,
    val iso: Double,
    val mode: Int,
    val state: Int,
    val periodUs: Double,
    val confidence: Double,
    val driftUsPerFrame: Double,
)

class SyncController(private val handle: Long) {
    private val out = DoubleArray(8)

    /** Feed one frame (readout start in microseconds, mean luma 0..1), get the shutter command. */
    fun onFrame(tUs: Double, luminance: Double): NsCommand {
        NsJni.nativeOnFrame(handle, tUs, luminance, out)
        return NsCommand(
            exposureUs = out[0],
            fps = out[1],
            iso = out[2],
            mode = out[3].toInt(),
            state = out[4].toInt(),
            periodUs = out[5],
            confidence = out[6],
            driftUsPerFrame = out[7],
        )
    }

    fun close() {
        NsJni.nativeDestroy(handle)
    }
}

/** PWM (LED flicker) controller: feed the per-row luma profile of each frame. */
class PwmSyncController(private val handle: Long) {
    private val out = DoubleArray(8)

    /** Feed one frame's vertical luminance profile (per-row mean luma, 0..1). */
    fun onProfile(profile: DoubleArray): NsCommand {
        NsJni.nativeOnProfile(handle, profile, out)
        return NsCommand(
            exposureUs = out[0],
            fps = out[1],
            iso = out[2],
            mode = out[3].toInt(),
            state = out[4].toInt(),
            periodUs = out[5],
            confidence = out[6],
            driftUsPerFrame = out[7],
        )
    }

    fun close() {
        NsJni.nativeDestroyPwm(handle)
    }
}
