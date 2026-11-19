package de.tbuck.satchel.background

import android.content.Context
import android.net.ConnectivityManager
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Handler
import android.os.Looper
import android.util.Base64
import app.tauri.plugin.JSObject
import com.google.android.gms.tasks.Tasks
import com.google.firebase.messaging.FirebaseMessaging
import org.json.JSONArray
import org.json.JSONObject
import java.io.DataInputStream
import java.io.DataOutputStream
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.InetSocketAddress
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.UUID
import java.util.concurrent.TimeUnit
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.X509TrustManager

/** Only the phone and the terminal running its one-use command can authenticate the TLS certificate.
 * DNS-SD records are public and untrusted, including the display name. */
internal class PushPairing(private val context: Context) {
  private val nsd = context.getSystemService(NsdManager::class.java)
  private val main = Handler(Looper.getMainLooper())
  private val io = java.util.concurrent.ForkJoinPool.commonPool()
  private val prefs = PushStore.prefs(context)
  private var phase = "idle"
  private var key = ""
  private var candidate: NsdServiceInfo? = null
  private var discovery: NsdManager.DiscoveryListener? = null
  private var generation = 0
  private var announcements: DatagramSocket? = null
  private var directAddress: String? = null

  private fun end(next: String) {
    generation++
    discovery?.let { runCatching { nsd.stopServiceDiscovery(it) } }
    discovery = null
    announcements?.close()
    announcements = null
    directAddress = null
    key = ""
    candidate = null
    phase = next
  }

  /** The same command also works on LANs that filter multicast. Listen only during pairing. */
  @Suppress("DEPRECATION")
  private fun listenForAnnouncement(session: Int) {
    val network = context.getSystemService(ConnectivityManager::class.java)
    val address = network.getLinkProperties(network.activeNetwork)?.linkAddresses
      ?.map { it.address }?.filterIsInstance<Inet4Address>()?.firstOrNull() ?: return
    val socket = DatagramSocket(0)
    announcements = socket
    directAddress = "${address.hostAddress}:${socket.localPort}"
    val secret = Base64.decode(key, base64Flags)
    io.execute {
      val buffer = ByteArray(1024)
      while (!socket.isClosed) {
        val packet = DatagramPacket(buffer, buffer.size)
        try { socket.receive(packet) } catch (_: java.io.IOException) { break }
        val info = runCatching {
          val data = JSONObject(String(packet.data, 0, packet.length, Charsets.UTF_8))
          val label = data.getString("name")
          val port = data.getInt("port")
          val proof = data.getString("proof")
          require(port in 1..65535 && label.length in 1..80)
          val authenticated = mac(secret, "announce:$port:$proof:$label".toByteArray())
          require(MessageDigest.isEqual(authenticated, Base64.decode(data.getString("mac"), base64Flags)))
          NsdServiceInfo().apply {
            this.host = packet.address
            this.port = port
            setAttribute("name", label)
            setAttribute("proof", proof)
          }
        }.getOrNull() ?: continue
        main.post {
          if (generation == session && phase in listOf("searching", "found")) {
            candidate = info
            phase = "found"
          }
        }
      }
    }
  }

  private fun start() {
    end("searching")
    key = Base64.encodeToString(ByteArray(32).also(SecureRandom()::nextBytes), base64Flags)
    val wanted = "Satchel-" + digest(key).take(16)
    val session = generation
    runCatching { listenForAnnouncement(session) }
    val listener = object : NsdManager.DiscoveryListener {
      override fun onDiscoveryStarted(type: String) {}
      override fun onDiscoveryStopped(type: String) {}
      override fun onStopDiscoveryFailed(type: String, error: Int) {}
      override fun onStartDiscoveryFailed(type: String, error: Int) {
        main.post { if (generation == session && directAddress == null) end("failed") }
      }
      override fun onServiceLost(service: NsdServiceInfo) {
        main.post {
          if (generation == session && service.serviceName == wanted && phase == "found") {
            candidate = null
            phase = "searching"
          }
        }
      }
      @Suppress("DEPRECATION")
      override fun onServiceFound(service: NsdServiceInfo) {
        if (service.serviceName != wanted) return
        nsd.resolveService(service, object : NsdManager.ResolveListener {
          override fun onResolveFailed(info: NsdServiceInfo, error: Int) {
            main.post { if (generation == session && directAddress == null) end("failed") }
          }
          override fun onServiceResolved(info: NsdServiceInfo) {
            main.post {
              if (generation == session && phase == "searching" && name(info).isNotBlank()) {
                candidate = info
                phase = "found"
              }
            }
          }
        })
      }
    }
    discovery = listener
    runCatching { nsd.discoverServices("_satchel-wake._tcp.", NsdManager.PROTOCOL_DNS_SD, listener) }
      .onFailure { if (directAddress == null) end("failed") }
    main.postDelayed({ if (generation == session && phase != "connecting") end("expired") }, 600_000)
  }

  private fun name(info: NsdServiceInfo) =
    info.attributes["name"]?.toString(Charsets.UTF_8)?.take(80).orEmpty()

