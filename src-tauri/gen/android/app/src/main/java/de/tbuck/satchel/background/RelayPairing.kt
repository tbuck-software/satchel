package de.tbuck.satchel.background

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import android.util.Base64
import androidx.work.Constraints
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import app.tauri.plugin.JSObject
import com.google.android.gms.tasks.Tasks
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONArray
import org.json.JSONObject
import java.net.URL
import java.security.SecureRandom
import java.util.concurrent.TimeUnit
import javax.net.ssl.HttpsURLConnection

private class RelayHttpException(val status: Int) : java.io.IOException("HTTP $status")

private fun call(origin: String, path: String, auth: String? = null, data: JSONObject? = null): JSONObject {
  val c = URL("$origin/v1/$path").openConnection() as HttpsURLConnection
  try {
    c.instanceFollowRedirects = false
    c.connectTimeout = 8000
    c.readTimeout = 15000
    if (auth != null) c.setRequestProperty("Authorization", "Bearer $auth")
    if (data != null) {
      c.requestMethod = "POST"
      c.doOutput = true
      c.setRequestProperty("Content-Type", "application/json")
      c.outputStream.use { it.write(data.toString().toByteArray()) }
    }
    if (c.responseCode == 410) return JSONObject().put("phase", "expired")
    if (c.responseCode !in 200..299) throw RelayHttpException(c.responseCode)
    return JSONObject(c.inputStream.bufferedReader().use { it.readText() })
  } finally { c.disconnect() }
}

private fun transmit(device: JSONObject, op: String, data: JSONObject = JSONObject()): JSONObject =
  call(device.getString("relay"), "links/" + device.getString("id") + "/" + op, device.getString("receiver"), data)

class RelayCleanup(private val context: Context, params: WorkerParameters) : Worker(context, params) {
  override fun doWork(): Result {
    val prefs = PushStore.prefs(context)
    var retry = false
    for ((key, raw) in prefs.all.filterKeys { it.startsWith("revoke.") }) {
      val failure = runCatching { transmit(JSONObject(raw as String), "remove") }.exceptionOrNull()
      if (failure == null || (failure is RelayHttpException && failure.status == 403))
        prefs.edit().remove(key).commit()
      else retry = true
    }
    return if (retry) Result.retry() else Result.success()
  }
}

class RelayReceipt(private val context: Context, params: WorkerParameters) : Worker(context, params) {
  override fun doWork(): Result {
    val id = inputData.getString("device") ?: return Result.failure()
    val prefs = PushStore.prefs(context)
    val device = prefs.getString("device.$id", null)?.let(::JSONObject) ?: return Result.success()
    val proof = prefs.getString("challenge.$id", null) ?: return Result.success()
    return try {
      transmit(device, "received", JSONObject().put("challenge", proof))
      Result.success()
    } catch (e: RelayHttpException) {
      if (e.status == 403) Result.success() else Result.retry()
    } catch (_: java.io.IOException) { Result.retry() }
  }
}

@Suppress("DEPRECATION")
internal class RelayPairing(private val context: Context) {
  private val nsd = context.getSystemService(NsdManager::class.java)
  private val prefs = PushStore.prefs(context)
  private val computers = mutableMapOf<String, String>()
  private var discovery: NsdManager.DiscoveryListener? = null
  private var generation = 0
  private var scanning = false
  private var selected: String? = null
  @Volatile private var phase = "idle"
  private var number: String? = null

  @Synchronized private fun stop() {
    generation++
    discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
    discovery = null
    scanning = false
    computers.clear()
  }

  @Synchronized private fun scan() {
    stop()
    scanning = true
    phase = "searching"
    val current = generation
    val queue = ArrayDeque<NsdServiceInfo>()
    val present = mutableSetOf<String>()
    var resolving = false
    fun next() {
      if (resolving || current != generation || queue.isEmpty()) return
      val item = queue.removeFirst()
      resolving = true
      fun done(info: NsdServiceInfo?) = synchronized(this) {
        resolving = false
        if (generation == current && info != null && item.serviceName in present) {
          val id = info.attributes["id"]?.toString(Charsets.UTF_8)
          val name = info.attributes["name"]?.toString(Charsets.UTF_8)
          if (id != null && id == item.serviceName && id.matches(Regex("[0-9a-f]{32}")) && !name.isNullOrBlank())
            computers[id] = name.filterNot(Char::isISOControl).take(60)
        }
        next()
      }
      runCatching {
        nsd.resolveService(item, object : NsdManager.ResolveListener {
          override fun onResolveFailed(info: NsdServiceInfo, error: Int) { done(null) }
          override fun onServiceResolved(info: NsdServiceInfo) { done(info) }
        })
      }.onFailure { done(null) }
    }
    val listener = object : NsdManager.DiscoveryListener {
      override fun onDiscoveryStarted(type: String) {}
      override fun onDiscoveryStopped(type: String) {}
      override fun onStopDiscoveryFailed(type: String, error: Int) {}
      override fun onStartDiscoveryFailed(type: String, error: Int) = synchronized(this@RelayPairing) {
        if (generation == current) { stop(); phase = "failed" }
      }
      override fun onServiceFound(info: NsdServiceInfo) = synchronized(this@RelayPairing) {
        if (generation == current && present.size < 64 && info.serviceName.matches(Regex("[0-9a-f]{32}")) && present.add(info.serviceName)) {
          queue.addLast(info); next()
        }
      }
      override fun onServiceLost(info: NsdServiceInfo) = synchronized(this@RelayPairing) {
        if (generation == current) { present.remove(info.serviceName); computers.remove(info.serviceName) }
      }
    }
    discovery = listener
    nsd.discoverServices("_satchel-pair._tcp.", NsdManager.PROTOCOL_DNS_SD, listener)
    android.os.Handler(android.os.Looper.getMainLooper()).postDelayed({ synchronized(this) {
      if (current == generation && scanning) { stop(); phase = "expired" }
    } }, 300_000)
  }

