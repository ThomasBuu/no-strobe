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

    override fun onMeasure(widthSpec: Int, heightSpec: Int) {
        val maxW = MeasureSpec.getSize(widthSpec)
        val maxH = MeasureSpec.getSize(heightSpec)
        if (maxW <= 0 || maxH <= 0) {
            super.onMeasure(widthSpec, heightSpec)
            return
        }
        val m = resources.displayMetrics
        val ratio = if (m.widthPixels > m.heightPixels) 16f / 9f else 9f / 16f  // width / height
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
