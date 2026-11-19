package de.tbuck.satchel.background

import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.ListenableWorker.Result
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkInfo
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.multiprocess.RemoteCoroutineWorker
import androidx.work.multiprocess.RemoteListenableWorker
import androidx.work.multiprocess.RemoteWorkManager
import androidx.work.multiprocess.RemoteWorkerService
import androidx.work.workDataOf
import de.tbuck.satchel.MainActivity
import de.tbuck.satchel.R
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

object NotificationActionWorkScheduler {
  private const val EVENT_ID = "eventId"
  private const val THREAD_ID = "threadId"
  private const val ACTION_ID = "actionId"

  fun enqueue(context: Context, eventId: String, threadId: String, actionId: String) {
    val component = ComponentName(context.packageName, RemoteWorkerService::class.java.name)
    val input = workDataOf(
      EVENT_ID to eventId,
      THREAD_ID to threadId,
      ACTION_ID to actionId,
      RemoteListenableWorker.ARGUMENT_PACKAGE_NAME to component.packageName,
      RemoteListenableWorker.ARGUMENT_CLASS_NAME to component.className,
    )
    val request = OneTimeWorkRequestBuilder<NotificationActionWorker>()
      .setInputData(input)
      .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 15, TimeUnit.SECONDS)
      .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
      .build()
    WorkManager.getInstance(context).enqueueUniqueWork(
      "satchel-notification-action:$eventId:$actionId",
      ExistingWorkPolicy.KEEP,
      request,
    )
  }

  internal fun threadId(params: WorkerParameters): String? =
    params.inputData.getString(THREAD_ID)?.trim()?.takeIf { it.isNotEmpty() && it.length <= 512 }

  internal fun actionId(params: WorkerParameters): String? =
    params.inputData.getString(ACTION_ID)?.takeIf {
      it == SatchelNotifications.ACTION_ARCHIVE || it == SatchelNotifications.ACTION_READ
    }
}

object NotificationActionNative {
  init {
    System.loadLibrary("satchel_lib")
  }

  external fun executeNotificationAction(
    context: Context,
    databasePath: String,
    eventId: String,
    actionId: String,
    threadId: String,
  ): String?
}

class NotificationActionWorker(context: Context, private val params: WorkerParameters) :
  RemoteCoroutineWorker(context, params) {
  override suspend fun doRemoteWork(): Result {
    val eventId = params.inputData.getString("eventId")?.trim()?.takeIf { it.isNotEmpty() }
      ?: return Result.failure()
    val threadId = NotificationActionWorkScheduler.threadId(params) ?: return Result.failure()
    val actionId = NotificationActionWorkScheduler.actionId(params) ?: return Result.failure()
    val database = applicationContext.dataDir.resolve("satchel/satchel.db")
    val outcome = runCatching {
      NotificationActionNative.executeNotificationAction(
        applicationContext,
        database.absolutePath,
        eventId,
        actionId,
        threadId,
      )
    }.getOrNull()
    return if (NotificationActionFeedback.retry(
        applicationContext, threadId, actionId, outcome, runAttemptCount,
      )) Result.retry() else Result.success()
  }
}

/** Local persistence and permanent provider failures need attention; provider retries stay silent. */
internal object NotificationActionFeedback {
  private const val FAILURE_TAG = "satchel.notification-action-failed"

  fun retry(context: Context, threadId: String, actionId: String, outcome: String?, attempt: Int): Boolean {
    val status = runCatching { JSONObject(outcome ?: "").getString("status") }.getOrNull()
    val manager = context.getSystemService(NotificationManager::class.java)
    // Remove the old per-thread warnings, including those left by an upgrade.
    manager.activeNotifications.filter {
      it.notification.category == NotificationCompat.CATEGORY_ERROR &&
        it.notification.channelId == SatchelNotifications.CHANNEL_ID &&
        it.notification.contentIntent == null
    }.forEach { manager.cancel(it.tag, it.id) }
    if (status == "completed" || status == "pending") {
      manager.activeNotifications.filter {
        it.tag == FAILURE_TAG &&
          it.notification.extras.getString(SatchelNotifications.EXTRA_THREAD_ID) == threadId
      }.forEach { manager.cancel(it.tag, it.id) }
    } else if (status == "failed" || attempt == 5) {
      showFailure(context, threadId, actionId)
    }
    // A failed provider action waits in the app for retry or discard; running
    // the work again cannot change it.
    return status != "completed" && status != "failed"
  }

