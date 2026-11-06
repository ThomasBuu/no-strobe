package dev.nostrobe

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.util.AttributeSet
import android.view.View

/** Rule-of-thirds grid drawn over the preview; does not take touches. */
class GridOverlay @JvmOverloads constructor(
    context: Context, attrs: AttributeSet? = null,
) : View(context, attrs) {
    private val paint = Paint().apply {
        color = 0x99FFFFFF.toInt()
        strokeWidth = 2f * resources.displayMetrics.density
        isAntiAlias = true
    }

    init {
        isClickable = false
        isFocusable = false
    }

    override fun onDraw(canvas: Canvas) {
        val w = width.toFloat()
        val h = height.toFloat()
        for (i in 1..2) {
            canvas.drawLine(w * i / 3f, 0f, w * i / 3f, h, paint)
            canvas.drawLine(0f, h * i / 3f, w, h * i / 3f, paint)
        }
    }
}
