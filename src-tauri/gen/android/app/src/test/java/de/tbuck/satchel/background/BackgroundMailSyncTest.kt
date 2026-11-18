package de.tbuck.satchel.background

import android.Manifest
import android.app.ActivityManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Process
import androidx.work.ListenableWorker.Result
import org.json.JSONArray
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 36])
class BackgroundMailSyncTest {
  @Before fun allowMailNotifications() {
    shadowOf(RuntimeEnvironment.getApplication()).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
  }

  @Test fun serviceProcessWithoutAnActiveMailboxFetchStillDeliversMail() {
    val context: Context = RuntimeEnvironment.getApplication()
    val service = ActivityManager.RunningAppProcessInfo(context.packageName, Process.myPid() + 1, arrayOf(context.packageName))
    service.importance = ActivityManager.RunningAppProcessInfo.IMPORTANCE_SERVICE
    shadowOf(context.getSystemService(ActivityManager::class.java)).setProcesses(listOf(service))

    var receipt = JSONArray()
    val result = runBackgroundMailSync(context, 0, { _, _ ->
      """{"notifications":[{"eventId":"arrival","threadId":"thread","accountId":"account",
         "accountLabel":"Inbox","sender":"Sender","subject":"Login code","preview":"Test"}]}"""
    }, { _, _, entries ->
      receipt = JSONArray(entries)
      null
    })
    assertEquals(Result.success(), result)
    assertTrue(context.getSystemService(NotificationManager::class.java).activeNotifications.any { it.tag == "arrival" })
    assertEquals("arrival", receipt.getJSONObject(0).getString("eventId"))
    assertTrue(receipt.getJSONObject(0).getBoolean("shown"))
  }

  @Test fun disabledMailChannelIsReportedAsSuppressed() {
    val context: Context = RuntimeEnvironment.getApplication()
    val manager = context.getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(NotificationChannel(
      SatchelNotifications.CHANNEL_ID, "Mail", NotificationManager.IMPORTANCE_NONE,
    ))
    assertEquals(SatchelNotifications.REASON_PERMISSION_DENIED, showMail(context))
  }

  @Test fun disabledAppNotificationsAreReportedAsSuppressed() {
    val context: Context = RuntimeEnvironment.getApplication()
    shadowOf(context.getSystemService(NotificationManager::class.java)).setNotificationsEnabled(false)
    assertEquals(SatchelNotifications.REASON_PERMISSION_DENIED, showMail(context))
  }

  private fun showMail(context: Context): String? = SatchelNotifications.showMail(
    context, "arrival", "thread", "account", "Inbox", "Sender", "Login code", "Test", true,
  )
}
