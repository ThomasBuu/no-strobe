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
    private external fun nativeOnYPlane(
        handle: Long,
        buffer: java.nio.Buffer,
        width: Int,
        height: Int,
        rowStride: Int,
        pixelStride: Int,
        rowStep: Int,
        colStep: Int,
        out: DoubleArray,
    )
    private external fun nativeExtractProfile(
        buffer: java.nio.Buffer,
        width: Int,
        height: Int,
        rowStride: Int,
        pixelStride: Int,
        rowStep: Int,
        colStep: Int,
    ): DoubleArray

    private external fun nativeDestroyPwm(handle: Long)
    private external fun nativeSeedPwm(handle: Long, exposureUs: Double, fHz: Double)
    private external fun nativeFrequencyPwm(handle: Long): Double
    private external fun nativeSetShutterFraction(handle: Long, fraction: Double)
    private external fun nativeSetMinExposure(handle: Long, us: Double)
    private external fun nativeSetMaxExposure(handle: Long, us: Double)
    private external fun nativeSyntheticProfile(
        exposureUs: Double, fHz: Double, phaseCycles: Double, duty: Double, lineRateHz: Double, rows: Int,
    ): DoubleArray

    /** Extract the vertical row profile from a direct Y-buffer using SIMD. */
    fun extractProfile(
        buffer: java.nio.Buffer,
        width: Int,
        height: Int,
        rowStride: Int,
        pixelStride: Int,
        rowStep: Int = 1,
        colStep: Int = 1,
    ): DoubleArray = nativeExtractProfile(buffer, width, height, rowStride, pixelStride, rowStep, colStep)

    /** Test only: the row profile of a PWM light at [fHz], for the simulated-flicker mode. */
    fun syntheticProfile(
        exposureUs: Double, fHz: Double, phaseCycles: Double, duty: Double, lineRateHz: Double, rows: Int,
    ): DoubleArray = nativeSyntheticProfile(exposureUs, fHz, phaseCycles, duty, lineRateHz, rows)

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
        /** The flicker frequency found (0 while searching): the locked exposure is k / f. */
        val flickerHz: Double = 0.0,
    )

    class SyncController(private val handle: Long) {
        private val out = DoubleArray(9)

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
        private val out = DoubleArray(9)

        /** Feed one frame's vertical luminance profile (per-row mean luma, 0..1). */
        fun onProfile(profile: DoubleArray): NsCommand {
            nativeOnProfile(handle, profile, out)
            return command(out)
        }

        /** Feed one frame's raw direct Y-buffer with SIMD profile extraction. */
        fun onYPlane(
            buffer: java.nio.Buffer,
            width: Int,
            height: Int,
            rowStride: Int,
            pixelStride: Int,
            rowStep: Int = 1,
            colStep: Int = 1,
        ): NsCommand {
            nativeOnYPlane(handle, buffer, width, height, rowStride, pixelStride, rowStep, colStep, out)
            return command(out)
        }

        /** Prefer exposures near this fraction of the frame time (0.5 = 180 degree shutter). */
        fun setShutterFraction(fraction: Double) {
            nativeSetShutterFraction(handle, fraction)
        }

        /** Shortest exposure the search uses: brightness budget / ISO limit (keeps the preview clean). */
        fun setMinExposure(us: Double) {
            nativeSetMinExposure(handle, us)
        }

        /** Longest exposure that does not overexpose: brightness budget / lowest ISO. */
        fun setMaxExposure(us: Double) {
            nativeSetMaxExposure(handle, us)
        }

        /** Start locked on a flicker found earlier, instead of searching again. */
        fun seed(exposureUs: Double, fHz: Double) {
            nativeSeedPwm(handle, exposureUs, fHz)
        }

        /** The flicker frequency found (0 while searching); the locked exposure is k / f, not 1 / f. */
        fun frequencyHz(): Double = nativeFrequencyPwm(handle)

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
        flickerHz = out[8],
    )
}

// Nested inside NsJni so the external functions stay private (no JNI name mangling).
// The aliases keep the call sites unchanged.
typealias CameraLimits = NsJni.CameraLimits
typealias NsCommand = NsJni.NsCommand
typealias SyncController = NsJni.SyncController
typealias PwmSyncController = NsJni.PwmSyncController
