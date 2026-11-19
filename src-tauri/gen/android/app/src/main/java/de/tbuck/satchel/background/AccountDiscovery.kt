package de.tbuck.satchel.background

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import app.tauri.plugin.JSObject
import org.json.JSONArray
import org.json.JSONObject
import java.net.Inet4Address

/** Android owns multicast sockets. Passwords and pairing codes stay in Rust. */
@Suppress("DEPRECATION")
internal class AccountDiscovery(context: Context) {
  private val nsd = context.getSystemService(NsdManager::class.java)
  private val type = "_satchel-setup._tcp."
  private var current: String? = null
  private var registration: NsdManager.RegistrationListener? = null
  private var scan: NsdManager.DiscoveryListener? = null
  private val peers = mutableMapOf<String, JSONObject>()

  @Synchronized fun apply(id: String, op: String, port: Int): JSObject {
    if (op == "start") {
      stop()
      current = id
      if (port > 0) advertise(id, port) else browse(id)
    } else if (op == "stop" && current == id) stop()
    return JSObject().apply { put("peers", if (current == id) JSONArray(peers.values.toList()) else JSONArray()) }
  }

  private fun stop() {
    current = null
    registration?.let { runCatching { nsd.unregisterService(it) } }
    scan?.let { runCatching { nsd.stopServiceDiscovery(it) } }
    registration = null
    scan = null
    peers.clear()
  }

  private fun advertise(id: String, listenPort: Int) {
    val info = NsdServiceInfo().apply {
      serviceName = id
      serviceType = type
      setPort(listenPort)
      setAttribute("name", Build.MODEL.take(60))
    }
    val listener = object : NsdManager.RegistrationListener {
      override fun onServiceRegistered(info: NsdServiceInfo) {}
      override fun onRegistrationFailed(info: NsdServiceInfo, error: Int) {}
      override fun onServiceUnregistered(info: NsdServiceInfo) {}
      override fun onUnregistrationFailed(info: NsdServiceInfo, error: Int) {}
    }
    registration = listener
    nsd.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
  }

  private fun browse(id: String) {
    val present = mutableSetOf<String>()
    val queue = ArrayDeque<NsdServiceInfo>()
    var resolving = false
    fun next() {
      if (resolving || current != id || queue.isEmpty()) return
      val item = queue.removeFirst()
      resolving = true
      fun done(info: NsdServiceInfo?) = synchronized(this) {
        resolving = false
        if (current == id && info != null && item.serviceName in present) {
          val host = if (Build.VERSION.SDK_INT >= 34) info.hostAddresses.filterIsInstance<Inet4Address>().firstOrNull() else info.host as? Inet4Address
          val name = info.attributes["name"]?.toString(Charsets.UTF_8)?.filterNot(Char::isISOControl)?.take(60)
          if (host != null && !name.isNullOrEmpty())
            peers[item.serviceName] = JSONObject().put("name", name).put("address", "${host.hostAddress}:${info.port}")
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
      override fun onStartDiscoveryFailed(type: String, error: Int) {}
      override fun onStopDiscoveryFailed(type: String, error: Int) {}
      override fun onServiceFound(info: NsdServiceInfo) = synchronized(this@AccountDiscovery) {
        if (current == id && present.size < 64 && queue.size < 64 && present.add(info.serviceName)) {
          queue.addLast(info)
          next()
        }
      }
      override fun onServiceLost(info: NsdServiceInfo) = synchronized(this@AccountDiscovery) {
        if (current == id) { present.remove(info.serviceName); peers.remove(info.serviceName) }
        Unit
      }
    }
    scan = listener
    nsd.discoverServices(type, NsdManager.PROTOCOL_DNS_SD, listener)
  }
}