  private fun randomKey() = Base64.encodeToString(ByteArray(32).also(SecureRandom()::nextBytes), base64Flags)
  private fun load(id: String): JSONObject? = prefs.getString("device.$id", null)?.let(::JSONObject)
  private fun save(device: JSONObject) = check(prefs.edit().putString("device." + device.getString("id"), device.toString()).commit())

  private fun forget(device: JSONObject) {
    val id = device.getString("id")
    val credentials = JSONObject().put("id", id).put("receiver", device.getString("receiver")).put("relay", device.getString("relay"))
    check(prefs.edit().remove("device.$id").remove("received.$id").remove("challenge.$id")
      .putString("revoke.$id", credentials.toString()).commit())
    device.optString("topic").takeIf(String::isNotEmpty)?.let { FirebaseMessaging.getInstance().unsubscribeFromTopic(it) }
    val work = OneTimeWorkRequestBuilder<RelayCleanup>().setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()).build()
    WorkManager.getInstance(context).enqueueUniqueWork("satchel-relay-revoke", ExistingWorkPolicy.APPEND_OR_REPLACE, work)
    PushStore.stopIfUnused(context)
  }

  private fun select(id: String, origin: String) {
    val name = synchronized(this) { requireNotNull(computers[id]).also { stop() } }
    phase = "failed"
    val config = call(origin, "config")
    val oldProject = prefs.getString("firebase", null)?.let { JSONObject(it).optString("project") }
    if (oldProject != null && oldProject != config.getString("project")) { phase = "project"; return }
    val messaging = PushStore.initialize(context, config)
    val token = Tasks.await(messaging.token, 30, TimeUnit.SECONDS)
    val device = JSONObject().put("id", id).put("name", name).put("key", randomKey())
      .put("receiver", randomKey()).put("relay", origin).put("pending", true)
    selected = id
    try {
      check(prefs.edit().putString("firebase", config.toString()).putString("device.$id", device.toString()).commit())
      val joined = transmit(device, "join", JSONObject().put("token", token).put("key", device.getString("key")).put("phone", Build.MODEL))
      val topic = joined.getString("topic")
      require(topic.matches(Regex("satchel_[A-Za-z0-9_-]{43}")))
      device.put("topic", topic)
      save(device)
      Tasks.await(messaging.subscribeToTopic(topic), 30, TimeUnit.SECONDS)
      messaging.isAutoInitEnabled = true
      phase = "requested"
      number = joined.getString("code")
    } catch (e: Exception) { forget(device); selected = null; phase = "failed" }
  }

  // The plugin serializes calls off the UI thread. NSD callbacks only touch discovery state.
  fun apply(action: String, id: String?, origin: String): JSObject {
    if (selected == null) selected = PushStore.devices(context).firstOrNull { it.optBoolean("pending") }?.getString("id")
    when (action) {
      "start" -> { check(selected == null); scan() }
      "select" -> select(requireNotNull(id), origin)
      "cancel", "remove" -> {
        val target = if (action == "cancel") selected else id
        target?.let(::load)?.takeIf { it.has("relay") }?.let(::forget)
        stop(); selected = null; number = null; phase = "idle"
      }
      "status" -> {
        val device = selected?.let(::load)
        if (device != null) {
          val link = device.getString("id")
          val state = try {
            prefs.getString("challenge.$link", null)?.let {
              transmit(device, "received", JSONObject().put("challenge", it))
            }
            transmit(device, "status")
          } catch (e: RelayHttpException) {
            // The process can die between saving the local secret and joining the offer.
            if (e.status != 403) throw e
            JSONObject().put("phase", "expired")
          }
          phase = state.getString("phase")
          number = state.optString("code").takeIf(String::isNotEmpty)
          if (phase == "active") {
            device.remove("pending"); save(device); prefs.edit().remove("challenge.$link").apply()
            phase = "idle"; selected = null; number = null
          } else if (phase == "expired") { forget(device); selected = null; number = null }
        }
      }
      else -> error("action")
    }
    return synchronized(this) { JSObject().apply {
      put("phase", phase); put("code", number); put("name", selected?.let(::load)?.optString("name"))
      put("nearby", JSONArray(computers.map { (id, name) -> JSONObject().put("id", id).put("name", name).put("state", "visible") }))
      put("devices", JSONArray(PushStore.devices(context).filter { it.has("relay") && !it.optBoolean("pending") }.map {
        JSONObject().put("id", it.getString("id")).put("name", it.getString("name")).put("state", "verified")
      }))
    } }
  }
}
