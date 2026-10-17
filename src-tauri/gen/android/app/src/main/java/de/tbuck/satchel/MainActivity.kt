package de.tbuck.satchel

import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.util.Base64
import android.util.Log
import androidx.activity.enableEdgeToEdge
import de.tbuck.satchel.background.BackgroundSyncPlugin
import de.tbuck.satchel.background.SatchelNotifications
import de.tbuck.satchel.background.PendingNavigation
import java.io.File
import java.time.Instant

class MainActivity : TauriActivity() {
  private external fun initializePlatformTls(): Boolean

  override fun onCreate(savedInstanceState: Bundle?) {
    runCatching { PendingNavigation.capture(this, intent) }
    PendingNavigation.clearIntent(intent)
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    check(initializePlatformTls()) { "Android platform TLS verifier initialization failed" }
    AndroidExitReports.capturePendingExits(this)
    SatchelNotifications.ensureChannel(this)
  }

  override fun onStart() {
    super.onStart()
    // This runs while the Activity is visible (so Android permits the FGS
    // start) and retries whenever the task returns to the foreground.
    BackgroundSyncPlugin.restoreIfEnabled(this)
  }

  override fun onNewIntent(intent: Intent) {
    setIntent(intent)
    // A malformed or unwritable navigation payload must never stop Android
    // from bringing the already-running Activity to the foreground.
    runCatching { PendingNavigation.capture(this, intent) }
    // Tauri forwards this call to MailNotificationPlugin, which wakes the frontend even
    // when an already-focused singleTask Activity gets no resume/focus event.
    super.onNewIntent(intent)
    PendingNavigation.clearIntent(intent)
  }
}

/**
 * Persists Android's post-mortem exit information for the Rust crash-report UI.
 *
 * ANRs, native crashes and low-memory kills do not execute Rust's panic hook.
 * Android 11+ instead keeps them in ApplicationExitInfo, sometimes including a
 * system-generated trace. Reading that trace can touch disk and must not delay
 * Activity startup, so collection runs on its own thread and publishes each
 * report through an atomic rename.
 */
private object AndroidExitReports {
  private const val TAG = "SatchelExitReports"
  private const val PREFERENCES = "satchel.android-exit-reports"
  private const val LAST_CAPTURED_EXIT = "last-captured-exit"
  private const val MAX_HISTORY = 16
  private const val MAX_TRACE_BYTES = 256 * 1024
  private const val MAX_PRINTABLE_EXCERPT_CHARS = 4 * 1024

  private data class TraceContent(
    val body: String,
    val encoding: String,
    val printableExcerpt: String = "",
  )

