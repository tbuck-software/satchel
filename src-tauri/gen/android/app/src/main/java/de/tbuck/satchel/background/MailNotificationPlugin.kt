package de.tbuck.satchel.background

import android.app.Activity
import android.content.Intent
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class NavigationEventHandlerArgs {
  lateinit var handler: Channel
}

@InvokeArg
class DismissNotificationsArgs {
  var threadIds: Array<String> = emptyArray()
}

@InvokeArg
class ShowMailNotificationArgs {
  lateinit var eventId: String
  lateinit var accountId: String
  lateinit var accountLabel: String
  lateinit var sender: String
  lateinit var subject: String
  var threadId: String = ""
  var preview: String = ""
  var sound: Boolean = true
}

/**
 * Android half of `commands::mobile_notifications`.
 *
 * Nothing here talks to a server: the in-process sync and
 * `BackgroundMailSyncWorker` produce the notifications, this plugin only hands
 * them to the platform and carries a tap back to the frontend.
 */
@TauriPlugin
class MailNotificationPlugin(private val activity: Activity) : Plugin(activity) {
  private var navigationEventHandler: Channel? = null

  @Command
  fun setNavigationEventHandler(invoke: Invoke) {
    val args = invoke.parseArgs(NavigationEventHandlerArgs::class.java)
    navigationEventHandler = args.handler
    invoke.resolve()
  }

  /**
   * Wakes the frontend for a tap that arrives while the app is already running.
   * A focused singleTask Activity gets no resume event, so without this the
   * navigation would sit in [PendingNavigation] until the next cold start.
   *
   * MainActivity captures the extras before calling `super`, and strips them
   * afterwards, so the payload is still readable here.
   */
  override fun onNewIntent(intent: Intent) {
    val threadId = intent.getStringExtra(SatchelNotifications.EXTRA_THREAD_ID)
      ?.trim()
      ?.takeIf { it.isNotEmpty() && it.length <= 512 }
      ?: return
    navigationEventHandler?.send(JSObject().put("threadId", threadId))
  }

  /** Takes the cold-start navigation exactly once. */
  @Command
  fun consumePendingNotification(invoke: Invoke) {
    try {
      val pending = PendingNavigation.consume(activity)
      invoke.resolve(JSObject().put("threadId", pending.threadId).put("actionId", pending.actionId))
    } catch (error: Exception) {
      invoke.reject("Unable to consume notification navigation")
    }
  }

  @Command
  fun dismissNotifications(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(DismissNotificationsArgs::class.java)
      SatchelNotifications.dismissThreads(activity, args.threadIds.asList())
      invoke.resolve()
    } catch (error: Exception) {
      invoke.reject("Unable to dismiss mail notifications")
    }
  }

  @Command
  fun showMailNotification(invoke: Invoke) {
    try {
      val args = invoke.parseArgs(ShowMailNotificationArgs::class.java)
      val reason = SatchelNotifications.showMail(
        activity,
        args.eventId,
        // An absent thread id must stay absent: the quick actions hang off it,
        // and an empty one would archive nothing on tap.
        args.threadId.trim().takeIf { it.isNotEmpty() && it.length <= 512 },
        args.accountId,
        args.accountLabel,
        args.sender,
        args.subject,
        args.preview,
        args.sound,
      )
      // A notification Android dropped resolves as well, carrying why. Only a
      // real handover failure rejects, so Rust can tell "not shown" from
      // "could not be handed over" instead of recording both as delivered.
      invoke.resolve(
        JSObject().apply {
          put("shown", reason == null)
          if (reason != null) put("reason", reason)
        },
      )
    } catch (error: Exception) {
      invoke.reject(error.message ?: "Unable to show mail notification")
    }
  }
}
