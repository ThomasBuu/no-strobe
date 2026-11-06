package dev.nostrobe

import android.content.Context
import android.util.AttributeSet
import android.widget.LinearLayout

/** A LinearLayout that never grows wider than MAX_DP: the controls stay compact on a tablet or an opened foldable. */
class MaxWidthLinearLayout @JvmOverloads constructor(
    context: Context, attrs: AttributeSet? = null,
) : LinearLayout(context, attrs) {
    override fun onMeasure(widthSpec: Int, heightSpec: Int) {
        val max = (MAX_DP * resources.displayMetrics.density).toInt()
        val w = if (MeasureSpec.getSize(widthSpec) > max) MeasureSpec.makeMeasureSpec(max, MeasureSpec.EXACTLY) else widthSpec
        super.onMeasure(w, heightSpec)
    }

    private companion object {
        const val MAX_DP = 460
    }
}
