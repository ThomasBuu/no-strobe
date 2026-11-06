package dev.nostrobe

import android.graphics.ImageFormat
import android.media.Image
import java.nio.ByteBuffer

/**
 * Vertical luminance profile of a YUV_420_888 frame: the mean luma of each row.
 *
 * A rolling shutter reads rows one after another, so the row index is a time axis at
 * the sensor readout rate. PWM flicker shows up as banding along this profile.
 *
 * `step` subsamples rows. When `step > 1`, the profile's sample rate is
 * `lineRateHz / step`, so pass that value to `NsJni.createPwm`.
 */
object LineProfileExtractor {
    fun rowProfile(image: Image, step: Int = 1): DoubleArray {
        if (image.format != ImageFormat.YUV_420_888) return DoubleArray(0)
        val y = image.planes[0] ?: return DoubleArray(0)
        // Image.Plane.getBuffer() is typed java.nio.Buffer in android.jar; the
        // indexed get(int) is only on ByteBuffer.
        val buf = (y.buffer as ByteBuffer?) ?: return DoubleArray(0)
        if (buf.remaining() <= 0) return DoubleArray(0)

        val rowStride = y.rowStride
        val pixelStride = y.pixelStride
        val width = image.width
        val height = image.height
        val s = step.coerceAtLeast(1)
        val out = DoubleArray(height / s)

        for (i in 0 until height / s) {
            val row = i * s
            var sum = 0L
            var count = 0
            var col = 0
            while (col < width) {
                val offset = row * rowStride + col * pixelStride
                if (offset < buf.limit()) {
                    sum += buf.get(offset).toInt() and 0xFF
                    count++
                }
                col += 4
            }
            out[i] = if (count > 0) sum.toDouble() / (count.toDouble() * 255.0) else 0.0
        }
        return out
    }

    /**
     * Row readout rate in rows per second.
     *
     * Android publishes `SENSOR_INFO_ROLLING_SHUTTER_SUSTAIN_DURATION` (nanoseconds):
     * the time the sensor takes to read out all rows. `height / sustain` is the rate.
     * Returns 0.0 when the device does not publish it, which puts the controller in
     * sweep-calibration mode.
     */
    fun lineRate(height: Int, sustainNs: Double): Double {
        if (sustainNs <= 0.0) return 0.0
        return height.toDouble() * 1e9 / sustainNs
    }
}