  fun capturePendingExits(context: Context) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return
    val applicationContext = context.applicationContext
    Thread(
      {
        runCatching { capturePendingExitsNow(applicationContext) }
          .onFailure { Log.w(TAG, "Unable to capture historical process exit", it) }
      },
      "satchel-anr-report",
    ).start()
  }

  private fun capturePendingExitsNow(context: Context) {
    val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    val lastCaptured = preferences.getLong(LAST_CAPTURED_EXIT, 0L)
    val activityManager = context.getSystemService(ActivityManager::class.java)
    val exits = activityManager
      .getHistoricalProcessExitReasons(context.packageName, 0, MAX_HISTORY)
      .asSequence()
      .filter {
        AndroidExitReportPolicy.isReportable(
          it.reason,
          it.processName,
          context.packageName,
          it.importance,
        ) && it.timestamp > lastCaptured
      }
      .sortedBy { it.timestamp }
      .toList()

    for (exit in exits) {
      if (!writeReport(context, exit)) continue
      // Commit only after the atomic report is visible. A failed write remains
      // eligible on the next launch instead of silently losing the report.
      if (!preferences.edit().putLong(LAST_CAPTURED_EXIT, exit.timestamp).commit()) {
        Log.w(TAG, "Unable to persist captured process-exit timestamp")
      }
    }
  }

  private fun writeReport(context: Context, exit: ApplicationExitInfo): Boolean {
    // Tauri's Android AppLog directory is dataDir/logs. Rust resolves
    // "satchel/logs" below it and drains the android-exit directory there.
    val reportDirectory = File(context.dataDir, "logs/satchel/logs/android-exit")
    if (!reportDirectory.mkdirs() && !reportDirectory.isDirectory) return false
    val target = File(reportDirectory, "${exit.timestamp}.exit.log")
    if (target.isFile) return true
    val temporary = File(reportDirectory, ".${exit.timestamp}.tmp")

    return runCatching {
      temporary.writeText(formatReport(exit))
      if (!temporary.renameTo(target)) {
        if (!target.isFile) error("Unable to publish ANR report")
        temporary.delete()
      }
      true
    }.getOrElse {
      temporary.delete()
      Log.w(TAG, "Unable to persist ANR report", it)
      false
    }
  }

  private fun formatReport(exit: ApplicationExitInfo): String {
    val description = exit.description?.trim().orEmpty().ifEmpty {
      AndroidExitReportPolicy.defaultDescription(exit.reason)
    }
    val kind = when (exit.reason) {
      ApplicationExitInfo.REASON_ANR -> "android anr"
      ApplicationExitInfo.REASON_CRASH_NATIVE -> "android native crash"
      ApplicationExitInfo.REASON_CRASH -> "android crash"
      ApplicationExitInfo.REASON_LOW_MEMORY -> "android low memory"
      else -> "android process exit"
    }
    val trace = readTrace(exit)
    return buildString {
      append("==== ").append(kind).append(" at ")
      append(Instant.ofEpochMilli(exit.timestamp))
      append(" ====\n")
      append("payload: ").append(kind).append(": ").append(description).append('\n')
      append("reason: ").append(exit.reason).append('\n')
      append("timestamp_ms: ").append(exit.timestamp).append('\n')
      append("process: ").append(exit.processName).append('\n')
      append("importance: ").append(exit.importance).append('\n')
      append("status: ").append(exit.status).append('\n')
      append("pss_kb: ").append(exit.pss).append('\n')
      append("rss_kb: ").append(exit.rss).append('\n')
      if (trace.printableExcerpt.isNotEmpty()) {
        append("trace_printable_excerpt:\n").append(trace.printableExcerpt).append('\n')
      }
      append("trace_encoding: ").append(trace.encoding).append('\n')
      append("trace:\n").append(trace.body.ifEmpty { "(no Android exit trace available)" }).append("\n\n")
    }
  }

  private fun readTrace(exit: ApplicationExitInfo): TraceContent {
    val input = runCatching { exit.traceInputStream }.getOrNull()
      ?: return TraceContent("", "none")
    return input.use { stream ->
      val bytes = stream.readNBytes(MAX_TRACE_BYTES)
      val truncated = stream.read() >= 0
      if (isLikelyText(bytes)) {
        val suffix = if (truncated) "\n… trace truncated by Satchel …" else ""
        TraceContent(bytes.toString(Charsets.UTF_8) + suffix, "utf-8")
      } else {
        val suffix = if (truncated) "\ntrace_truncated: true" else ""
        TraceContent(
          Base64.encodeToString(bytes, Base64.NO_WRAP) + suffix,
          "base64",
          printableExcerpt(bytes),
        )
      }
    }
  }

  private fun isLikelyText(bytes: ByteArray): Boolean = bytes.none { byte ->
    val value = byte.toInt() and 0xff
    value == 0 || (value < 0x20 && value != '\n'.code && value != '\r'.code && value != '\t'.code)
  }

  private fun printableExcerpt(bytes: ByteArray): String {
    val output = StringBuilder()
    val run = StringBuilder()
    fun flushRun() {
      if (run.length >= 12 && output.length < MAX_PRINTABLE_EXCERPT_CHARS) {
        if (output.isNotEmpty()) output.append('\n')
        output.append(run.take(MAX_PRINTABLE_EXCERPT_CHARS - output.length))
      }
      run.clear()
    }
    for (byte in bytes) {
      val value = byte.toInt() and 0xff
      if (value in 0x20..0x7e) run.append(value.toChar()) else flushRun()
      if (output.length >= MAX_PRINTABLE_EXCERPT_CHARS) break
    }
    flushRun()
    return output.toString()
  }
}

internal object AndroidExitReportPolicy {
  fun isReportable(reason: Int, processName: String, packageName: String, importance: Int): Boolean =
    when (reason) {
      ApplicationExitInfo.REASON_ANR,
      ApplicationExitInfo.REASON_CRASH,
      ApplicationExitInfo.REASON_CRASH_NATIVE,
      -> true
      // Android is expected to reclaim Satchel's disposable WorkManager process
      // under memory pressure, and reclaiming the cached main process the same
      // way is the system's designed lifecycle, not a defect: a cached app
      // (importance >= service) loses its memory to whatever the user opened
      // next, and Satchel restores from its persisted state on relaunch. A
      // low-memory exit is actionable only when it took down the application
      // process the user was actually looking at.
      ApplicationExitInfo.REASON_LOW_MEMORY ->
        processName == packageName && importance <= ActivityManager.RunningAppProcessInfo.IMPORTANCE_VISIBLE
      else -> false
    }

  fun defaultDescription(reason: Int): String = when (reason) {
    ApplicationExitInfo.REASON_ANR -> "App did not respond"
    ApplicationExitInfo.REASON_CRASH_NATIVE -> "Native process crash"
    ApplicationExitInfo.REASON_CRASH -> "Process crashed"
    ApplicationExitInfo.REASON_LOW_MEMORY -> "Android terminated the app under memory pressure"
    else -> "Process exited unexpectedly"
  }
}