  private fun showFailure(context: Context, threadId: String, actionId: String) {
    SatchelNotifications.ensureChannel(context)
    val label = if (actionId == SatchelNotifications.ACTION_READ) {
      NativeUiStrings.get(context, NativeUiStrings.READ)
    } else {
      NativeUiStrings.get(context, NativeUiStrings.ARCHIVE)
    }
    val notification = NotificationCompat.Builder(context, SatchelNotifications.CHANNEL_ID)
      .setSmallIcon(R.drawable.ic_notification)
      .setContentTitle(NativeUiStrings.get(context, NativeUiStrings.ACTION_FAILED_TITLE))
      .setContentText(
        NativeUiStrings.format(
          context,
          NativeUiStrings.ACTION_NOT_APPLIED_MESSAGE,
          "action",
          label,
        ),
      )
      .setContentIntent(PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).apply {
          flags = Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
          putExtra(SatchelNotifications.EXTRA_THREAD_ID, threadId)
        },
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
      ))
      .addExtras(android.os.Bundle().apply {
        putString(SatchelNotifications.EXTRA_THREAD_ID, threadId)
      })
      .setSilent(true)
      .setOnlyAlertOnce(true)
      .setCategory(NotificationCompat.CATEGORY_ERROR)
      .setAutoCancel(true)
      .build()
    context.getSystemService(NotificationManager::class.java)
      .notify(FAILURE_TAG, 0, notification)
  }
}

/**
 * Durable safety net for the local foreground connection.
 *
 * The keepalive service can outlive a working mail connection. The Rust
 * account leases serialize actual inbox fetches across both processes.
 */
object BackgroundMailSyncWorkScheduler {
  private const val PERIODIC_NAME = "satchel-background-mail-sync-v1"
  private val constraints = Constraints.Builder()
    .setRequiredNetworkType(NetworkType.CONNECTED)
    .build()

  private const val PUSH_NAME = "satchel-push-inbox"
  private const val RECOVERY_NAME = "satchel-push-recovery"
  private const val PUSH_INPUT = "push-wake"

  private fun request(context: Context, push: Boolean): androidx.work.OneTimeWorkRequest.Builder {
    val component = ComponentName(context.packageName, RemoteWorkerService::class.java.name)
    return OneTimeWorkRequestBuilder<BackgroundMailSyncWorker>()
      .setInputData(workDataOf(
        PUSH_INPUT to push,
        RemoteListenableWorker.ARGUMENT_PACKAGE_NAME to component.packageName,
        RemoteListenableWorker.ARGUMENT_CLASS_NAME to component.className,
      ))
      .setConstraints(constraints)
  }

  // FCM calls on a background thread. Wait for the enqueue transaction before admitting
  // the next signal. A running fetch may already have passed the changed mailbox.
  @Synchronized
  fun wake(context: Context, signalId: String) {
    if (!BackgroundSyncPlugin.isEnabled(context)) return
    val manager = WorkManager.getInstance(context)
    val active = manager.getWorkInfosForUniqueWork(PUSH_NAME).get().filterNot { it.state.isFinished }
    if (active.any { signalId in it.tags || it.state == WorkInfo.State.BLOCKED ||
        (it.state == WorkInfo.State.ENQUEUED && it.runAttemptCount == 0) }) return
    val policy = if (active.any { it.state == WorkInfo.State.RUNNING }) ExistingWorkPolicy.APPEND_OR_REPLACE
      else ExistingWorkPolicy.REPLACE
    val work = request(context, true).addTag(signalId)
      .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST).build()
    manager.enqueueUniqueWork(PUSH_NAME, policy, work).result.get()
  }

  fun isPush(params: WorkerParameters) = params.inputData.getBoolean(PUSH_INPUT, false)

  // A failing provider shares one backoff across all senders. New arrivals in healthy
  // inboxes can still start an expedited fetch while this retry waits.
  fun recover(context: Context) {
    val work = request(context, false).setInitialDelay(30, TimeUnit.SECONDS)
      .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS).build()
    RemoteWorkManager.getInstance(context).enqueueUniqueWork(RECOVERY_NAME, ExistingWorkPolicy.KEEP, work).get()
  }

  fun schedulePeriodic(context: Context) {
    val component = ComponentName(context.packageName, RemoteWorkerService::class.java.name)
    val request = PeriodicWorkRequestBuilder<BackgroundMailSyncWorker>(15, TimeUnit.MINUTES)
      .setInputData(
        workDataOf(
          RemoteListenableWorker.ARGUMENT_PACKAGE_NAME to component.packageName,
          RemoteListenableWorker.ARGUMENT_CLASS_NAME to component.className,
        ),
      )
      .setConstraints(constraints)
      .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
      .build()
    WorkManager.getInstance(context).enqueueUniquePeriodicWork(
      PERIODIC_NAME,
      ExistingPeriodicWorkPolicy.UPDATE,
      request,
    )
  }

  fun cancel(context: Context) {
    WorkManager.getInstance(context).cancelUniqueWork(PERIODIC_NAME)
  }

  fun rescheduleIfEnabled(context: Context) {
    if (BackgroundSyncPlugin.isEnabled(context)) schedulePeriodic(context) else cancel(context)
  }
}

