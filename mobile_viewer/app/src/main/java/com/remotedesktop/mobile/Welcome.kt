package com.remotedesktop.mobile

import android.app.Activity
import android.app.AlertDialog
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.ViewGroup
import android.view.ViewOutlineProvider
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.Toast

/** Dialog mazmuni (PC versiyadagi `welcome.rs` bilan bir xil). Keyinchalik URL'dan olinadi: faqat shu obyektni almashtirish kifoya. */
object WelcomeInfo {
    /** Mazmun olinadigan manzil (hozircha belgilanmagan: o'rnatilgan matn ishlatiladi). */
    const val INFO_URL: String = ""

    val line1 = "MS Remote: boshqa kompyuterni uzoqdan, tez va xavfsiz boshqaring."
    val line2 = "Shifrlangan QUIC ulanishi, ID orqali yoki to'g'ridan-to'g'ri (P2P) ulanish. Bepul va ochiq."

    /** Donat uchun kartalar: (nomi, raqami). */
    val cards = listOf("Humo" to "9860176619947672", "Visa" to "4023060145526470")
}

private fun spaced(n: String) = n.chunked(4).joinToString(" ")

/** Ilova ochilganda: rasm, dastur haqida ikki qator va donat uchun karta raqamlari. */
fun Activity.showWelcome() {
    val radius = dp(16).toFloat()
    val banner = FrameLayout(this).apply {
        background = GradientDrawable(
            GradientDrawable.Orientation.TL_BR,
            intArrayOf(0xFF3454D6.toInt(), 0xFF8A46D2.toInt()),
        )
        // Yarim shaffof doiralar
        fun bubble(sizeDp: Int, gravity: Int, alpha: Int, mx: Int, my: Int) = addView(
            android.view.View(context).apply {
                background = GradientDrawable().apply {
                    shape = GradientDrawable.OVAL
                    setColor((alpha shl 24) or 0xFFFFFF)
                }
            },
            FrameLayout.LayoutParams(dp(sizeDp), dp(sizeDp), gravity).apply {
                setMargins(dp(mx), dp(my), dp(mx), dp(my))
            },
        )
        bubble(90, Gravity.TOP or Gravity.START, 0x28, 10, 12)
        bubble(60, Gravity.TOP or Gravity.END, 0x22, 24, 10)
        bubble(110, Gravity.BOTTOM or Gravity.END, 0x1C, 0, 0)
        bubble(46, Gravity.BOTTOM or Gravity.START, 0x1E, 30, 8)
        addView(
            ImageView(context).apply { setImageResource(R.mipmap.ic_launcher) },
            FrameLayout.LayoutParams(dp(96), dp(96), Gravity.CENTER),
        )
    }

    fun copy(number: String) {
        val cm = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
        cm.setPrimaryClip(ClipData.newPlainText("karta", number))
        Toast.makeText(this, "Nusxalandi", Toast.LENGTH_SHORT).show()
    }

    val body = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(18), dp(14), dp(18), dp(16))
        addView(label(WelcomeInfo.line1, 16f, bold = true))
        addView(label(WelcomeInfo.line2, 13f, Colors.DIM), lp(top = 6))
        addView(label("Loyihani qo'llab-quvvatlash (donat) uchun karta raqamlari:", 13f), lp(top = 12))
        for ((name, number) in WelcomeInfo.cards) {
            val row = LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                addView(label("$name:", 14f, bold = true))
                addView(
                    label(spaced(number), 15f).apply { typeface = android.graphics.Typeface.MONOSPACE },
                    LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { leftMargin = dp(8) },
                )
                addView(button("Nusxa") { copy(number) })
            }
            addView(row, lp(top = 6))
        }
    }

    val content = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        background = rounded(Colors.CARD, 16)
        clipToOutline = true
        outlineProvider = ViewOutlineProvider.BACKGROUND
        addView(banner, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, dp(150)))
        addView(body)
    }

    val dialog = AlertDialog.Builder(this).setView(content).create()
    content.addView(
        button("Davom etish", filled = true) { dialog.dismiss() },
        LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
            .apply { setMargins(dp(18), 0, dp(18), dp(18)) },
    )
    dialog.window?.setBackgroundDrawable(GradientDrawable().apply { setColor(0); cornerRadius = radius })
    dialog.show()
}
