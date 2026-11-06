package com.remotedesktop.mobile

/** Rust yadrosi (`mobile_viewer/core`, `librdmobile.so`) bilan JNI ko'prik. */
object Native {
    init {
        System.loadLibrary("rdmobile")
    }

    external fun init(filesDir: String)

    // --- relay agenti ---
    external fun startAgent(server: String, key: String, name: String, id: String, token: String): Boolean
    external fun stopAgent()
    external fun subscribe(ids: String)

    /** `status\t0|1\ttext`, `registered\tid\ttoken`, `presence\tid\t0|1\tname` yoki null. */
    external fun agentEvent(): String?

    // --- sessiya ---
    external fun connect(id: String, password: String, fps: Int, kbps: Int, height: Int): Boolean

    /** P2P (to'g'ridan-to'g'ri) sessiya: `addr` = `IP`, `IP:port` yoki `nom[:port]`. Agent shart emas. */
    external fun connectDirect(addr: String, password: String, fps: Int, kbps: Int, height: Int): Boolean

    /** `confirm\tfp`, `connected\tfp\t0|1`, `video`, `reconnecting\tattempt\treason`, `ended\terror` yoki null. */
    external fun pollEvent(): String?

    /** Qurilma tarmog'i o'zgardi: agent va ochiq sessiya darrov qayta ulanadi. */
    external fun networkChanged()
    external fun confirm(accept: Boolean)

    /**
     * Keyingi kadrni kutadi: `-1` = vaqt tugadi, aks holda `(baytlar shl 1) or kalit_kadr`.
     * Kadr [copyFrame] yoki [discardFrame] chaqirilguncha Rust tomonda saqlanadi.
     */
    external fun waitFrame(timeoutMs: Int): Long

    /** Kutilgan kadrni MediaCodec kirish buferiga (direct) ko'chiradi. `-1`: sig'madi, kadr tashlandi. */
    external fun copyFrame(buf: java.nio.ByteBuffer): Int

    /** Kutilgan kadrni tashlaydi. `chainBroken`: dekoder kadrni ololmadi, Host'dan kalit kadr so'raladi. */
    external fun discardFrame(chainBroken: Boolean)

    /** Dekoder xatosi yoki qayta yaratildi: Host'dan kalit kadr so'raladi. */
    external fun requestKeyframe()

    /** `[kadrlar, baytlar, rtt_us]` */
    external fun stats(): LongArray?
    external fun close()

    // --- kiritish ---
    external fun sendMove(x: Int, y: Int)
    external fun sendButton(button: Int, pressed: Boolean)
    external fun sendScroll(dx: Int, dy: Int)
    external fun sendKey(code: Int, pressed: Boolean)
}
