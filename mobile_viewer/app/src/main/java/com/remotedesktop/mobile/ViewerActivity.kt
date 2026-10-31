package com.remotedesktop.mobile

import android.app.Activity
import android.app.AlertDialog
import android.content.Context
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Gravity
import android.view.KeyEvent
import android.view.SurfaceHolder
import android.view.View
import android.view.WindowManager
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.FrameLayout
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.TextView

interface KeySink {
    fun typeText(s: String)
    fun androidKey(keyCode: Int, down: Boolean): Boolean
}

/** Ko'rinmas, fokus oladigan view: soft klaviatura (IME) matni va tugmalarini `KeySink` ga uzatadi. */
class KeyboardView(context: Context, private val sink: KeySink) : View(context) {
    init {
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun onCheckIsTextEditor() = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_ACTION_NONE
        return object : BaseInputConnection(this, false) {
            private var composing = ""

            private fun setComposing(t: String) {
                var common = 0
                while (common < composing.length && common < t.length && composing[common] == t[common]) common++
                repeat(composing.length - common) { tap(KeyEvent.KEYCODE_DEL) }
                sink.typeText(t.substring(common))
                composing = t
            }

            private fun tap(code: Int) {
                sink.androidKey(code, true)
                sink.androidKey(code, false)
            }

            override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
                setComposing(text.toString())
                return true
            }

            override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
                setComposing(text.toString())
                composing = ""
                return true
            }

            override fun finishComposingText(): Boolean {
                composing = ""
                return true
            }

            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                if (composing.isNotEmpty()) composing = composing.dropLast(minOf(beforeLength, composing.length))
                repeat(maxOf(beforeLength, 1)) { tap(KeyEvent.KEYCODE_DEL) }
                return true
            }

            override fun sendKeyEvent(event: KeyEvent): Boolean {
                composing = ""
                return sink.androidKey(event.keyCode, event.action == KeyEvent.ACTION_DOWN)
            }

            override fun performEditorAction(actionCode: Int): Boolean {
                tap(KeyEvent.KEYCODE_ENTER)
                return true
            }
        }
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean = sink.androidKey(keyCode, true) || super.onKeyDown(keyCode, event)
    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean = sink.androidKey(keyCode, false) || super.onKeyUp(keyCode, event)
}

/** Ko'rish ekrani: video (MediaCodec -> SurfaceView), klaviatura tugmasi, sichqoncha rejimi, zoom. */
class ViewerActivity : Activity(), KeySink {
    private lateinit var settings: Settings
    private lateinit var container: VideoContainer
    private lateinit var kb: KeyboardView
    private lateinit var overlay: TextView
    private lateinit var stats: TextView
    private lateinit var extraRow: HorizontalScrollView
    private lateinit var modeBtn: Button
    private lateinit var kbBtn: Button
    private val modButtons = HashMap<Int, Button>()
    private val modOn = HashSet<Int>()

    private val handler = Handler(Looper.getMainLooper())
    private var decoder: VideoDecoder? = null
    private var surfaceReady = false
    private var videoStarted = false
    private var kbVisible = false
    private var targetId = ""
    private var lastStatsAt = 0L
    private var lastFrames = 0L
    private var lastBytes = 0L

    private val poll = object : Runnable {
        override fun run() {
            while (true) {
                val ev = Native.pollEvent() ?: break
                handleEvent(ev.split('\t'))
            }
            updateStats()
            handler.postDelayed(this, 100)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        settings = Settings(this)
        targetId = intent.getStringExtra("id").orEmpty()
        buildUi()
        val ok = Native.connect(
            targetId,
            intent.getStringExtra("password").orEmpty(),
            intent.getIntExtra("fps", 30),
            intent.getIntExtra("kbps", 8000),
            intent.getIntExtra("height", 1080),
        )
        if (!ok) overlay.text = "Agent ishlamayapti. Server sozlamalarini tekshiring."
        else overlay.text = "Ulanilmoqda...\nHost ruxsat berishini kuting"
        handler.post(poll)
    }

    private fun buildUi() {
        container = VideoContainer(this).apply { touchpad = settings.touchpad }
        container.surface.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(h: SurfaceHolder) {
                surfaceReady = true
                maybeStartDecoder()
            }

            override fun surfaceChanged(h: SurfaceHolder, f: Int, w: Int, hh: Int) {}
            override fun surfaceDestroyed(h: SurfaceHolder) {
                surfaceReady = false
                stopDecoder()
            }
        })

