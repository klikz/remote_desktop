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

    /** `confirm\tfp`, `connected\tfp\t0|1`, `video`, `ended\terror` yoki null. */
    external fun pollEvent(): String?
    external fun confirm(accept: Boolean)

    /** Birinchi bayt 1 = kalit-kadr, qolgani Annex-B H.264. Vaqt tugasa null. */
    external fun nextFrame(timeoutMs: Int): ByteArray?

    /** `[kadrlar, baytlar, rtt_us]` */
    external fun stats(): LongArray?
    external fun close()

    // --- kiritish ---
    external fun sendMove(x: Int, y: Int)
    external fun sendButton(button: Int, pressed: Boolean)
    external fun sendScroll(dx: Int, dy: Int)
    external fun sendKey(code: Int, pressed: Boolean)
}