object BackgroundMailSyncNative {
  init {
    System.loadLibrary("satchel_lib")
  }

  external fun syncMail(context: Context, databasePath: String): String

  external fun acknowledgeNotifications(
    context: Context,
    databasePath: String,
    eventIdsJson: String,
  ): String?
}

class BackgroundMailSyncWorker(context: Context, private val params: WorkerParameters) :
  RemoteCoroutineWorker(context, params) {
  override suspend fun doRemoteWork(): Result {
    val outcome = runBackgroundMailSync(applicationContext, runAttemptCount)
    if (outcome != Result.retry() || !BackgroundMailSyncWorkScheduler.isPush(params)) return outcome
    return finishPushRetry { BackgroundMailSyncWorkScheduler.recover(applicationContext) }
  }
}

internal fun finishPushRetry(schedule: () -> Unit): Result = try {
  schedule()
  Result.success()
} catch (error: Exception) {
  Log.w(TAG, "Could not schedule shared mail retry", error)
  Result.retry()
}

internal fun runBackgroundMailSync(
  applicationContext: Context,
  runAttemptCount: Int,
  syncMail: (Context, String) -> String = { context, path -> BackgroundMailSyncNative.syncMail(context, path) },
  acknowledge: (Context, String, String) -> String? = { context, path, entries ->
    BackgroundMailSyncNative.acknowledgeNotifications(context, path, entries)
  },
): Result {
  if (!BackgroundSyncPlugin.isEnabled(applicationContext)) return Result.success()
  val database = applicationContext.dataDir.resolve("satchel/satchel.db")
  return runCatching {
    val response = JSONObject(
      syncMail(applicationContext, database.absolutePath),
    )
    val notifications = response.optJSONArray("notifications")
    // What Android did with each mail, not what we asked it to do. Posting
    // every event id unseen recorded a notification refused for a missing
    // POST_NOTIFICATIONS permission as delivered, and nothing retried it.
    val acknowledgements = JSONArray()
    if (notifications != null) {
      for (index in 0 until notifications.length()) {
        val notification = notifications.getJSONObject(index)
        val reason = SatchelNotifications.showMail(
          applicationContext,
          notification.getString("eventId"),
          notification.optString("threadId", "").trim().takeIf(String::isNotEmpty),
          notification.getString("accountId"),
          notification.getString("accountLabel"),
          notification.getString("sender"),
          notification.getString("subject"),
          notification.getString("preview"),
          notification.optBoolean("sound", true),
        )
        acknowledgements.put(
          JSONObject()
            .put("eventId", notification.getString("eventId"))
            .put("shown", reason == null)
            .put("reason", reason),
        )
      }
    }
    if (acknowledgements.length() > 0) {
      val acknowledgementError = acknowledge(
        applicationContext,
        database.absolutePath,
        acknowledgements.toString(),
      )
      check(acknowledgementError == null) {
        acknowledgementError ?: "Notification acknowledgement failed"
      }
    }
    if (response.optBoolean("retry", false)) {
      Log.w(TAG, "Headless mail sync incomplete: ${response.optString("error", "provider error")}")
      if (runAttemptCount < 5) Result.retry() else Result.success()
    } else {
      Result.success()
    }
  }.getOrElse { error ->
    Log.e(TAG, "Headless mail sync failed", error)
    if (runAttemptCount < 5) Result.retry() else Result.success()
  }
}

private const val TAG = "SatchelBackgroundMail"