  @Suppress("DEPRECATION")
  private fun exchange(info: NsdServiceInfo, secret: String, request: JSONObject): JSONObject {
    val rawKey = Base64.decode(secret, base64Flags)
    val proof = info.attributes["proof"] ?: error("proof")
    val trust = object : X509TrustManager {
      override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
      override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) =
        throw CertificateException()
      override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
        val cert = chain.firstOrNull() ?: throw CertificateException()
        val expected = mac(rawKey, (name(info) + "\u0000").toByteArray() + cert.encoded)
        if (!MessageDigest.isEqual(Base64.encode(expected, base64Flags), proof)) throw CertificateException()
        cert.checkValidity()
      }
    }
    val tls = SSLContext.getInstance("TLS").apply { init(null, arrayOf(trust), null) }
    return (tls.socketFactory.createSocket() as SSLSocket).use { socket ->
      socket.soTimeout = 30_000
      socket.connect(InetSocketAddress(info.host, info.port), 5_000)
      socket.startHandshake()
      val body = request.put("secret", secret).toString().toByteArray()
      val output = DataOutputStream(socket.outputStream)
      output.writeInt(body.size)
      output.write(body)
      output.flush()
      val input = DataInputStream(socket.inputStream)
      val size = input.readInt()
      require(size in 1..8192)
      JSONObject(ByteArray(size).also(input::readFully).toString(Charsets.UTF_8))
    }
  }

  private fun confirm() {
    val info = candidate ?: error("candidate")
    require(phase == "found")
    phase = "connecting"
    val secret = key
    io.execute {
      var id = UUID.randomUUID().toString().replace("-", "")
      var existing: JSONObject? = null
      val result = runCatching {
        val nonce = UUID.randomUUID().toString()
        val hello = JSONObject().put("op", "config").put("nonce", nonce)
        prefs.getString("firebase", null)?.let { hello.put("firebase", JSONObject(it)) }
        val response = exchange(info, secret, hello)
        val known = response.optJSONArray("known") ?: JSONArray()
        val proofs = (0 until known.length()).map { known.optString(it) }.toSet()
        existing = PushStore.devices(context).firstOrNull { previous ->
          val key = Base64.decode(previous.getString("key"), base64Flags)
          Base64.encodeToString(mac(key, "reconnect:$nonce".toByteArray()), base64Flags) in proofs
        }
        existing?.let { id = it.getString("id") }
        val config = response.getJSONObject("firebase")
        val messaging = PushStore.initialize(context, config)
        val token = Tasks.await(messaging.token, 30, TimeUnit.SECONDS)
        val device = existing ?: JSONObject().put("id", id)
          .put("key", Base64.encodeToString(ByteArray(32).also(SecureRandom()::nextBytes), base64Flags))
        device.put("name", name(info))
        // Keep a random, per-pair topic for token rotation. The normal path addresses the token.
        Tasks.await(messaging.subscribeToTopic("satchel_$id"), 30, TimeUnit.SECONDS)
        check(prefs.edit().putString("firebase", config.toString())
          .putString("device.$id", device.toString()).commit())
        val receipt = exchange(info, secret, JSONObject().put("op", "pair").put("device", id)
          .put("token", token).put("key", device.getString("key")))
        check(receipt.optBoolean("ok"))
        messaging.isAutoInitEnabled = true
      }
      if (result.isFailure) {
        android.util.Log.w("SatchelPush", "Pairing failed (${result.exceptionOrNull()?.javaClass?.simpleName})")
        if (existing == null) {
          prefs.edit().remove("device.$id").remove("received.$id").apply()
          runCatching { FirebaseMessaging.getInstance().unsubscribeFromTopic("satchel_$id") }
          PushStore.stopIfUnused(context)
        }
      }
      main.post { end(if (result.isSuccess) "idle" else "failed") }
    }
  }

  fun apply(action: String, id: String?): JSObject {
    when (action) {
      "start" -> { require(phase != "connecting"); start() }
      "confirm" -> confirm()
      "cancel" -> if (phase != "connecting") end("idle")
      "remove" -> {
        require(id != null && prefs.contains("device.$id"))
        check(prefs.edit().remove("device.$id").remove("received.$id").commit())
        FirebaseMessaging.getInstance().unsubscribeFromTopic("satchel_$id")
        PushStore.stopIfUnused(context)
      }
      "status" -> Unit
      else -> error("action")
    }
    return JSObject().apply {
      put("phase", phase)
      put("candidate", candidate?.let(::name))
      if (phase == "searching" || phase == "found") {
        val project = prefs.getString("firebase", null)?.let { JSONObject(it).optString("project") }
        val option = project?.takeIf { it.matches(Regex("[a-z][a-z0-9-]{4,28}[a-z0-9]")) }
          ?.let { " --project $it" }.orEmpty()
        val direct = directAddress?.let { " --discover $it" }.orEmpty()
        put("command", "curl -fsSL https://raw.githubusercontent.com/tbuck-software/satchel/main/tooling/push-device/install.sh | sh -s -- $key$option$direct")
      }
      put("devices", JSONArray(PushStore.devices(context).filterNot { it.has("relay") }.map {
        JSONObject().put("id", it.getString("id")).put("name", it.getString("name"))
          .put("state", if (prefs.contains("received." + it.getString("id"))) "verified" else "paired")
      }))
    }
  }

  private fun digest(value: String) = MessageDigest.getInstance("SHA-256")
    .digest(value.toByteArray()).joinToString("") { "%02x".format(it) }
}
