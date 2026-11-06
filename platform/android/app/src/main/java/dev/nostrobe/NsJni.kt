package dev.nostrobe

/**
 * JNI bridge to the Rust strobe-sync controller (libns_ffi.so).
 *
 * Output slots from nativeOnFrame:
 * 0 exposure_us, 1 fps, 2 iso, 3 mode, 4 state, 5 period_us, 6 confidence, 7 drift_us_per_frame.
 *
 * The controller classes are nested inside this object and the external functions are
 * private. Kotlin name-mangles `internal` members with the module name (`nativeCreatePwm$app`),
 * and the JNI symbol `Java_dev_nostrobe_NsJni_nativeCreatePwm` then never resolves.
 */
object NsJni {
    init {
        System.loadLibrary("ns_ffi")
    }

    private external fun nativeCreate(
        minExposureUs: Double,
        maxExposureUs: Double,
        sustainUs: Double,
        fps: DoubleArray,
        minIso: Double,
        maxIso: Double,
    ): Long

    private external fun nativeOnFrame(handle: Long, tUs: Double, luminance: Double, out: DoubleArray)

    private external fun nativeDestroy(handle: Long)

    private external fun nativeCreatePwm(
        minExposureUs: Double,
        maxExposureUs: Double,
        sustainUs: Double,
        fps: DoubleArray,
        minIso: Double,
        maxIso: Double,
        lineRateHz: Double,
    ): Long

    private external fun nativeOnProfile(handle: Long, profile: DoubleArray, out: DoubleArray)

    private external fun nativeDestroyPwm(handle: Long)

    const val MODE_INTEGRATE = 0
    const val MODE_GAP = 1
    const val MODE_FLASH = 2
    const val MODE_PWM = 3
    /** Not a controller mode: the app went back to auto-exposure (no flicker found). */
    const val MODE_AUTO = -1
    /** The camera cannot do manual exposure: flicker sync is unavailable on this device. */
    const val MODE_UNSUPPORTED = -2
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
            nativeOnFrame(handle, tUs, luminance, out)
            return command(out)
        }

        fun close() {
            nativeDestroy(handle)
        }
    }

    /** PWM (LED flicker) controller: feed the per-row luma profile of each frame. */
    class PwmSyncController(private val handle: Long) {
        private val out = DoubleArray(8)

        /** Feed one frame's vertical luminance profile (per-row mean luma, 0..1). */
        fun onProfile(profile: DoubleArray): NsCommand {
            nativeOnProfile(handle, profile, out)
            return command(out)
        }

        fun close() {
            nativeDestroyPwm(handle)
        }
    }

    private fun command(out: DoubleArray) = NsCommand(
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

// Nested inside NsJni so the external functions stay private (no JNI name mangling).
// The aliases keep the call sites unchanged.
typealias CameraLimits = NsJni.CameraLimits
typealias NsCommand = NsJni.NsCommand
typealias SyncController = NsJni.SyncController
typealias PwmSyncController = NsJni.PwmSyncController
