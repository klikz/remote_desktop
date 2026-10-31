package com.remotedesktop.mobile

import android.content.Context
import android.view.Gravity
import android.view.MotionEvent
import android.view.SurfaceView
import android.view.ViewConfiguration
import android.widget.FrameLayout
import kotlin.math.abs
import kotlin.math.hypot
import kotlin.math.max
import kotlin.math.min

/**
 * Video ko'rinishi va barcha teginish mantiqi.
 *
 * Rejimlar (`touchpad`):
 *  - to'g'ridan-to'g'ri: tap = shu nuqtada chap klik, uzoq bosish = o'ng klik, surish = chap tugma bosilgan holda
 *    kursor barmoq ostida ketadi, ikki barmoq surish = scroll.
 *  - touchpad: surish kursorni nisbiy suradi, tap = chap klik, ikki barmoq tap = o'ng klik,
 *    ikki barmoq surish = scroll, tap-tap-surish = drag.
 * Ikki barmoq masofasi o'zgarsa — chimchilash zoom (ko'rinish kattalashadi, koordinatalar moslanadi).
 */
class VideoContainer(context: Context) : FrameLayout(context) {
    val surface = SurfaceView(context)
    var touchpad = false

    private var videoW = 0
    private var videoH = 0
    private var zoom = 1f
    private var tx = 0f
    private var ty = 0f

    // touchpad kursori (video kasri)
    private var curX = 0.5f
    private var curY = 0.5f

    // Barmoq qo'yilganda tabiiy titrash bo'ladi: bosish/uzoq bosish uchun slop ikki baravar bo'shashtirilgan
    private val slop = ViewConfiguration.get(context).scaledTouchSlop.toFloat() * 2f
    private val scrollStepPx = context.dp(28).toFloat()
    private val longPressMs = 500L
    private val tapMs = 350L
    private val sensitivity = 1.6f

    private enum class G { NONE, SINGLE, MULTI, IGNORE }
    private enum class M { UNDECIDED, SCROLL, ZOOM }

    private var g = G.NONE
    private var downX = 0f
    private var downY = 0f
    private var lastX = 0f
    private var lastY = 0f
    private var downTime = 0L
    private var moved = false
    private var pressed = false
    private var longFired = false
    private var dragCandidate = false
    private var lastTapUp = 0L

    private var multi = M.UNDECIDED
    private var multiStart = 0L
    private var multiMoved = false
    private var startSpan = 0f
    private var startMidX = 0f
    private var startMidY = 0f
    private var lastSpan = 0f
    private var lastMidX = 0f
    private var lastMidY = 0f
    private var scrollAccX = 0f
    private var scrollAccY = 0f

    // Uzoq bosish = o'ng klik (ikkala rejimda; sensor rejimida bosilgan nuqtada, touchpadda kursor ostida)
    private val longPress = Runnable {
        if (g == G.SINGLE && !moved) {
            longFired = true
            performHapticFeedback(android.view.HapticFeedbackConstants.LONG_PRESS)
            if (touchpad) sendFrac(curX, curY) else moveTo(downX, downY)
            click(1)
        }
    }

    init {
        setBackgroundColor(0xFF000000.toInt())
        addView(surface, LayoutParams(1, 1, Gravity.CENTER))
    }

    fun setVideoSize(w: Int, h: Int) {
        videoW = w
        videoH = h
        fit()
    }

    fun resetZoom() {
        zoom = 1f
        tx = 0f
        ty = 0f
        applyTransform()
    }

    override fun onSizeChanged(w: Int, h: Int, ow: Int, oh: Int) {
        super.onSizeChanged(w, h, ow, oh)
        fit()
    }

    private fun fit() {
        if (videoW <= 0 || videoH <= 0 || width <= 0 || height <= 0) return
        val s = min(width.toFloat() / videoW, height.toFloat() / videoH)
        val lp = surface.layoutParams as LayoutParams
        lp.width = (videoW * s).toInt().coerceAtLeast(1)
        lp.height = (videoH * s).toInt().coerceAtLeast(1)
        surface.layoutParams = lp
        post { applyTransform() }
    }

    private fun applyTransform() {
        val sw = surface.width.toFloat() * zoom
        val sh = surface.height.toFloat() * zoom
        val maxTx = max(0f, (sw - width) / 2)
        val maxTy = max(0f, (sh - height) / 2)
        tx = tx.coerceIn(-maxTx, maxTx)
        ty = ty.coerceIn(-maxTy, maxTy)
        surface.scaleX = zoom
        surface.scaleY = zoom
        surface.translationX = tx
        surface.translationY = ty
    }

    /** Konteyner koordinatasi -> video kasri (0..1). */
    private fun frac(px: Float, py: Float): Pair<Float, Float> {
        val sw = surface.width.toFloat().coerceAtLeast(1f)
        val sh = surface.height.toFloat().coerceAtLeast(1f)
        val cx = surface.left + sw / 2 + tx
        val cy = surface.top + sh / 2 + ty
        val lx = (px - cx) / zoom + sw / 2
        val ly = (py - cy) / zoom + sh / 2
        return (lx / sw).coerceIn(0f, 1f) to (ly / sh).coerceIn(0f, 1f)
    }

    private fun sendFrac(fx: Float, fy: Float) {
        Native.sendMove((fx * 65535).toInt(), (fy * 65535).toInt())
    }

    private fun moveTo(px: Float, py: Float) {
        val (fx, fy) = frac(px, py)
        curX = fx
        curY = fy
        sendFrac(fx, fy)
    }

