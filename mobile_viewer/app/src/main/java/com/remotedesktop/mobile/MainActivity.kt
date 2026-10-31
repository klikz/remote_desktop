package com.remotedesktop.mobile

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.text.InputType
import android.view.View
import android.view.ViewGroup
import android.widget.AdapterView
import android.widget.ArrayAdapter
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.RadioButton
import android.widget.RadioGroup
import android.widget.ScrollView
import android.widget.SeekBar
import android.widget.Spinner
import android.widget.TextView
import kotlin.math.exp
import kotlin.math.ln
import kotlin.math.roundToInt

/** Bosh ekran: server sozlamalari, ID + parol, sifat shablonlari va kontaktlar. */
class MainActivity : Activity() {
    private lateinit var settings: Settings
    private lateinit var statusView: TextView
    private lateinit var serverTitle: TextView
    private lateinit var serverState: TextView
    private lateinit var serverEdit: EditText
    private lateinit var keyEdit: EditText
    private lateinit var nameEdit: EditText
    private lateinit var idEdit: EditText
    private lateinit var passEdit: EditText
    private lateinit var contactsBox: LinearLayout
    private lateinit var presetGroup: RadioGroup
    private lateinit var fpsBar: SeekBar
    private lateinit var kbpsBar: SeekBar
    private lateinit var fpsLabel: TextView
    private lateinit var kbpsLabel: TextView
    private lateinit var heightSpinner: Spinner
    private var updating = false

