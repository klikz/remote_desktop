package com.remotedesktop.mobile

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.ViewOutlineProvider
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Ochilish dialogi (PC versiyadagi `welcome.rs` bilan bir xil): to'lov ma'lumoti (text, phone, card) va reklama
 * (rasm + matn). Mazmun har ochilishda serverdan olinadi; oxirgi muvaffaqiyatli javob saqlanadi va aloqa
 * bo'lmaganda shu kesh ko'rsatiladi.
 */
private const val API = "http://cloud.hubnode.uz:8780/api/public"
private const val TIMEOUT_MS = 6000
private const val MAX_IMAGE_BYTES = 10 * 1024 * 1024
private const val MAX_IMAGE_SIDE = 1080
private const val BANNER_FILE = "welcome_banner.bin"

private data class WelcomeData(val text: String, val phone: String, val card: String, val adText: String) {
    val hasText get() = listOf(text, phone, card, adText).any { it.isNotEmpty() }
}

private fun Context.prefs() = getSharedPreferences("welcome", Context.MODE_PRIVATE)

private fun Context.loadCache(): WelcomeData = prefs().let {
    WelcomeData(it.getString("text", "") ?: "", it.getString("phone", "") ?: "", it.getString("card", "") ?: "", it.getString("adText", "") ?: "")
}

private fun Context.saveCache(d: WelcomeData) {
    prefs().edit().putString("text", d.text).putString("phone", d.phone).putString("card", d.card).putString("adText", d.adText).apply()
}

/** HTTP GET: faqat 200 javob tanasini qaytaradi (xato bo'lsa null). */
private fun httpGet(path: String, limit: Int): ByteArray? = try {
    val c = URL("$API$path").openConnection() as HttpURLConnection
    c.connectTimeout = TIMEOUT_MS
    c.readTimeout = TIMEOUT_MS
    try {
        if (c.responseCode != 200) null else c.inputStream.use { input ->
            val out = java.io.ByteArrayOutputStream()
            val buf = ByteArray(16 * 1024)
            while (true) {
                val n = input.read(buf)
                if (n < 0) break
                out.write(buf, 0, n)
                if (out.size() > limit) return null
            }
            out.toByteArray()
        }
    } finally {
        c.disconnect()
    }
} catch (e: Exception) {
    null
}

/** `"result" == "ok"` bo'lgan JSON. */
private fun getJson(path: String): JSONObject? {
    val body = httpGet(path, 256 * 1024) ?: return null
    return try {
        JSONObject(String(body, Charsets.UTF_8)).takeIf { it.optString("result") == "ok" }
    } catch (e: Exception) {
        null
    }
}

private fun JSONObject.str(k: String): String = if (isNull(k)) "" else optString(k, "").trim()

/** Rasmni o'lchami bo'yicha kichraytirib dekodlaydi (katta rasm xotirani to'ldirmasin). */
private fun decodeBanner(bytes: ByteArray): Bitmap? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    var sample = 1
    while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= MAX_IMAGE_SIDE) sample *= 2
    return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
}

/** Faqat raqamlardan iborat bo'lsa 4 tadan guruhlaydi. */
private fun spaced(n: String) = if (n.isNotEmpty() && n.all(Char::isDigit)) n.chunked(4).joinToString(" ") else n

/** Ilova ochilganda: text, phone, card, reklama rasmi va reklama matni. */
fun Activity.showWelcome() {
    val bannerFile = File(filesDir, BANNER_FILE)
    var data = loadCache()
    var banner: Bitmap? = if (bannerFile.exists()) runCatching { decodeBanner(bannerFile.readBytes()) }.getOrNull() else null

    // --- ko'rinishlar
    val textView = label("", 16f, bold = true)
    fun mono() = label("", 15f).apply { typeface = android.graphics.Typeface.MONOSPACE }
    val phoneView = mono()
    val cardView = mono()
    val phoneRow: View = phoneView
    val cardRow: View = cardView
    val maxImageH = (resources.displayMetrics.heightPixels * 0.45).toInt()
    val imageView = ImageView(this).apply {
        adjustViewBounds = true
        scaleType = ImageView.ScaleType.FIT_CENTER
        maxHeight = maxImageH
        background = rounded(Colors.BG, 12)
        clipToOutline = true
        outlineProvider = ViewOutlineProvider.BACKGROUND
    }
    val adView = label("", 14f)

    val body = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(18), dp(18), dp(18), dp(8))
        addView(textView)
        addView(phoneRow, lp(top = 10))
        addView(cardRow, lp(top = 6))
        addView(
            imageView,
            LinearLayout.LayoutParams(ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT).apply {
                topMargin = dp(14)
                gravity = Gravity.CENTER_HORIZONTAL
            },
        )
        addView(adView, lp(top = 10))
    }

    fun render() {
        fun set(v: TextView, row: View, value: String, shown: String = value) {
            v.text = shown
            v.tag = value
            row.visibility = if (value.isEmpty()) View.GONE else View.VISIBLE
        }
        set(textView, textView, data.text)
        set(phoneView, phoneRow, data.phone)
        set(cardView, cardRow, data.card, spaced(data.card))
        set(adView, adView, data.adText)
        imageView.setImageBitmap(banner)
        imageView.visibility = if (banner == null) View.GONE else View.VISIBLE
    }
    render()

    // Hammasi (tugma ham) ScrollView ichida: rasm katta bo'lsa ham tugmaga yetib borish mumkin
    val content = ScrollView(this).apply {
        background = rounded(Colors.CARD, 16)
        clipToOutline = true
        outlineProvider = ViewOutlineProvider.BACKGROUND
        addView(body)
    }
    val dialog = AlertDialog.Builder(this).setView(content).create()
    body.addView(
        button("Davom etish", filled = true) { dialog.dismiss() },
        LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT)
            .apply { setMargins(0, dp(14), 0, dp(10)) },
    )
    dialog.window?.setBackgroundDrawable(GradientDrawable().apply { setColor(0); cornerRadius = dp(16).toFloat() })

    // Kesh bo'lsa darhol ko'rsatiladi; bo'lmasa ma'lumot kelgach (hech narsa kelmasa umuman chiqmaydi).
    fun showIfReady() {
        if (!dialog.isShowing && !isFinishing && (data.hasText || banner != null)) dialog.show()
    }
    showIfReady()

    Thread {
        val pay = getJson("/payment-info")
        val ad = getJson("/ad-text")
        val bytes = httpGet("/banner-image", MAX_IMAGE_BYTES)
        val bmp = bytes?.let { runCatching { decodeBanner(it) }.getOrNull() }
        runOnUiThread {
            if (isFinishing || isDestroyed) return@runOnUiThread
            if (pay != null) data = data.copy(text = pay.str("text"), phone = pay.str("phone"), card = pay.str("card"))
            if (ad != null) data = data.copy(adText = ad.str("text"))
            if (pay != null || ad != null) saveCache(data)
            if (bmp != null) {
                banner = bmp
                runCatching { bannerFile.writeBytes(bytes!!) }
            }
            render()
            showIfReady()
        }
    }.start()
}