    private fun click(button: Int) {
        Native.sendButton(button, true)
        Native.sendButton(button, false)
    }

    private fun releaseButton() {
        if (pressed) {
            Native.sendButton(0, false)
            pressed = false
        }
    }

    override fun onTouchEvent(e: MotionEvent): Boolean {
        when (e.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                g = G.SINGLE
                downX = e.x; downY = e.y; lastX = e.x; lastY = e.y
                downTime = e.eventTime
                moved = false; longFired = false; pressed = false
                dragCandidate = touchpad && e.eventTime - lastTapUp < tapMs
                removeCallbacks(longPress)
                postDelayed(longPress, longPressMs)
            }
            MotionEvent.ACTION_POINTER_DOWN -> {
                if (g == G.SINGLE && e.pointerCount == 2) {
                    removeCallbacks(longPress)
                    releaseButton()
                    startMulti(e)
                } else if (g != G.MULTI) {
                    g = G.IGNORE
                }
            }
            MotionEvent.ACTION_MOVE -> when (g) {
                G.SINGLE -> moveSingle(e)
                G.MULTI -> moveMulti(e)
                else -> {}
            }
            MotionEvent.ACTION_POINTER_UP -> {
                if (g == G.MULTI && e.pointerCount == 2) {
                    // ikki barmoq tap (touchpad): o'ng klik
                    if (multi == M.UNDECIDED && !multiMoved && e.eventTime - multiStart < tapMs) {
                        if (touchpad) sendFrac(curX, curY) else moveTo(midX(e), midY(e))
                        click(1)
                    }
                    g = G.IGNORE
                }
            }
            MotionEvent.ACTION_UP -> {
                removeCallbacks(longPress)
                if (g == G.SINGLE) endSingle(e)
                releaseButton()
                g = G.NONE
            }
            MotionEvent.ACTION_CANCEL -> {
                removeCallbacks(longPress)
                releaseButton()
                g = G.NONE
            }
        }
        return true
    }

    private fun moveSingle(e: MotionEvent) {
        val x = e.x
        val y = e.y
        if (!moved) {
            if (hypot(x - downX, y - downY) <= slop) return
            moved = true
            removeCallbacks(longPress)
            if (!touchpad) {
                moveTo(x, y)
                Native.sendButton(0, true)
                pressed = true
                lastX = x; lastY = y
                return
            }
            if (dragCandidate) {
                Native.sendButton(0, true)
                pressed = true
            }
            lastX = downX; lastY = downY
        }
        if (touchpad) {
            val sw = surface.width * zoom
            val sh = surface.height * zoom
            curX = (curX + (x - lastX) / sw * sensitivity).coerceIn(0f, 1f)
            curY = (curY + (y - lastY) / sh * sensitivity).coerceIn(0f, 1f)
            sendFrac(curX, curY)
        } else {
            moveTo(x, y)
        }
        lastX = x; lastY = y
    }

    private fun endSingle(e: MotionEvent) {
        if (moved || longFired) return
        // tap
        if (touchpad) sendFrac(curX, curY) else moveTo(e.x, e.y)
        click(0)
        lastTapUp = e.eventTime
    }

    private fun startMulti(e: MotionEvent) {
        g = G.MULTI
        multi = M.UNDECIDED
        multiMoved = false
        multiStart = e.eventTime
        lastSpan = span(e)
        startSpan = lastSpan
        lastMidX = midX(e); lastMidY = midY(e)
        startMidX = lastMidX; startMidY = lastMidY
        scrollAccX = 0f; scrollAccY = 0f
    }

    private fun moveMulti(e: MotionEvent) {
        if (e.pointerCount < 2) return
        val sp = span(e)
        val mx = midX(e)
        val my = midY(e)
        if (multi == M.UNDECIDED) {
            if (abs(sp - startSpan) > slop * 1.5f) {
                multi = M.ZOOM
                multiMoved = true
            } else if (hypot(mx - startMidX, my - startMidY) > slop) {
                multi = M.SCROLL
                multiMoved = true
            }
        }
        when (multi) {
            M.ZOOM -> {
                if (lastSpan > 0f) zoom = (zoom * sp / lastSpan).coerceIn(1f, 6f)
                tx += mx - lastMidX
                ty += my - lastMidY
                applyTransform()
            }
            M.SCROLL -> {
                scrollAccX += mx - lastMidX
                scrollAccY += my - lastMidY
                // barmoq yuqoriga = kontent pastga (tabiiy scroll); protokolda musbat = pastga/o'ngga
                while (abs(scrollAccY) >= scrollStepPx) {
                    Native.sendScroll(0, if (scrollAccY < 0) 1 else -1)
                    scrollAccY -= if (scrollAccY < 0) -scrollStepPx else scrollStepPx
                }
                while (abs(scrollAccX) >= scrollStepPx) {
                    Native.sendScroll(if (scrollAccX < 0) 1 else -1, 0)
                    scrollAccX -= if (scrollAccX < 0) -scrollStepPx else scrollStepPx
                }
            }
            M.UNDECIDED -> {}
        }
        lastSpan = sp
        lastMidX = mx
        lastMidY = my
    }

    private fun span(e: MotionEvent) = hypot(e.getX(0) - e.getX(1), e.getY(0) - e.getY(1))
    private fun midX(e: MotionEvent) = (e.getX(0) + e.getX(1)) / 2
    private fun midY(e: MotionEvent) = (e.getY(0) + e.getY(1)) / 2
}
