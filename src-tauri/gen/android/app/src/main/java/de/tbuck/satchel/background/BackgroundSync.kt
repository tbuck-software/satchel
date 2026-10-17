package de.tbuck.satchel.background

import android.app.Activity
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.IBinder
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import de.tbuck.satchel.MainActivity
import de.tbuck.satchel.R

class BootRescheduleReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    if (intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED) {
      runCatching { BackgroundMailSyncWorkScheduler.rescheduleIfEnabled(context) }
        .onFailure { Log.e("SatchelBackgroundSync", "Unable to reschedule durable mail sync", it) }
    }
  }
}

@TauriPlugin
class BackgroundSyncPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun start(invoke: Invoke) {
    try {
      require(setEnabled(activity, true)) { "Unable to persist background sync state" }
      BackgroundMailSyncWorkScheduler.schedulePeriodic(activity)
      startService(activity)
      invoke.resolve()
    } catch (error: Exception) {
      Log.e(TAG, "Unable to start background sync", error)
      invoke.reject("Unable to start background sync")
    }
  }

  @Command
  fun stop(invoke: Invoke) {
    try {
      require(setEnabled(activity, false)) { "Unable to persist background sync state" }
      activity.stopService(Intent(activity, SatchelSyncService::class.java))
      BackgroundMailSyncWorkScheduler.cancel(activity)
      invoke.resolve()
    } catch (error: Exception) {
      Log.e(TAG, "Unable to stop background sync", error)
      invoke.reject("Unable to stop background sync")
    }
  }

  @Command
  fun status(invoke: Invoke) {
    try {
      val enabled = activity.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
        .getBoolean(ENABLED, true)
      val powerManager = activity.getSystemService(PowerManager::class.java)
      invoke.resolve(JSObject().apply {
        put("enabled", enabled)
        put("running", SatchelSyncService.running)
        put("batteryOptimizationExempt", powerManager.isIgnoringBatteryOptimizations(activity.packageName))
      })
    } catch (error: Exception) {
      Log.e(TAG, "Unable to read background sync status", error)
      invoke.reject("Unable to read background sync status")
    }
  }

  @Command
  fun openSettings(invoke: Invoke) {
    try {
      activity.startActivity(
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS).apply {
          data = Uri.parse("package:${activity.packageName}")
        },
      )
      invoke.resolve()
    } catch (error: Exception) {
      Log.e(TAG, "Unable to open Android app settings", error)
      invoke.reject("Unable to open Android app settings")
    }
  }

  companion object {
    private const val TAG = "SatchelBackgroundSync"
    private const val PREFERENCES = "satchel.background-sync"
    private const val ENABLED = "enabled"

    fun isEnabled(context: Context): Boolean =
      context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
        .getBoolean(ENABLED, true)

    private fun setEnabled(context: Context, enabled: Boolean): Boolean {
      return context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
        .edit()
        .putBoolean(ENABLED, enabled)
        .commit()
    }

    private fun startService(context: Context) {
      ContextCompat.startForegroundService(context, Intent(context, SatchelSyncService::class.java))
    }

    /**
     * Start while MainActivity still has Android's foreground-service launch
     * allowance. Waiting for the WebView and a large mailbox to initialize can
     * move the activity out of that window before the frontend invokes start.
     */
    fun restoreIfEnabled(context: Context) {
      if (!isEnabled(context)) {
        BackgroundMailSyncWorkScheduler.cancel(context)
        return
      }
      BackgroundMailSyncWorkScheduler.schedulePeriodic(context)
      runCatching { startService(context) }
        .onFailure { Log.e(TAG, "Unable to restore background sync", it) }
    }
  }
}

class SatchelSyncService : Service() {
  override fun onCreate() {
    super.onCreate()
    running = true
    val manager = getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(
      NotificationChannel(
        CHANNEL_ID,
        NativeUiStrings.get(this, NativeUiStrings.BACKGROUND_SYNC_CHANNEL),
        NotificationManager.IMPORTANCE_LOW,
      ).apply {
        description = NativeUiStrings.get(this@SatchelSyncService, NativeUiStrings.BACKGROUND_SYNC_DESCRIPTION)
        setShowBadge(false)
      },
    )

    val openApp = PendingIntent.getActivity(
      this,
      0,
      Intent(this, MainActivity::class.java).apply {
        flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
      },
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    val notification = NotificationCompat.Builder(this, CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_notification)
      .setContentTitle("Satchel")
      .setContentText(NativeUiStrings.get(this, NativeUiStrings.BACKGROUND_SYNC_TEXT))
      .setContentIntent(openApp)
      .setCategory(NotificationCompat.CATEGORY_SERVICE)
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setSilent(true)
      .build()
    startForeground(NOTIFICATION_ID, notification)
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    Log.i(TAG, "Background sync keepalive started")
    return START_NOT_STICKY
  }

  override fun onTaskRemoved(rootIntent: Intent?) {
    Log.i(TAG, "Recent-apps task removed; keeping background sync alive")
    super.onTaskRemoved(rootIntent)
  }

  override fun onDestroy() {
    running = false
    Log.w(TAG, "Background sync keepalive stopped")
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null

  companion object {
    @Volatile var running = false
      private set

    private const val TAG = "SatchelBackgroundSync"
    private const val CHANNEL_ID = "satchel.background-sync"
    private const val NOTIFICATION_ID = 0x5A7C_51AC
  }
}
