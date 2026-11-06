package dev.nostrobe

/**
 * The frame rate a high-speed recording can use, from the camera's stream map. Paul's rule: the
 * frame must hold one flicker period (k = 1), so the rate is at most 0.9 x the flicker frequency.
 * Split out of `MainActivity.pickHighSpeed` so the choice can be tested without a camera.
 */
object HighSpeedPick {
    /** One entry of `SCALER_STREAM_CONFIGURATION_MAP.highSpeedVideoFpsRanges`. */
    data class FpsRange(val lower: Int, val upper: Int)
    /** One entry of `getHighSpeedVideoSizesFor`. */
    data class Size(val width: Int, val height: Int)

    /**
     * Fixed ranges only (`lower == upper`), at least 100 fps, at most 0.9 x `hz`, sorted by rate
     * descending. For the fastest qualifying rate, the largest size with `height <= maxHigh`; if
     * that rate has no such size, the next rate down. Returns (fps, width, height) or null.
     */
    fun pick(ranges: List<FpsRange>, sizesFor: (FpsRange) -> List<Size>, hz: Double,
             maxHigh: Int = 1080): Triple<Int, Int, Int>? {
        val fixed = ranges.filter { it.lower == it.upper && it.upper >= 100 && it.upper <= 0.9 * hz }
            .sortedByDescending { it.upper }
        for (r in fixed) {
            val size = sizesFor(r).filter { it.height <= maxHigh }
                .maxByOrNull { it.width.toLong() * it.height } ?: continue
            return Triple(r.upper, size.width, size.height)
        }
        return null
    }

    /**
     * The longest exposure that is a whole number of flicker periods and fits the frame:
     * `k = floor(0.9 x frame time x f)`, `e = k / f` microseconds.
     */
    fun exposureFor(frameUs: Double, hz: Double): Pair<Double, Double> {
        val k = kotlin.math.floor(0.9 * frameUs * hz / 1e6).coerceAtLeast(1.0)
        return k to k * 1e6 / hz
    }
}
