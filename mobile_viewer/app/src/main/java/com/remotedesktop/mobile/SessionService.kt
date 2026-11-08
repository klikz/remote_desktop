package com.remotedesktop.mobile

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager

/**
 * Ochiq sessiya davomida ishlaydi: foreground service (Android ilovani fonda o'ldirmasin), CPU va Wi-Fi qulflari
 * (ekran o'chganda tarmoq uxlab qolmasin) hamda tarmoq o'zgarishini kuzatuvchi (Wi-Fi <-> mobil internet:
 * ulanish darrov qayta o'rnatiladi, eski soket kutib o'tirilmaydi).
 */
class SessionService : Service() {
    private var wake: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null
    private var cm: ConnectivityManager? = null
    private var callback: ConnectivityManager.NetworkCallback? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = buildNotification()
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        acquireLocks()
        watchNetwork()
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        callback?.let { runCatching { cm?.unregisterNetworkCallback(it) } }
        callback = null
        wake?.let { if (it.isHeld) it.release() }
        wifi?.let { if (it.isHeld) it.release() }
        wake = null
        wifi = null
        super.onDestroy()
    }

    private fun buildNotification(): Notification {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "Faol ulanish", NotificationManager.IMPORTANCE_LOW))
        val open = packageManager.getLaunchIntentForPackage(packageName)
        val pending = open?.let { PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE) }
        return Notification.Builder(this, CHANNEL)
            .setContentTitle("Ulan")
            .setContentText("Kompyuter bilan ulanish faol")
            .setSmallIcon(android.R.drawable.ic_menu_view)
            .setOngoing(true)
            .setContentIntent(pending)
            .build()
    }

    private fun acquireLocks() {
        if (wake == null) {
            val pm = applicationContext.getSystemService(Context.POWER_SERVICE) as PowerManager
            wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "ulan:session").apply {
                setReferenceCounted(false)
                acquire()
            }
        }
        if (wifi == null) {
            val wm = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF
            wifi = wm.createWifiLock(mode, "ulan:session").apply {
                setReferenceCounted(false)
                acquire()
            }
        }
    }

    private fun watchNetwork() {
        if (callback != null) return
        val manager = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        val cb = object : ConnectivityManager.NetworkCallback() {
            private var current: Network? = null
            private var seen = false

            override fun onAvailable(network: Network) {
                val prev = current
                current = network
                // Birinchi chaqiruv — hozirgi holat; keyingilari: boshqa tarmoqqa o'tildi yoki uzilgandan keyin qaytdi.
                if (seen && prev != network) Native.networkChanged()
                seen = true
            }

            override fun onLost(network: Network) {
                if (current == network) current = null
            }
        }
        callback = cb
        cm = manager
        runCatching { manager.registerDefaultNetworkCallback(cb) }
    }

    companion object {
        private const val CHANNEL = "session"
        private const val NOTIFICATION_ID = 1

        fun start(context: Context) {
            runCatching { context.startForegroundService(Intent(context, SessionService::class.java)) }
        }

        fun stop(context: Context) {
            context.stopService(Intent(context, SessionService::class.java))
        }
    }
}
