package dev.nostrobe

import android.graphics.ImageFormat
import android.media.Image
import java.nio.ByteBuffer

/** Mean luma of a YUV_420_888 frame, normalized to 0..1. */
object LumaExtractor {
    private const val STEP = 8

    fun meanLuma(image: Image): Double {
        if (image.format != ImageFormat.YUV_420_888) return 0.0
        val y = image.planes[0] ?: return 0.0
        // Image.Plane.getBuffer() is typed java.nio.Buffer in android.jar; the
        // indexed get(int) is only on ByteBuffer.
        val buf = (y.buffer as ByteBuffer?) ?: return 0.0
        if (buf.remaining() <= 0) return 0.0

        val rowStride = y.rowStride
        val pixelStride = y.pixelStride
        val width = image.width
        val height = image.height
        var sum = 0.0
        var count = 0L

        var row = 0
        while (row < height) {
            var col = 0
            while (col < width) {
                val offset = row * rowStride + col * pixelStride
                if (offset < buf.limit()) {
                    sum += buf.get(offset).toInt() and 0xFF
                    count++
                }
                col += STEP
            }
            row += STEP
        }

        if (count == 0L) return 0.0
        return sum / (count.toDouble() * 255.0)
    }
}