    private var fps = 30
    private var kbps = 8000
    private var height = 1080

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        settings = Settings(this)
        Native.init(filesDir.absolutePath)
        fps = settings.fps
        kbps = settings.kbps
        height = settings.height
        build()
        if (!AgentManager.isStarted) AgentManager.start(settings)
    }

    override fun onResume() {
        super.onResume()
        AgentManager.listener = {
            refreshStatus()
            refreshContacts()
        }
        refreshStatus()
        refreshContacts()
        AgentManager.subscribe()
    }

    override fun onPause() {
        AgentManager.listener = null
        super.onPause()
    }

    private fun build() {
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(24), dp(16), dp(24))
        }
        root.addView(label("MS Remote", 22f, bold = true))
        statusView = label("", 13f, Colors.DIM)
        root.addView(statusView, lp(top = 2))

        // --- ulanish kartasi
        idEdit = edit("9 xonali ID", settings.lastTarget, InputType.TYPE_CLASS_NUMBER)
        passEdit = edit("Kalit (agar kerak bo'lsa)", "", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
        presetGroup = RadioGroup(this).apply {
            PRESETS.forEachIndexed { i, p ->
                addView(
                    RadioButton(context).apply {
                        id = 1000 + i
                        text = p.name
                        setTextColor(Colors.TEXT)
                    },
                    LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT),
                )
            }
            setOnCheckedChangeListener { _, checked ->
                if (updating || checked < 1000) return@setOnCheckedChangeListener
                val p = PRESETS[checked - 1000]
                fps = p.fps
                kbps = p.kbps
                this@MainActivity.height = p.height
                syncQualityViews()
            }
        }
        val advanced = advancedBox()
        root.addView(
            card(
                label("Server orqali ulanish", 16f, bold = true),
                label("Boshqa kompyuterning ID sini kiriting. Ruxsat berilgach ulanasiz.", 12f, Colors.DIM),
                idEdit,
                passEdit,
                label("Sifat", 13f, Colors.DIM),
                presetGroup,
                button("Qo'shimcha sozlamalar") {
                    advanced.visibility = if (advanced.visibility == View.GONE) View.VISIBLE else View.GONE
                },
                advanced,
                button("Ulanish", filled = true) { connect() },
            ),
            lp(top = 16),
        )
        syncQualityViews()

        // --- kontaktlar
        contactsBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        root.addView(card(label("Kontaktlar", 16f, bold = true), contactsBox), lp(top = 12))

        // --- server sozlamalari
        serverEdit = edit("Server manzili (host yoki host:port)", settings.server, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        keyEdit = edit("Server kaliti", settings.key, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
        nameEdit = edit("Bu qurilma nomi", settings.name)
        serverTitle = label("Server sozlamalari", 16f, bold = true)
        serverState = label("", 13f, Colors.DIM, bold = true)
        val serverHeader = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = android.view.Gravity.CENTER_VERTICAL
            addView(serverTitle, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(serverState)
        }
        root.addView(
            card(
                serverHeader,
                serverEdit,
                keyEdit,
                nameEdit,
                button("Saqlash va qayta ulanish") {
                    settings.server = serverEdit.text.toString()
                    settings.key = keyEdit.text.toString()
                    settings.name = nameEdit.text.toString()
                    AgentManager.start(settings)
                },
            ),
            lp(top = 12),
        )

        val scroll = ScrollView(this).apply {
            setBackgroundColor(Colors.BG)
            isFillViewport = true
            clipToPadding = false
            addView(root)
        }
        scroll.applySafeArea()
        setContentView(scroll)
    }

    /** FPS / bitrate / o'lcham slayderlari. Bitrate logarifmik (500..50000 kbit/s) — desktop bilan bir xil. */
    private fun advancedBox(): LinearLayout {
        fpsLabel = label("", 13f, Colors.DIM)
        kbpsLabel = label("", 13f, Colors.DIM)
        fpsBar = SeekBar(this).apply {
            max = 55 // 5..60
            setOnSeekBarChangeListener(seek { fps = it + 5 })
        }
        kbpsBar = SeekBar(this).apply {
            max = 1000
            setOnSeekBarChangeListener(seek { kbps = barToKbps(it) })
        }
        heightSpinner = Spinner(this).apply {
            adapter = ArrayAdapter(context, android.R.layout.simple_spinner_dropdown_item, HEIGHTS.map { it.second })
            onItemSelectedListener = object : AdapterView.OnItemSelectedListener {
                override fun onItemSelected(p: AdapterView<*>?, v: View?, pos: Int, id: Long) {
                    if (updating) return
                    this@MainActivity.height = HEIGHTS[pos].first
                    syncQualityViews()
                }

                override fun onNothingSelected(p: AdapterView<*>?) {}
            }
        }
        return LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            visibility = View.GONE
            addView(fpsLabel)
            addView(fpsBar)
            addView(kbpsLabel)
            addView(kbpsBar)
            addView(label("O'lcham", 13f, Colors.DIM))
            addView(heightSpinner)
        }
    }

    private fun seek(onChange: (Int) -> Unit) = object : SeekBar.OnSeekBarChangeListener {
        override fun onProgressChanged(s: SeekBar?, progress: Int, fromUser: Boolean) {
            if (!fromUser || updating) return
            onChange(progress)
            syncQualityViews(fromBars = true)
        }

        override fun onStartTrackingTouch(s: SeekBar?) {}
        override fun onStopTrackingTouch(s: SeekBar?) {}
    }

    private fun kbpsToBar(k: Int) = ((ln(k.toDouble() / 500) / ln(100.0)) * 1000).roundToInt().coerceIn(0, 1000)
    private fun barToKbps(b: Int) = (500 * exp(b / 1000.0 * ln(100.0))).roundToInt().coerceIn(500, 50_000)

    private fun syncQualityViews(fromBars: Boolean = false) {
        updating = true
        val idx = PRESETS.indexOfFirst { it.fps == fps && it.kbps == kbps && it.height == height }
        if (idx >= 0) presetGroup.check(1000 + idx) else presetGroup.clearCheck()
        fpsLabel.text = "FPS: $fps"
        kbpsLabel.text = "Bitrate: $kbps kbit/s"
        if (!fromBars) {
            fpsBar.progress = (fps - 5).coerceIn(0, 55)
            kbpsBar.progress = kbpsToBar(kbps)
        }
        heightSpinner.setSelection(HEIGHTS.indexOfFirst { it.first == height }.coerceAtLeast(0))
        updating = false
    }

    private fun refreshStatus() {
        val id = settings.id
        statusView.text = if (AgentManager.online) {
            "Onlayn · sizning ID: ${if (id.length == 9) id else "—"}"
        } else {
            "Oflayn · ${AgentManager.status}"
        }
        statusView.setTextColor(if (AgentManager.online) Colors.GOOD else Colors.DIM)
        serverTitle.setTextColor(if (AgentManager.online) Colors.GOOD else Colors.TEXT)
        serverState.text = if (AgentManager.online) "Ulandi ✓" else AgentManager.status
        serverState.setTextColor(if (AgentManager.online) Colors.GOOD else Colors.DIM)
    }

    private fun refreshContacts() {
        contactsBox.removeAllViews()
        val list = settings.contacts
        if (list.isEmpty()) {
            contactsBox.addView(label("Hozircha yo'q. Muvaffaqiyatli ulanishdan keyin shu yerda paydo bo'ladi.", 12f, Colors.DIM))
            return
        }
        for (c in list) {
            val pres = AgentManager.presence[c.id]
            val on = pres?.first == true
            val name = c.name.ifBlank { pres?.second.orEmpty() }
            val text = "${if (on) "●" else "○"}  ${c.id}${if (name.isNotBlank()) "  ·  $name" else ""}"
            val row = label(text, 15f, if (on) Colors.TEXT else Colors.DIM)
            row.setPadding(0, dp(8), 0, dp(8))
            row.setOnClickListener { idEdit.setText(c.id) }
            row.setOnLongClickListener {
                AlertDialog.Builder(this).setMessage("${c.id} kontaktini o'chirish?")
                    .setPositiveButton("O'chirish") { _, _ ->
                        settings.contacts = settings.contacts.filter { it.id != c.id }
                        AgentManager.subscribe()
                        refreshContacts()
                    }.setNegativeButton("Bekor", null).show()
                true
            }
            contactsBox.addView(row)
        }
    }

    private fun connect() {
        val id = idEdit.text.toString().trim()
        if (id.length != 9 || !id.all(Char::isDigit)) {
            idEdit.error = "9 ta raqam kerak"
            return
        }
        if (!AgentManager.online) {
            AlertDialog.Builder(this)
                .setMessage("Server bilan aloqa yo'q (${AgentManager.status}). Server sozlamalarini tekshiring.")
                .setPositiveButton("OK", null).show()
            return
        }
        settings.fps = fps
        settings.kbps = kbps
        settings.height = height
        settings.lastTarget = id
        startActivity(
            Intent(this, ViewerActivity::class.java)
                .putExtra("id", id)
                .putExtra("password", passEdit.text.toString())
                .putExtra("fps", fps)
                .putExtra("kbps", kbps)
                .putExtra("height", height),
        )
    }
}
