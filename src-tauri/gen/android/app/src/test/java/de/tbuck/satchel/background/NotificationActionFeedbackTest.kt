package de.tbuck.satchel.background

import android.app.NotificationManager
import android.content.Context
import androidx.core.app.NotificationCompat
import de.tbuck.satchel.R
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28])
class NotificationActionFeedbackTest {
  private val context: Context = RuntimeEnvironment.getApplication()
  private val manager = context.getSystemService(NotificationManager::class.java)
  private val notApplied = """{"status":"not_applied","error":"database unavailable"}"""
  private val pending = """{"status":"pending","error":"network unavailable"}"""

  @Test fun multipleFailedActionsLeaveOneSilentClickableWarning() {
    repeat(5) { index ->
      assertTrue(NotificationActionFeedback.retry(context, "thread-$index", SatchelNotifications.ACTION_ARCHIVE, notApplied, 5))
    }
    val notifications = manager.activeNotifications
    assertEquals(1, notifications.size)
    val warning = notifications.single().notification
    assertNotNull(warning.contentIntent)
    assertNull(warning.sound)
    assertNull(warning.vibrate)
    assertTrue(warning.extras.getCharSequence("android.text").toString().contains("not applied"))
  }

  @Test fun locallySavedActionsRetryWithoutWarnings() {
    repeat(8) { attempt ->
      assertTrue(NotificationActionFeedback.retry(context, "thread", SatchelNotifications.ACTION_ARCHIVE, pending, attempt))
    }
    assertTrue(manager.activeNotifications.isEmpty())
  }

  @Test fun savedActionClearsItsWarningAndOldPerThreadWarnings() {
    SatchelNotifications.ensureChannel(context)
    repeat(5) { index ->
      manager.notify(index, NotificationCompat.Builder(context, SatchelNotifications.CHANNEL_ID)
        .setSmallIcon(R.drawable.ic_notification)
        .setCategory(NotificationCompat.CATEGORY_ERROR).build())
    }
    NotificationActionFeedback.retry(context, "thread", SatchelNotifications.ACTION_READ, notApplied, 5)
    assertEquals(1, manager.activeNotifications.size)
    NotificationActionFeedback.retry(context, "thread", SatchelNotifications.ACTION_READ, pending, 6)
    assertTrue(manager.activeNotifications.isEmpty())
    assertFalse(NotificationActionFeedback.retry(context, "thread", SatchelNotifications.ACTION_READ, """{"status":"completed"}""", 7))
  }

  @Test fun permanentlyFailedActionWarnsAndStopsRetrying() {
    val failed = """{"status":"failed","error":"imapUidValidityChanged"}"""
    assertFalse(NotificationActionFeedback.retry(context, "thread", SatchelNotifications.ACTION_ARCHIVE, failed, 0))
    assertEquals(1, manager.activeNotifications.size)
  }

  @Test fun unrelatedSuccessDoesNotDismissAnUnsavedAction() {
    NotificationActionFeedback.retry(context, "failed", SatchelNotifications.ACTION_READ, notApplied, 5)
    NotificationActionFeedback.retry(context, "other", SatchelNotifications.ACTION_READ, """{"status":"completed"}""", 0)
    assertEquals(1, manager.activeNotifications.size)
  }

  @Test fun unavailableNativeRuntimeCannotReportSuccess() {
    assertTrue(NotificationActionFeedback.retry(context, "thread", SatchelNotifications.ACTION_READ, null, 0))
    assertTrue(manager.activeNotifications.isEmpty())
  }
}