        kbBtn = bar("⌨ Klaviatura") { toggleKeyboard() }
        modeBtn = bar(modeLabel()) {
            settings.touchpad = !settings.touchpad
            container.touchpad = settings.touchpad
            modeBtn.text = modeLabel()
        }
        val zoomBtn = bar("1×") { container.resetZoom() }
        stats = label("", 12f, Colors.DIM).apply { gravity = Gravity.CENTER_VERTICAL or Gravity.END }
        val closeBtn = bar("✕") { finish() }
        val top = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setBackgroundColor(Colors.BG)
            setPadding(dp(6), dp(4), dp(6), dp(4))
            addView(kbBtn)
            addView(modeBtn, lp(width = LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = dp(6) })
            addView(zoomBtn, lp(width = LinearLayout.LayoutParams.WRAP_CONTENT).apply { leftMargin = dp(6) })
            addView(stats, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.MATCH_PARENT, 1f).apply { leftMargin = dp(8); rightMargin = dp(8) })
            addView(closeBtn)
        }

        val extra = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            setPadding(dp(6), dp(2), dp(6), dp(2))
            fun key(text: String, code: Int) = addView(bar(text) { tapKey(code) }, lp(width = LinearLayout.LayoutParams.WRAP_CONTENT).apply { rightMargin = dp(4) })
            fun mod(text: String, code: Int) {
                val b = bar(text) { toggleMod(code) }
                modButtons[code] = b
                addView(b, lp(width = LinearLayout.LayoutParams.WRAP_CONTENT).apply { rightMargin = dp(4) })
            }
            key("Esc", KeyMap.ESC)
            key("Tab", KeyMap.TAB)
            mod("Ctrl", KeyMap.CTRL)
            mod("Alt", KeyMap.ALT)
            mod("Win", KeyMap.WIN)
            key("Del", KeyMap.DELETE)
            key("←", KeyMap.LEFT)
            key("↑", KeyMap.UP)
            key("↓", KeyMap.DOWN)
            key("→", KeyMap.RIGHT)
        }
        extraRow = HorizontalScrollView(this).apply {
            visibility = View.GONE
            setBackgroundColor(Colors.BG)
            isHorizontalScrollBarEnabled = false
            addView(extra)
        }

        val column = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(top, lp())
            addView(extraRow, lp())
            addView(container, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
        }
        overlay = label("", 15f).apply {
            gravity = Gravity.CENTER
            setPadding(dp(24), dp(24), dp(24), dp(24))
        }
        kb = KeyboardView(this, this)
        val root = FrameLayout(this).apply {
            setBackgroundColor(0xFF000000.toInt())
            addView(column, FrameLayout.LayoutParams(-1, -1))
            addView(this@ViewerActivity.overlay, FrameLayout.LayoutParams(-1, -1).apply { topMargin = dp(48) })
            addView(kb, FrameLayout.LayoutParams(1, 1))
        }
        setContentView(root)
        root.applySafeArea(onIme = { ime -> if (kbVisible != ime) setKeyboardUi(ime) })
    }

    private fun modeLabel() = if (settings.touchpad) "Touchpad" else "Sensor"

    private fun bar(text: String, onClick: () -> Unit): Button = button(text, onClick = onClick).apply {
        textSize = 13f
        minimumHeight = dp(36)
        background = rounded(Colors.CARD, 8)
    }

    // --- hodisalar ------------------------------------------------------------------------------

    private fun handleEvent(f: List<String>) {
        when (f[0]) {
            "confirm" -> AlertDialog.Builder(this)
                .setTitle("Yangi kompyuter")
                .setMessage("Bu kompyuterga birinchi marta ulanyapsiz.\nFingerprint:\n${f.getOrElse(1) { "" }}\n\nIshonasizmi?")
                .setCancelable(false)
                .setPositiveButton("Ishonaman") { _, _ -> Native.confirm(true) }
                .setNegativeButton("Bekor") { _, _ -> Native.confirm(false) }
                .show()
            "connected" -> {
                settings.addContact(targetId)
                AgentManager.subscribe()
                overlay.text = "Video kutilmoqda..."
            }
            "video" -> {
                videoStarted = true
                overlay.visibility = View.GONE
                maybeStartDecoder()
            }
            "ended" -> {
                val err = f.getOrElse(1) { "" }
                stopDecoder()
                videoStarted = false
                if (err.isEmpty()) {
                    finish()
                } else {
                    overlay.visibility = View.VISIBLE
                    overlay.text = "Ulanish tugadi:\n$err"
                    overlay.setTextColor(Colors.BAD)
                }
            }
        }
    }

    private fun maybeStartDecoder() {
        if (!videoStarted || !surfaceReady || decoder != null) return
        decoder = VideoDecoder(container.surface.holder.surface) { w, h -> runOnUiThread { container.setVideoSize(w, h) } }.also { it.start() }
    }

    private fun stopDecoder() {
        decoder?.stop()
        decoder = null
    }

    private fun updateStats() {
        val now = System.currentTimeMillis()
        if (now - lastStatsAt < 1000) return
        val s = Native.stats() ?: return
        val dt = (now - lastStatsAt).coerceAtLeast(1) / 1000.0
        if (lastStatsAt != 0L) {
            val fps = (s[0] - lastFrames) / dt
            val mbit = (s[1] - lastBytes) * 8 / dt / 1e6
            stats.text = "%.0f FPS · %.1f Mbit/s · %d ms".format(fps, mbit, s[2] / 1000)
        }
        lastStatsAt = now
        lastFrames = s[0]
        lastBytes = s[1]
    }

    // --- klaviatura -----------------------------------------------------------------------------

    private fun toggleKeyboard() {
        val imm = getSystemService(Context.INPUT_METHOD_SERVICE) as InputMethodManager
        if (kbVisible) {
            imm.hideSoftInputFromWindow(kb.windowToken, 0)
            setKeyboardUi(false)
        } else {
            kb.requestFocus()
            imm.showSoftInput(kb, 0)
            setKeyboardUi(true)
        }
    }

    private fun setKeyboardUi(visible: Boolean) {
        kbVisible = visible
        extraRow.visibility = if (visible) View.VISIBLE else View.GONE
        kbBtn.background = rounded(if (visible) Colors.ACCENT else Colors.CARD, 8)
    }

    private fun toggleMod(code: Int) {
        val on = modOn.add(code)
        if (!on) modOn.remove(code)
        Native.sendKey(code, on)
        modButtons[code]?.background = rounded(if (on) Colors.ACCENT else Colors.CARD, 8)
    }

    /** Ctrl/Alt/Win bir martalik: keyingi tugmadan keyin qo'yib yuboriladi. */
    private fun releaseMods() {
        for (code in modOn.toList()) toggleMod(code)
    }

    private fun tapKey(code: Int, shift: Boolean = false) {
        if (shift) Native.sendKey(KeyMap.SHIFT, true)
        Native.sendKey(code, true)
        Native.sendKey(code, false)
        if (shift) Native.sendKey(KeyMap.SHIFT, false)
        releaseMods()
    }

    override fun typeText(s: String) {
        for (c in s) KeyMap.forChar(c)?.let { (code, shift) -> tapKey(code, shift) }
    }

    override fun androidKey(keyCode: Int, down: Boolean): Boolean {
        val code = KeyMap.forAndroid(keyCode) ?: return false
        Native.sendKey(code, down)
        if (!down && code != KeyMap.CTRL && code != KeyMap.ALT && code != KeyMap.SHIFT && code != KeyMap.WIN) releaseMods()
        return true
    }

    // --- hayot sikli ----------------------------------------------------------------------------

    override fun onStop() {
        super.onStop()
        // v1: fon rejimida sessiya yo'q — ekran yopilsa ulanish tugatiladi
        if (!isChangingConfigurations) finish()
    }

    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null)
        stopDecoder()
        Native.close()
        super.onDestroy()
    }
}
