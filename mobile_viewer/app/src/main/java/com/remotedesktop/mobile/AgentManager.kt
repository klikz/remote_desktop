package com.remotedesktop.mobile

import android.os.Handler
import android.os.Looper

/** Relay agenti holati: asosiy oqimda har 250 ms da Rust agenti hodisalarini o'qiydi. */
object AgentManager {
    var online = false
        private set
    var status = "oflayn"
        private set
    val presence = HashMap<String, Pair<Boolean, String>>()
    var listener: (() -> Unit)? = null

    private val handler = Handler(Looper.getMainLooper())
    private var running = false
    private lateinit var settings: Settings

    private val tick = object : Runnable {
        override fun run() {
            if (!running) return
            var changed = false
            while (true) {
                val ev = Native.agentEvent() ?: break
                val f = ev.split('\t')
                when (f[0]) {
                    "status" -> {
                        online = f.getOrElse(1) { "0" } == "1"
                        status = f.getOrElse(2) { "" }
                    }
                    "registered" -> {
                        settings.id = f.getOrElse(1) { "" }
                        settings.token = f.getOrElse(2) { "" }
                    }
                    "presence" -> presence[f.getOrElse(1) { "" }] = (f.getOrElse(2) { "0" } == "1") to f.getOrElse(3) { "" }
                }
                changed = true
            }
            if (changed) listener?.invoke()
            handler.postDelayed(this, 250)
        }
    }

    fun start(s: Settings) {
        settings = s
        online = false
        presence.clear()
        if (s.server.isBlank()) {
            status = "server manzilini kiriting"
            listener?.invoke()
            return
        }
        Native.startAgent(s.serverAddr(), s.key, s.name, s.id, s.token)
        status = "ulanilmoqda..."
        subscribe()
        if (!running) {
            running = true
            handler.post(tick)
        }
        listener?.invoke()
    }

    fun subscribe() {
        if (!::settings.isInitialized) return
        Native.subscribe(settings.contacts.joinToString(",") { it.id })
    }

    val isStarted get() = running
}
