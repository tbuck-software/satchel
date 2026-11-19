package de.tbuck.satchel.background

import android.content.Context
import android.util.Base64
import androidx.work.ListenableWorker.Result
import org.junit.Assert.assertEquals
import com.google.firebase.FirebaseApp
import org.json.JSONObject
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [28, 36])
class MailPushServiceTest {
  @Test fun remoteWorkersResolveTheSchedulerInTheMainProcess() {
    val app = RuntimeEnvironment.getApplication() as PushApplication
    assertEquals(app.packageName, app.workManagerConfiguration.defaultProcessName)
  }

  @Test fun aProviderFailureReleasesThePushQueueOnceRecoveryIsScheduled() {
    var scheduled = false
    assertEquals(Result.success(), finishPushRetry { scheduled = true })
    assertTrue(scheduled)
    assertEquals(Result.retry(), finishPushRetry { error("scheduler unavailable") })
  }

  @Test fun coldStartRestoresTheSameFirebaseProjectForAdditionalDevices() {
    val context: Context = RuntimeEnvironment.getApplication()
    val config = JSONObject().put("project", "test-satchel").put("sender", "123456")
      .put("app", "1:123456:android:aabbcc").put("apiKey", "A" + "x".repeat(38))
    val prefs = PushStore.prefs(context)
    prefs.edit().putString("firebase", config.toString()).putString("device.first", "{}").commit()
    try {
      PushStore.restore(context)
      PushStore.initialize(context, config)
      assertTrue(FirebaseApp.getInstance().options.projectId == "test-satchel")
    } finally {
      runCatching { FirebaseApp.getInstance().delete() }
      prefs.edit().clear().commit()
    }
  }

  @Test fun onlyCurrentSignalsFromAPairedSenderMayWakeTheMailbox() {
    val context: Context = RuntimeEnvironment.getApplication()
    val id = "ab".repeat(16)
    val key = ByteArray(32) { it.toByte() }
    val now = 1700000000L
    val signed = Base64.encodeToString(mac(key, "$id:$now".toByteArray()), base64Flags)
    val data = mapOf("device" to id, "time" to now.toString(), "proof" to signed)
    val prefs = PushStore.prefs(context)
    prefs.edit().putString("device.$id", JSONObject().put("key", Base64.encodeToString(key, base64Flags)).toString()).commit()
    assertTrue(PushStore.accept(context, data, now))
    assertFalse(PushStore.accept(context, data + ("proof" to "wrong"), now))
    assertFalse(PushStore.accept(context, data, now + 3601))
    prefs.edit().remove("device.$id").commit()
    assertFalse(PushStore.accept(context, data, now))
  }
}
