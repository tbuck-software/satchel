package de.tbuck.satchel.background

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import de.tbuck.satchel.MainActivity
import de.tbuck.satchel.R
import org.json.JSONObject

/**
 * Localized strings persisted by Rust's `set_ui_strings` command.
 *
 * FCM, WorkManager and the foreground sync service may run in a headless
 * process with no WebView. Generated catalogue defaults follow the system
 * locale before the frontend has persisted the user's preference.
 */
object NativeUiStrings {
  const val ARCHIVE = "notification_archive"
  const val READ = "notification_read"
  const val CHANNEL_NAME = "notification_channel_name"
  const val CHANNEL_DESCRIPTION = "notification_channel_description"
  const val NEW_MESSAGE = "notification_new_message"
  const val SYNCING = "notification_syncing"
  const val REPLY = "notification_reply"
  const val SUMMARY = "notification_summary"
  const val BACKGROUND_SYNC_CHANNEL = "notification_background_sync_channel"
  const val BACKGROUND_SYNC_DESCRIPTION = "notification_background_sync_description"
  const val BACKGROUND_SYNC_TEXT = "notification_background_sync_text"
  const val ACTION_FAILED_TITLE = "notification_action_failed_title"
  const val ACTION_NOT_APPLIED_MESSAGE = "notification_action_not_applied_message"

  private const val PREFERENCES = "satchel.ui-strings"
  private const val STRINGS = "strings"
  private const val FOLLOW_SYSTEM_LOCALE = "follow_system_locale"

  private val defaults = NativeUiDefaults.forSystemLocale()

  fun get(context: Context, key: String): String {
    val fallback = defaults[key] ?: key
    val json = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
      .getString(STRINGS, null)
      ?: return fallback
    return runCatching {
      val values = JSONObject(json)
      if (values.optBoolean(FOLLOW_SYSTEM_LOCALE, false)) fallback
      else values.optString(key, fallback)
    }
      .getOrDefault(fallback)
      .ifBlank { fallback }
  }

  fun format(context: Context, key: String, parameter: String, value: String): String =
    get(context, key).replace("{$parameter}", value)
}

object SatchelNotifications {
  /** Historical id; renaming would leave the user's channel settings on an orphan. */
  const val CHANNEL_ID = "relay_mail"
  /** Stacks a morning's mail instead of listing every message separately. */
  const val GROUP_KEY = "satchel.mail.incoming"
  const val EXTRA_THREAD_ID = "satchel.threadId"
  const val EXTRA_EVENT_ID = "satchel.eventId"
  const val EXTRA_ACTION_ID = "satchel.actionId"
  const val EXTRA_ACCOUNT_ID = "satchel.accountId"
  const val EXTRA_ACCOUNT_LABEL = "satchel.accountLabel"
  const val EXTRA_SENDER = "satchel.sender"
  const val EXTRA_SUBJECT = "satchel.subject"
  const val ACTION_ARCHIVE = "satchel.archive"
  const val ACTION_READ = "satchel.read"
  const val ACTION_REPLY = "satchel.reply"

  /**
   * Why a mail was not put on screen. Matches `reason::PERMISSION_DENIED` in
   * `commands/notifications.rs`, which is also what macOS reports for the same
   * refusal, so the Diagnose page has one code for it rather than two.
   */
  const val REASON_PERMISSION_DENIED = "permission_denied"

