package de.tbuck.satchel.background

import android.app.Application
import android.content.Context
import android.os.Build
import android.util.Base64
import androidx.work.ExistingWorkPolicy
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import androidx.work.Configuration
import com.google.firebase.FirebaseApp
import com.google.firebase.FirebaseOptions
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import org.json.JSONObject
import java.security.MessageDigest
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

internal val base64Flags = Base64.NO_PADDING or Base64.NO_WRAP or Base64.URL_SAFE
internal fun mac(key: ByteArray, value: ByteArray): ByteArray =
  Mac.getInstance("HmacSHA256").run {
    init(SecretKeySpec(key, "HmacSHA256"))
    doFinal(value)
  }

internal object PushStore {
  fun prefs(context: Context) = context.getSharedPreferences("satchel.push-devices", Context.MODE_PRIVATE)
  fun devices(context: Context) = prefs(context).all.filterKeys { it.startsWith("device.") }
    .values.map { JSONObject(it as String) }

  @Synchronized
  fun initialize(context: Context, config: JSONObject): FirebaseMessaging {
    val options = FirebaseOptions.Builder()
      .setProjectId(config.getString("project"))
      .setApplicationId(config.getString("app"))
      .setGcmSenderId(config.getString("sender"))
      .setApiKey(config.getString("apiKey")).build()
    val existing = try { FirebaseApp.getInstance() } catch (_: IllegalStateException) { null }
    if (existing?.options != options) {
      require(existing == null || devices(context).isEmpty())
      existing?.delete()
      FirebaseApp.initializeApp(context, options)
    }
    return FirebaseMessaging.getInstance()
  }

  fun stopIfUnused(context: Context) {
    if (devices(context).isNotEmpty()) return
    prefs(context).edit().remove("firebase").apply()
    runCatching {
      val messaging = FirebaseMessaging.getInstance()
      messaging.isAutoInitEnabled = false
      messaging.deleteToken()
    }
  }

  fun restore(context: Context) {
    val config = prefs(context).getString("firebase", null) ?: return
    initialize(context, JSONObject(config))
  }

  fun accept(context: Context, data: Map<String, String>, now: Long): Boolean {
    val id = data["device"] ?: return false
    if (!id.matches(Regex("[0-9a-f]{32}"))) return false
    val stamp = data["time"]?.toLongOrNull() ?: return false
    if (stamp !in (now - 3600)..(now + 300)) return false
    val value = prefs(context).getString("device.$id", null) ?: return false
    val device = JSONObject(value)
    val proof = data["proof"] ?: return false
    val expected = mac(Base64.decode(device.getString("key"), base64Flags), "$id:$stamp".toByteArray())
    return MessageDigest.isEqual(Base64.encodeToString(expected, base64Flags).toByteArray(), proof.toByteArray())
  }
}

class PushApplication : Application(), Configuration.Provider {
  // AndroidX Startup runs in the UI process. The remote mail worker also needs
  // on-demand configuration to enqueue its retry through RemoteWorkManager.
  override val workManagerConfiguration: Configuration
    get() = Configuration.Builder().setDefaultProcessName(packageName).build()

  override fun onCreate() {
    super.onCreate()
    // The remote WorkManager process only accesses the mail database.
    val process = if (Build.VERSION.SDK_INT >= 28) getProcessName() else
      java.io.File("/proc/self/cmdline").readText().substringBefore('\u0000')
    if (process == packageName) runCatching { PushStore.restore(this) }
  }
}

class MailPushService : FirebaseMessagingService() {
  override fun onMessageReceived(message: RemoteMessage) {
    if (PushStore.accept(this, message.data, System.currentTimeMillis() / 1000)) {
      val device = message.data.getValue("device")
      val prefs = PushStore.prefs(this)
      if (!prefs.contains("received.$device")) prefs.edit().putBoolean("received.$device", true).apply()
      message.data["verify"]?.takeIf { it.matches(Regex("[A-Za-z0-9_-]{43}")) }?.let {
        val peer = prefs.getString("device.$device", null)?.let(::JSONObject)
        if (peer?.optBoolean("pending") == true && prefs.edit().putString("challenge.$device", it).commit()) {
          val work = OneTimeWorkRequestBuilder<RelayReceipt>().setInputData(workDataOf("device" to device)).build()
          WorkManager.getInstance(this).enqueueUniqueWork("satchel-receipt-$device", ExistingWorkPolicy.KEEP, work)
        }
      }
      val signalId = message.messageId ?: "$device-${message.data.getValue("time")}"
      BackgroundMailSyncWorkScheduler.wake(this, signalId)
    }
  }

  override fun onDeletedMessages() {
    if (PushStore.devices(this).isNotEmpty()) BackgroundMailSyncWorkScheduler.wake(this, "missed-${System.currentTimeMillis() / 1000}")
  }
}
