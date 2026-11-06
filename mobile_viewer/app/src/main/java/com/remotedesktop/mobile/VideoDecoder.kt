package com.remotedesktop.mobile

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.view.Surface

/**
 * Rust'dan kelgan Annex-B H.264 kadrlarni MediaCodec bilan dekodlab, to'g'ridan-to'g'ri Surface'ga chiqaradi.
 * Alohida oqimda ishlaydi. Kadr Rust'dan dekoderning kirish buferiga bir marta ko'chiriladi (Java massivi yaratilmaydi).
 *
 * Host kalit kadrni faqat so'ralganda yuboradi, shuning uchun zanjir uzilsa (kirish buferi yo'q, kadr sig'madi,
 * dekoder xatosi) keyingi kalit kadrgacha kadrlar tashlanadi va Host'dan kalit kadr so'raladi. Dekoder xatosida
 * kodek qayta yaratiladi: video qotib qolmaydi.
 */
class VideoDecoder(private val surface: Surface, private val onSize: (Int, Int) -> Unit) {
    @Volatile private var running = true
    private val thread = Thread(::loop, "video-decoder")

    fun start() = thread.start()

    fun stop() {
        running = false
        thread.join(1500)
    }

    private fun loop() {
        var codec: MediaCodec? = null
        val info = MediaCodec.BufferInfo()
        // Kodek yangi yoki zanjir uzilgan: kalit kadr kutilmoqda
        var needKey = true
        while (running) {
            try {
                val meta = Native.waitFrame(20)
                if (meta >= 0) {
                    val key = (meta and 1L) == 1L
                    if (needKey && !key) {
                        // Kalit kadr allaqachon so'ralgan: qayta so'ramaymiz
                        Native.discardFrame(false)
                    } else {
                        val c = codec ?: createCodec().also { codec = it }
                        val idx = c.dequeueInputBuffer(30_000)
                        if (idx < 0) {
                            Native.discardFrame(true)
                            needKey = true
                        } else {
                            val buf = c.getInputBuffer(idx)!!
                            buf.clear()
                            val n = Native.copyFrame(buf)
                            if (n < 0) {
                                // Sig'madi: kadr tashlandi va kalit kadr so'raldi; bufer bo'sh holda qaytariladi
                                c.queueInputBuffer(idx, 0, 0, 0, 0)
                                needKey = true
                            } else {
                                val flags = if (key) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0
                                c.queueInputBuffer(idx, 0, n, System.nanoTime() / 1000, flags)
                                if (key) needKey = false
                            }
                        }
                    }
                }
                codec?.let { drain(it, info) }
            } catch (e: Exception) {
                android.util.Log.e("rd", "dekoder xatosi: kodek qayta yaratiladi", e)
                release(codec)
                codec = null
                needKey = true
                Native.requestKeyframe()
                try {
                    Thread.sleep(50)
                } catch (_: InterruptedException) {
                }
            }
        }
        release(codec)
    }

    private fun release(codec: MediaCodec?) {
        if (codec == null) return
        try {
            codec.stop()
        } catch (_: Exception) {
        }
        try {
            codec.release()
        } catch (_: Exception) {
        }
    }

    private fun createCodec(): MediaCodec {
        // Boshlang'ich o'lcham taxminiy: haqiqiysi SPS dan keladi (adaptive playback, INFO_OUTPUT_FORMAT_CHANGED).
        val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, 1920, 1080).apply {
            setInteger(MediaFormat.KEY_MAX_WIDTH, 4096)
            setInteger(MediaFormat.KEY_MAX_HEIGHT, 4096)
            // Katta kalit kadrlar sig'ishi uchun (4K matnli ekran IDR'i 1 MB dan oshishi mumkin)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 4 * 1024 * 1024)
            if (Build.VERSION.SDK_INT >= 30) setInteger(MediaFormat.KEY_LOW_LATENCY, 1)
        }
        return MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_AVC).also {
            it.configure(fmt, surface, null, 0)
            it.start()
        }
    }

    private fun drain(codec: MediaCodec, info: MediaCodec.BufferInfo) {
        while (true) {
            val out = codec.dequeueOutputBuffer(info, 0)
            when {
                out >= 0 -> codec.releaseOutputBuffer(out, true)
                out == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    val f = codec.outputFormat
                    val crop = f.containsKey("crop-right") && f.containsKey("crop-left")
                    val w = if (crop) f.getInteger("crop-right") - f.getInteger("crop-left") + 1 else f.getInteger(MediaFormat.KEY_WIDTH)
                    val h = if (f.containsKey("crop-bottom")) f.getInteger("crop-bottom") - f.getInteger("crop-top") + 1 else f.getInteger(MediaFormat.KEY_HEIGHT)
                    onSize(w, h)
                }
                else -> return
            }
        }
    }
}
