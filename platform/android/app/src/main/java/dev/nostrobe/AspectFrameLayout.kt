package dev.nostrobe

import android.content.Context
import android.util.AttributeSet
import android.widget.FrameLayout

/**
 * A FrameLayout that keeps the viewfinder's aspect ratio inside the space it is given: 16:9 when
 * the screen is wider than tall (landscape, or a foldable opened flat and held sideways), 9:16
 * otherwise. The camera frame is only scaled into the box, so a box with the frame's own
 * ratio shows it undistorted.
 */
class AspectFrameLayout @JvmOverloads constructor(
    context: Context, attrs: AttributeSet? = null,
) : FrameLayout(context, attrs) {

    private var aspectWidth = 16
    private var aspectHeight = 9

    /**
     * Sets the camera stream or video aspect ratio (e.g. 1920x1080 or 16:9).
     */
    fun setAspectRatio(width: Int, height: Int) {
        if (width <= 0 || height <= 0) return
        val w = maxOf(width, height)
        val h = minOf(width, height)
        if (aspectWidth != w || aspectHeight != h) {
            aspectWidth = w
            aspectHeight = h
            requestLayout()
        }
    }

    override fun onMeasure(widthSpec: Int, heightSpec: Int) {
        val maxW = MeasureSpec.getSize(widthSpec)
        val maxH = MeasureSpec.getSize(heightSpec)
        if (maxW <= 0 || maxH <= 0) {
            super.onMeasure(widthSpec, heightSpec)
            return
        }
        val isLandscape = resources.configuration.orientation == android.content.res.Configuration.ORIENTATION_LANDSCAPE
            || maxW > maxH
        val targetRatio = aspectWidth.toFloat() / aspectHeight.toFloat()
        val ratio = if (isLandscape) targetRatio else (1f / targetRatio)  // width / height
        var w = maxW
        var h = (maxW / ratio).toInt()
        if (h > maxH) {
            h = maxH
            w = (maxH * ratio).toInt()
        }
        super.onMeasure(
            MeasureSpec.makeMeasureSpec(w, MeasureSpec.EXACTLY),
            MeasureSpec.makeMeasureSpec(h, MeasureSpec.EXACTLY),
        )
    }
}
