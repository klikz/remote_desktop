package com.remotedesktop.mobile

import android.content.Context
import android.content.SharedPreferences

/** Sifat shabloni: (nom, fps, kbps, balandlik; 0 = asl o'lcham). Desktop viewer'dagi PRESETS bilan bir xil. */
data class Preset(val name: String, val fps: Int, val kbps: Int, val height: Int)

val PRESETS = listOf(
    Preset("Past · 720p/15", 15, 2500, 720),
    Preset("O'rta · 1080p/30", 30, 8000, 1080),
    Preset("Yuqori · 1080p/30", 30, 15000, 1080),
    Preset("Matn · 1440p/20", 20, 20_000, 1440),
    Preset("Maksimal · asl", 30, 40_000, 0),
)

val HEIGHTS = listOf(720 to "720p", 1080 to "1080p", 1440 to "1440p", 2160 to "2160p", 0 to "Asl o'lcham")

data class Contact(val id: String, val name: String)

class Settings(ctx: Context) {
    private val p: SharedPreferences = ctx.getSharedPreferences("rd", Context.MODE_PRIVATE)

    var server: String
        get() = p.getString("server", "") ?: ""
        set(v) = p.edit().putString("server", v.trim()).apply()
    var key: String
        get() = p.getString("key", "") ?: ""
        set(v) = p.edit().putString("key", v.trim()).apply()
    var name: String
        get() = p.getString("name", null) ?: android.os.Build.MODEL
        set(v) = p.edit().putString("name", v.trim()).apply()
    var id: String
        get() = p.getString("id", "") ?: ""
        set(v) = p.edit().putString("id", v).apply()
    var token: String
        get() = p.getString("token", "") ?: ""
        set(v) = p.edit().putString("token", v).apply()

    var fps: Int
        get() = p.getInt("fps", PRESETS[1].fps)
        set(v) = p.edit().putInt("fps", v).apply()
    var kbps: Int
        get() = p.getInt("kbps", PRESETS[1].kbps)
        set(v) = p.edit().putInt("kbps", v).apply()
    var height: Int
        get() = p.getInt("height", PRESETS[1].height)
        set(v) = p.edit().putInt("height", v).apply()

    /** true = touchpad, false = to'g'ridan-to'g'ri (sensor). */
    var touchpad: Boolean
        get() = p.getBoolean("touchpad", false)
        set(v) = p.edit().putBoolean("touchpad", v).apply()

    var lastTarget: String
        get() = p.getString("lastTarget", "") ?: ""
        set(v) = p.edit().putString("lastTarget", v).apply()

    /** `host` yoki `host:port` -> `host:port` (relay standart porti 7780). */
    fun serverAddr(): String {
        val s = server.trim()
        val port = s.substringAfterLast(':', "").toIntOrNull()
        return if (s.contains(':') && port != null) s else "$s:7780"
    }

    var contacts: List<Contact>
        get() = (p.getString("contacts", "") ?: "").lines().mapNotNull {
            val parts = it.split('\t')
            if (parts[0].length == 9 && parts[0].all(Char::isDigit)) Contact(parts[0], parts.getOrElse(1) { "" }) else null
        }
        set(v) = p.edit().putString("contacts", v.joinToString("\n") { "${it.id}\t${it.name.replace('\t', ' ').replace('\n', ' ')}" }).apply()

    fun addContact(id: String, name: String = "") {
        val cur = contacts
        if (cur.any { it.id == id }) return
        contacts = cur + Contact(id, name)
    }
}
