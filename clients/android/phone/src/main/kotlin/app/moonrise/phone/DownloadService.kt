package app.moonrise.phone

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import app.moonrise.core.DownloadState
import app.moonrise.core.bytesText
import app.moonrise.shared.Log
import app.moonrise.shared.Session
import app.moonrise.shared.downloads.Downloads
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Keeps the app running while something downloads (the iPhone's Live Activity and continued
 * processing): a foreground service with the download's progress in a notification.
 */
class DownloadService : Service() {
    private var ticker: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val notification = build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(ID, notification)
        }
        if (ticker?.isActive != true) ticker = Session.ui.launch {
            while (isActive && Downloads.active.isNotEmpty()) {
                manager().notify(ID, build())
                delay(2000)
            }
            stopSelf()
        }
        return START_NOT_STICKY
    }

    // Android 15: a data-sync service runs at most 6 hours a day; downloads go on while the app is open.
    override fun onTimeout(startId: Int, fgsType: Int) { stopSelf() }

    override fun onDestroy() {
        ticker?.cancel()
        super.onDestroy()
    }

    private fun manager() = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager

    private fun build(): Notification {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager().createNotificationChannel(NotificationChannel(CHANNEL, "Downloads", NotificationManager.IMPORTANCE_LOW))
        }
        val r = Downloads.records.firstOrNull { it.state == DownloadState.Downloading } ?: Downloads.active.firstOrNull()
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val b = Notification.Builder(this, CHANNEL).setSmallIcon(android.R.drawable.stat_sys_download).setOngoing(true).setOnlyAlertOnce(true).setContentIntent(open)
            .setContentTitle(r?.title ?: "Downloading")
        if (r != null) {
            b.setProgress(100, (r.progress * 100).toInt(), r.progress <= 0)
            b.setContentText(if (r.bytesExpected > 0) "${bytesText(r.bytesWritten)} of ${bytesText(r.bytesExpected)}" else r.qualityLabel)
        }
        return b.build()
    }

    companion object {
        private const val ID = 7301
        private const val CHANNEL = "downloads"

        /** A download started or the last one ended: the service runs or goes. */
        fun sync(context: Context) {
            if (Downloads.active.isEmpty()) return // it stops itself on its next tick
            runCatching { context.startForegroundService(Intent(context, DownloadService::class.java)) }
                .onFailure { Log.w("MoonriseDownload", "foreground service: ${it.message}") }
        }
    }
}
