package com.remotedesktop.mobile

import android.content.Context
import android.graphics.Color
import android.graphics.drawable.GradientDrawable
import android.text.InputType
import android.util.TypedValue
import android.view.View
import android.view.ViewGroup
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView

object Colors {
    const val BG = 0xFF14171C.toInt()
    const val CARD = 0xFF1E232B.toInt()
    const val TEXT = 0xFFE6E9EF.toInt()
    const val DIM = 0xFF8D96A5.toInt()
    const val ACCENT = 0xFF4C8DFF.toInt()
    const val GOOD = 0xFF4CC38A.toInt()
    const val BAD = 0xFFFF6B6B.toInt()
}

fun Context.dp(v: Int): Int = TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), resources.displayMetrics).toInt()

fun Context.rounded(color: Int, radiusDp: Int = 10): GradientDrawable =
    GradientDrawable().apply {
        setColor(color)
        cornerRadius = dp(radiusDp).toFloat()
    }

fun Context.label(text: String, sizeSp: Float = 14f, color: Int = Colors.TEXT, bold: Boolean = false): TextView =
    TextView(this).apply {
        this.text = text
        textSize = sizeSp
        setTextColor(color)
        if (bold) setTypeface(typeface, android.graphics.Typeface.BOLD)
    }

fun Context.edit(hint: String, text: String = "", inputType: Int = InputType.TYPE_CLASS_TEXT): EditText =
    EditText(this).apply {
        this.hint = hint
        setText(text)
        this.inputType = inputType
        setTextColor(Colors.TEXT)
        setHintTextColor(Colors.DIM)
        background = rounded(Colors.BG, 8)
        setPadding(dp(12), dp(10), dp(12), dp(10))
        isSingleLine = true
    }

fun Context.button(text: String, filled: Boolean = false, onClick: () -> Unit): Button =
    Button(this).apply {
        this.text = text
        isAllCaps = false
        setTextColor(if (filled) Color.WHITE else Colors.TEXT)
        background = rounded(if (filled) Colors.ACCENT else Colors.BG, 8)
        minHeight = 0
        minimumHeight = dp(40)
        minWidth = 0
        minimumWidth = 0
        setPadding(dp(14), dp(6), dp(14), dp(6))
        setOnClickListener { onClick() }
    }

fun Context.lp(top: Int = 0, width: Int = ViewGroup.LayoutParams.MATCH_PARENT): LinearLayout.LayoutParams =
    LinearLayout.LayoutParams(width, ViewGroup.LayoutParams.WRAP_CONTENT).apply { topMargin = dp(top) }

fun Context.card(vararg children: View): LinearLayout =
    LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        background = rounded(Colors.CARD, 12)
        setPadding(dp(14), dp(12), dp(14), dp(14))
        for (c in children) addView(c, lp(top = 8))
    }

/** Safe area: status/navigation panel, kesim (cutout) va klaviatura (IME) ostida qolmaslik uchun padding beradi. */
fun View.applySafeArea(extraDp: Int = 0, onIme: ((Boolean) -> Unit)? = null) {
    val extra = (extraDp * resources.displayMetrics.density).toInt()
    setOnApplyWindowInsetsListener { v, insets ->
        val l: Int
        val t: Int
        val r: Int
        val b: Int
        if (android.os.Build.VERSION.SDK_INT >= 30) {
            val bars = insets.getInsets(android.view.WindowInsets.Type.systemBars() or android.view.WindowInsets.Type.displayCutout())
            val ime = insets.getInsets(android.view.WindowInsets.Type.ime())
            l = bars.left; t = bars.top; r = bars.right; b = maxOf(bars.bottom, ime.bottom)
            onIme?.invoke(insets.isVisible(android.view.WindowInsets.Type.ime()))
        } else {
            @Suppress("DEPRECATION")
            run {
                l = insets.systemWindowInsetLeft; t = insets.systemWindowInsetTop
                r = insets.systemWindowInsetRight; b = insets.systemWindowInsetBottom
            }
        }
        v.setPadding(l + extra, t + extra, r + extra, b + extra)
        insets
    }
}
