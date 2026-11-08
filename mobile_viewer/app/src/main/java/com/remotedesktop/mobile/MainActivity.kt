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
    private lateinit var reconnectBtn: android.widget.Button
    private lateinit var serverTitle: TextView
    private lateinit var serverState: TextView
    private lateinit var serverEdit: EditText
    private lateinit var keyEdit: EditText
    private lateinit var nameEdit: EditText
    private lateinit var idEdit: EditText
    private lateinit var passEdit: EditText
    private lateinit var addrEdit: EditText
    private lateinit var descLabel: TextView
    private lateinit var modeRelayBtn: android.widget.Button
    private lateinit var modeP2pBtn: android.widget.Button
    private lateinit var contactsCard: LinearLayout
    private lateinit var p2pCard: LinearLayout
    private lateinit var p2pBox: LinearLayout
    private var p2pMode = false
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
        p2pMode = settings.p2p
        fps = settings.fps
        kbps = settings.kbps
        height = settings.height
        build()
        if (!AgentManager.isStarted) AgentManager.start(settings)
        // Ilova ochilganda bir marta (ekran aylantirilganda qayta chiqmaydi)
        if (savedInstanceState == null) showWelcome()
    }

    override fun onResume() {
        super.onResume()
        refreshP2pHosts()
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
        // Server bilan aloqa yo'q bo'lsa: kutib o'tirmasdan darhol qayta ulanish
        reconnectBtn = button("Qayta ulanish") {
            if (AgentManager.isStarted && settings.server.isNotBlank()) Native.networkChanged() else AgentManager.start(settings)
        }
        root.addView(
            LinearLayout(this).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = android.view.Gravity.CENTER_VERTICAL
                addView(statusView, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
                addView(reconnectBtn)
            },
            lp(top = 2),
        )

        // --- ulanish kartasi
        idEdit = edit("9 xonali ID", settings.lastTarget, InputType.TYPE_CLASS_NUMBER)
        addrEdit = edit("Host manzili (IP yoki nom[:port])", settings.lastAddr, InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI)
        passEdit = edit("Kalit (agar kerak bo'lsa)", "", InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_PASSWORD)
        descLabel = label("", 12f, Colors.DIM)
        modeRelayBtn = button("Server orqali") { setMode(false) }
        modeP2pBtn = button("P2P") { setMode(true) }
        val modeRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            addView(modeRelayBtn, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f))
            addView(modeP2pBtn, LinearLayout.LayoutParams(0, ViewGroup.LayoutParams.WRAP_CONTENT, 1f).apply { leftMargin = dp(8) })
        }
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
                label("Ulanish", 16f, bold = true),
                modeRow,
                descLabel,
                idEdit,
                addrEdit,
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
        contactsCard = card(label("Kontaktlar", 16f, bold = true), contactsBox)
        root.addView(contactsCard, lp(top = 12))
        p2pBox = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        p2pCard = card(label("Saqlangan kompyuterlar", 16f, bold = true), p2pBox)
        root.addView(p2pCard, lp(top = 12))
        applyMode()

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
        reconnectBtn.visibility = if (AgentManager.online) View.GONE else View.VISIBLE
    }

    private fun setMode(p2p: Boolean) {
        p2pMode = p2p
        settings.p2p = p2p
        applyMode()
    }

    /** Server orqali (ID) yoki P2P (manzil) rejimiga mos maydonlar va ro'yxatlar. */
    private fun applyMode() {
        modeRelayBtn.background = rounded(if (p2pMode) Colors.BG else Colors.ACCENT, 8)
        modeP2pBtn.background = rounded(if (p2pMode) Colors.ACCENT else Colors.BG, 8)
        modeRelayBtn.setTextColor(if (p2pMode) Colors.TEXT else android.graphics.Color.WHITE)
        modeP2pBtn.setTextColor(if (p2pMode) android.graphics.Color.WHITE else Colors.TEXT)
        descLabel.text = if (p2pMode) {
            "Boshqa kompyuterning manzili va kalitini kiriting. Server shart emas."
        } else {
            "Boshqa kompyuterning ID sini kiriting. Ruxsat berilgach ulanasiz."
        }
        idEdit.visibility = if (p2pMode) View.GONE else View.VISIBLE
        addrEdit.visibility = if (p2pMode) View.VISIBLE else View.GONE
        passEdit.hint = if (p2pMode) "Kalit" else "Kalit (agar kerak bo'lsa)"
        contactsCard.visibility = if (p2pMode) View.GONE else View.VISIBLE
        p2pCard.visibility = if (p2pMode) View.VISIBLE else View.GONE
        refreshP2pHosts()
    }

    private fun refreshP2pHosts() {
        p2pBox.removeAllViews()
        val list = settings.p2pHosts
        if (list.isEmpty()) {
            p2pBox.addView(label("Hozircha yo'q. Muvaffaqiyatli ulanishdan keyin shu yerda paydo bo'ladi.", 12f, Colors.DIM))
            return
        }
        for (a in list) {
            val row = label(a, 15f)
            row.setPadding(0, dp(8), 0, dp(8))
            row.setOnClickListener { addrEdit.setText(a) }
            row.setOnLongClickListener {
                AlertDialog.Builder(this).setMessage("$a manzilini o'chirish?")
                    .setPositiveButton("O'chirish") { _, _ ->
                        settings.p2pHosts = settings.p2pHosts.filter { it != a }
                        refreshP2pHosts()
                    }.setNegativeButton("Bekor", null).show()
                true
            }
            p2pBox.addView(row)
        }
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

    private fun connectP2p() {
        val addr = addrEdit.text.toString().trim()
        if (addr.isEmpty()) {
            addrEdit.error = "Manzilni kiriting"
            return
        }
        if (passEdit.text.isEmpty()) {
            passEdit.error = "Kalitni kiriting"
            return
        }
        settings.fps = fps
        settings.kbps = kbps
        settings.height = height
        settings.lastAddr = addr
        startActivity(
            Intent(this, ViewerActivity::class.java)
                .putExtra("addr", addr)
                .putExtra("password", passEdit.text.toString())
                .putExtra("fps", fps)
                .putExtra("kbps", kbps)
                .putExtra("height", height),
        )
    }

    private fun connect() {
        if (p2pMode) return connectP2p()
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
