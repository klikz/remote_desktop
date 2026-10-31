package com.remotedesktop.mobile

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Build
import android.view.Surface

/**
 * Rust'dan kelgan Annex-B H.264 kadrlarni MediaCodec bilan dekodlab, to'g'ridan-to'g'ri Surface'ga chiqaradi.
 * Alohida oqimda ishlaydi; kalit-kadrgacha kelgan kadrlar tashlanadi (SPS/PPS kalit-kadr ichida keladi).
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
        var pts = 0L
        try {
            while (running) {
                val frame = Native.nextFrame(20)
                if (frame != null && frame.size > 1) {
                    val key = frame[0].toInt() == 1
                    if (codec == null && key) codec = createCodec()
                    if (codec != null) {
                        val idx = codec.dequeueInputBuffer(30_000)
                        if (idx >= 0) {
                            val buf = codec.getInputBuffer(idx)!!
                            val len = minOf(frame.size - 1, buf.capacity())
                            buf.clear()
                            buf.put(frame, 1, len)
                            pts += 16_666
                            codec.queueInputBuffer(idx, 0, len, pts, if (key) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
                        }
                    }
                }
                if (codec != null) drain(codec, info)
            }
        } catch (e: Exception) {
            android.util.Log.e("rd", "dekoder xatosi", e)
        } finally {
            try {
                codec?.stop()
            } catch (_: Exception) {
            }
            codec?.release()
        }
    }

    private fun createCodec(): MediaCodec {
        // Boshlang'ich o'lcham taxminiy: haqiqiysi SPS dan keladi (adaptive playback, INFO_OUTPUT_FORMAT_CHANGED).
        val fmt = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_AVC, 1920, 1080).apply {
            setInteger(MediaFormat.KEY_MAX_WIDTH, 4096)
            setInteger(MediaFormat.KEY_MAX_HEIGHT, 4096)
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