  fun ensureChannel(context: Context) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      context.getSystemService(NotificationManager::class.java).createNotificationChannel(
        NotificationChannel(
          CHANNEL_ID,
          NativeUiStrings.get(context, NativeUiStrings.CHANNEL_NAME),
          NotificationManager.IMPORTANCE_DEFAULT,
        ).apply {
          description = NativeUiStrings.get(context, NativeUiStrings.CHANNEL_DESCRIPTION)
          enableVibration(true)
        },
      )
    }
  }

  /**
   * Hands one mail to the notification manager.
   *
   * Returns `null` when it was posted, or a reason code when Android never
   * showed it. A silent `Unit` return made the caller record a delivery for a
   * notification the user could not have seen.
   */
  fun showMail(
    context: Context,
    eventId: String,
    threadId: String?,
    accountId: String,
    accountLabel: String,
    sender: String,
    subject: String,
    preview: String,
    sound: Boolean,
  ): String? {
    ensureChannel(context)
    if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return REASON_PERMISSION_DENIED
    val safeAccountId = accountId.trim().take(512).ifEmpty { "default" }
    val safeAccountLabel = accountLabel.trim().take(128).ifEmpty { "Satchel" }
    val safeSender = sender.trim().take(128).ifEmpty { "Satchel" }
    val safeSubject = subject.trim().take(512).ifEmpty {
      NativeUiStrings.get(context, NativeUiStrings.NEW_MESSAGE)
    }
    val safePreview = preview.trim().take(1024)
    val expandedText = listOf(safeSubject, safePreview).filter(String::isNotEmpty).joinToString("\n")
    val groupKey = groupKey(safeAccountId)
    val intent = Intent(context, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
      data = notificationUri("open", eventId)
      putExtra(EXTRA_EVENT_ID, eventId)
      threadId?.takeIf { it.isNotBlank() && it.length <= 512 }?.let { putExtra(EXTRA_THREAD_ID, it) }
    }
    val id = eventId.hashCode() and Int.MAX_VALUE
    val pendingIntent = PendingIntent.getActivity(context, id, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
    val notification = NotificationCompat.Builder(context, CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_notification)
      .setContentTitle(safeSender)
      .setContentText(safeSubject)
      .setSubText(safeAccountLabel)
      .setStyle(
        NotificationCompat.BigTextStyle()
          .setBigContentTitle(safeSender)
          .bigText(expandedText),
      )
      .setContentIntent(pendingIntent)
      .setAutoCancel(true)
      // Delivery acknowledgement and the Android notification manager are
      // separate durable systems. Replaying an unacknowledged message updates
      // the same id without sounding or vibrating a second time.
      .setOnlyAlertOnce(true)
      .setCategory(NotificationCompat.CATEGORY_EMAIL)
      .setPriority(NotificationCompat.PRIORITY_DEFAULT)
      .setGroup(groupKey)
      .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
      .setSilent(!sound)
      .addExtras(Bundle().apply {
        threadId?.let { putString(EXTRA_THREAD_ID, it) }
        putString(EXTRA_ACCOUNT_ID, safeAccountId)
        putString(EXTRA_ACCOUNT_LABEL, safeAccountLabel)
        putString(EXTRA_SENDER, safeSender)
        putString(EXTRA_SUBJECT, safeSubject)
      })
    if (threadId != null) {
      notification
        .addAction(R.drawable.ic_notification, NativeUiStrings.get(context, NativeUiStrings.ARCHIVE), backgroundActionIntent(context, eventId, threadId, ACTION_ARCHIVE))
        .addAction(R.drawable.ic_notification, NativeUiStrings.get(context, NativeUiStrings.READ), backgroundActionIntent(context, eventId, threadId, ACTION_READ))
        .addAction(R.drawable.ic_notification, NativeUiStrings.get(context, NativeUiStrings.REPLY), replyActionIntent(context, eventId, threadId))
    }
    val manager = NotificationManagerCompat.from(context)
    // The message id is the notification tag. Unlike a 32-bit hash, this
    // cannot make two different messages overwrite one another.
    manager.notify(eventId, 0, notification.build())
    updateGroupSummary(context, safeAccountId, safeAccountLabel)
    return null
  }

  fun dismissThreads(context: Context, threadIds: Collection<String>) {
    val wanted = threadIds.map(String::trim).filter(String::isNotEmpty).toSet()
    if (wanted.isEmpty()) return
    val manager = context.getSystemService(NotificationManager::class.java)
    val affectedAccounts = linkedMapOf<String, String>()
    for (entry in manager.activeNotifications.toList()) {
      val notificationThreadId = entry.notification.extras.getString(EXTRA_THREAD_ID)
        ?: entry.notification.extras.getString("threadId")
      if (notificationThreadId in wanted) {
        val accountId = entry.notification.extras.getString(EXTRA_ACCOUNT_ID)
        val accountLabel = entry.notification.extras.getString(EXTRA_ACCOUNT_LABEL)
        if (accountId != null && accountLabel != null) affectedAccounts[accountId] = accountLabel
        manager.cancel(entry.tag, entry.id)
      }
    }
    for ((accountId, accountLabel) in affectedAccounts) {
      updateGroupSummary(context, accountId, accountLabel)
    }
  }

  private fun updateGroupSummary(context: Context, accountId: String, accountLabel: String) {
    val notificationManager = context.getSystemService(NotificationManager::class.java)
    val groupKey = groupKey(accountId)
    val summaryId = summaryId(groupKey)
    val children = notificationManager.activeNotifications
      .filter { it.notification.group == groupKey && it.id != summaryId }
      .sortedByDescending { it.postTime }
    if (children.size < 2) {
      notificationManager.cancel(summaryId)
      return
    }

    val summaryTitle = NativeUiStrings.format(
      context,
      NativeUiStrings.SUMMARY,
      "count",
      children.size.toString(),
    )
    val inbox = NotificationCompat.InboxStyle()
      .setBigContentTitle(summaryTitle)
      .setSummaryText(accountLabel)
    children.take(5).forEach { child ->
      val sender = child.notification.extras.getString(EXTRA_SENDER).orEmpty()
      val subject = child.notification.extras.getString(EXTRA_SUBJECT).orEmpty()
      inbox.addLine(listOf(sender, subject).filter(String::isNotEmpty).joinToString(" · "))
    }
    val openApp = PendingIntent.getActivity(
      context,
      summaryId,
      Intent(context, MainActivity::class.java).apply {
        flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
      },
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    val summary = NotificationCompat.Builder(context, CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_notification)
      .setContentTitle(summaryTitle)
      .setContentText(accountLabel)
      .setSubText(accountLabel)
      .setStyle(inbox)
      .setContentIntent(openApp)
      .setAutoCancel(true)
      .setCategory(NotificationCompat.CATEGORY_EMAIL)
      .setPriority(NotificationCompat.PRIORITY_DEFAULT)
      .setGroup(groupKey)
      .setGroupSummary(true)
      .setGroupAlertBehavior(NotificationCompat.GROUP_ALERT_CHILDREN)
      .setSilent(true)
      .build()
    notificationManager.notify(summaryId, summary)
  }

  private fun groupKey(accountId: String): String = "$GROUP_KEY.$accountId"

  private fun summaryId(groupKey: String): Int = "summary:$groupKey".hashCode() and Int.MAX_VALUE

  private fun backgroundActionIntent(context: Context, eventId: String, threadId: String, actionId: String): PendingIntent {
    require(actionId == ACTION_ARCHIVE || actionId == ACTION_READ)
    val intent = Intent(context, NotificationActionReceiver::class.java).apply {
      action = NotificationActionReceiver.ACTION_NOTIFICATION_ACTION
      data = notificationUri(actionId, eventId)
      putExtra(EXTRA_EVENT_ID, eventId)
      putExtra(EXTRA_THREAD_ID, threadId)
      putExtra(EXTRA_ACTION_ID, actionId)
    }
    return PendingIntent.getBroadcast(
      context,
      "$eventId:$actionId".hashCode() and Int.MAX_VALUE,
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
  }

  private fun replyActionIntent(context: Context, eventId: String, threadId: String): PendingIntent {
    val intent = Intent(context, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
      data = notificationUri(ACTION_REPLY, eventId)
      putExtra(EXTRA_EVENT_ID, eventId)
      putExtra(EXTRA_THREAD_ID, threadId)
      putExtra(EXTRA_ACTION_ID, ACTION_REPLY)
    }
    return PendingIntent.getActivity(
      context,
      "$eventId:$ACTION_REPLY".hashCode() and Int.MAX_VALUE,
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
  }

  private fun notificationUri(operation: String, eventId: String): Uri = Uri.Builder()
    .scheme("satchel-notification")
    .authority(operation)
    .appendPath(eventId)
    .build()

}

/**
 * A receiver is the only component a notification action starts directly.
 * WorkManager owns the durable mutation, so process death or a temporarily
 * unavailable provider cannot lose the user's tap.
 */
class NotificationActionReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    if (intent.action != ACTION_NOTIFICATION_ACTION) return
    val actionId = intent.getStringExtra(SatchelNotifications.EXTRA_ACTION_ID)
    if (actionId != SatchelNotifications.ACTION_ARCHIVE && actionId != SatchelNotifications.ACTION_READ) return
    val threadId = intent.getStringExtra(SatchelNotifications.EXTRA_THREAD_ID)
      ?.trim()
      ?.takeIf { it.isNotEmpty() && it.length <= 512 }
      ?: return
    val eventId = intent.getStringExtra(SatchelNotifications.EXTRA_EVENT_ID)
      ?.trim()
      ?.takeIf { it.isNotEmpty() && it.length <= 512 }
      ?: return

    SatchelNotifications.dismissThreads(context, listOf(threadId))
    NotificationActionWorkScheduler.enqueue(context, eventId, threadId, actionId)
  }

  companion object {
    const val ACTION_NOTIFICATION_ACTION = "de.tbuck.satchel.NOTIFICATION_ACTION"
  }
}


object PendingNavigation {
  data class PendingNotification(val threadId: String?, val actionId: String?)

  private const val preferencesName = "satchel.notification-navigation"
  private const val threadIdKey = "threadId"
  private const val actionIdKey = "actionId"
  private var inMemory: PendingNotification? = null

  @Synchronized
  fun capture(context: Context, intent: Intent?) {
    val threadId = intent?.getStringExtra(SatchelNotifications.EXTRA_THREAD_ID)?.trim()?.takeIf { it.isNotEmpty() && it.length <= 512 } ?: return
    val actionId = intent.getStringExtra(SatchelNotifications.EXTRA_ACTION_ID)
      ?.takeIf { it in setOf(SatchelNotifications.ACTION_ARCHIVE, SatchelNotifications.ACTION_READ, SatchelNotifications.ACTION_REPLY) }
    val pending = PendingNotification(threadId, actionId)
    inMemory = pending
    // These are opaque navigation ids, not secrets. Keeping them in
    // their own tiny preference avoids doing Android Keystore I/O on the main
    // thread while the Activity is being brought to the foreground.
    runCatching {
      context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
        .edit()
        .putString(threadIdKey, threadId)
        .apply {
          if (actionId == null) remove(actionIdKey) else putString(actionIdKey, actionId)
        }
        .commit()
    }
  }

  fun clearIntent(intent: Intent?) {
    intent?.removeExtra(SatchelNotifications.EXTRA_THREAD_ID)
    intent?.removeExtra(SatchelNotifications.EXTRA_ACTION_ID)
    intent?.removeExtra(SatchelNotifications.EXTRA_EVENT_ID)
  }

  @Synchronized
  fun consume(context: Context): PendingNotification {
    val preferences = context.getSharedPreferences(preferencesName, Context.MODE_PRIVATE)
    val persisted = PendingNotification(
      preferences.getString(threadIdKey, null),
      preferences.getString(actionIdKey, null),
    )
    val pending = inMemory ?: persisted
    inMemory = null
    runCatching {
      preferences.edit().remove(threadIdKey).remove(actionIdKey).commit()
    }
    return pending
  }
}
