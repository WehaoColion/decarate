// v2.22.32 - Persist isolated diagnostic collection sessions through the background log writer.

const SESSION_PATH: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticRecordingSessionStore.kt";
const LOG_PATH: &str = "com/ofairyo/gridtimer/diagnostics/DiagnosticLogStore.kt";

pub fn render(path: &str, base: &str) -> Result<String, String> {
    match path {
        SESSION_PATH => {
            if !base.contains("object DiagnosticRecordingSessionStore {") {
                return Err("diagnostic session template changed".into());
            }
            Ok(SESSION_SOURCE.to_owned())
        }
        LOG_PATH => render_log_store(base),
        _ => Ok(base.to_owned()),
    }
}

fn replace(target: &mut String, old: &str, new: &str, label: &str) -> Result<(), String> {
    if target.matches(old).count() != 1 {
        return Err(format!("expected one {label} fragment"));
    }
    *target = target.replacen(old, new, 1);
    Ok(())
}

fn render_log_store(base: &str) -> Result<String, String> {
    let mut rendered = base.to_owned();
    replace(&mut rendered,
        "        onBufferOverflow = BufferOverflow.DROP_OLDEST\n",
        "        onBufferOverflow = BufferOverflow.DROP_OLDEST,\n        onUndeliveredElement = { entry -> entry.collectionTicket?.droppedEvents?.incrementAndGet() }\n",
        "account for dropped collection events")?;
    replace(&mut rendered,
        "        val callerThreadName: String\n",
        "        val callerThreadName: String,\n        val collectionTicket: DiagnosticCollectionTicket?\n",
        "capture collection identity in queued events")?;
    replace(&mut rendered,
        "        val application = context.applicationContext as? Application ?: return\n",
        "        val application = context.applicationContext as? Application ?: return\n        DiagnosticRecordingSessionStore.currentSession(application)\n",
        "restore collection state off the main thread")?;
    replace(&mut rendered,
        "        pendingEntries.trySend(\n            PendingLogEntry(\n                context = context.applicationContext,\n                category = category,\n                message = message,\n                throwable = throwable,\n                recordedAt = Instant.now(),\n                callerThreadName = Thread.currentThread().name\n            )\n        )",
        "        DiagnosticRecordingSessionStore.withCollectionTicket { ticket ->\n            pendingEntries.trySend(\n                PendingLogEntry(\n                    context = context.applicationContext,\n                    category = category,\n                    message = message,\n                    throwable = throwable,\n                    recordedAt = Instant.now(),\n                    callerThreadName = Thread.currentThread().name,\n                    collectionTicket = ticket\n                )\n            )\n        }",
        "make ticket capture and enqueue atomic with stop")?;
    replace(
        &mut rendered,
        "    private fun appendEntry(pending: PendingLogEntry) {",
        r#"    internal fun flushForCollection(): Boolean = runBlocking {
        withTimeoutOrNull(FLUSH_TIMEOUT_MILLIS) {
            val completion = CompletableDeferred<Unit>()
            flushRequests.send(completion)
            completion.await()
            true
        } ?: false
    }

    private fun appendEntry(pending: PendingLogEntry) {"#,
        "observable bounded collection flush",
    )?;
    replace(&mut rendered,
        "        synchronized(fileLock) {\n            val file = logFile(pending.context)\n            file.parentFile?.mkdirs()\n            rotateBeforeAppend(file, entry.toByteArray(Charsets.UTF_8).size.toLong())\n            file.appendText(entry)\n        }",
        "        try {\n            synchronized(fileLock) {\n                val file = logFile(pending.context)\n                file.parentFile?.mkdirs()\n                rotateBeforeAppend(file, entry.toByteArray(Charsets.UTF_8).size.toLong())\n                file.appendText(entry)\n            }\n        } finally {\n            DiagnosticRecordingSessionStore.appendCollectedEvent(\n                pending.context, pending.collectionTicket, pending.recordedAt.toEpochMilli(), entry\n            )\n        }",
        "collect from the background writer even when ordinary log writing fails")?;
    Ok(rendered)
}

const SESSION_SOURCE: &str = r####"package com.ofairyo.gridtimer.diagnostics

import android.content.Context
import androidx.annotation.WorkerThread
import com.ofairyo.gridtimer.data.AtomicFileStore
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.FileOutputStream
import java.io.RandomAccessFile
import java.util.Properties
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONObject

data class DiagnosticRecordingSession(
    val isActive: Boolean,
    val startedAtEpochMillis: Long? = null,
    val stoppedAtEpochMillis: Long? = null,
    val sessionId: String? = null,
    val hasCollection: Boolean = false,
    val eventCount: Long = 0L,
    val collectedBytes: Long = 0L,
    val droppedEventCount: Long = 0L,
    val limitReached: Boolean = false,
    val interrupted: Boolean = false,
    val lastError: String? = null,
    val isLoading: Boolean = false
)

internal class DiagnosticCollectionTicket(val sessionId: String) {
    val droppedEvents = AtomicLong()
}

internal data class DiagnosticCollectionSnapshot(val session: DiagnosticRecordingSession, val events: String)

object DiagnosticRecordingSessionStore {
    private const val COLLECTION_DIRECTORY = "diagnostic_collections"
    private const val EVENTS_FILE = "events.jsonl"
    private const val SESSION_FILE = "session.properties"
    private const val POINTER_FILE = "current.properties"
    private const val MAX_COLLECTION_BYTES = 1024 * 1024
    private const val MAX_EVENT_CHARACTERS = 8 * 1024
    private const val MAX_RETAINED_COLLECTIONS = 5
    private const val PUBLISH_INTERVAL_NANOS = 1_000_000_000L
    private val safeSessionId = Regex("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")
    private val operationLock = Any()
    private val storageLock = Any()
    private val enqueueGate = Any()
    private val restoreRequested = AtomicBoolean()
    private val restoreScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val mutableSession = MutableStateFlow(DiagnosticRecordingSession(false, isLoading = true))
    val session = mutableSession.asStateFlow()
    @Volatile private var restored = false
    private var current = DiagnosticRecordingSession(false, isLoading = true)
    private var openTicket: DiagnosticCollectionTicket? = null
    private var currentTicket: DiagnosticCollectionTicket? = null
    private var lastPublishedNanos = 0L
    private var lastCheckpointNanos = 0L
    private var publicationJob: Job? = null

    fun currentSession(context: Context): DiagnosticRecordingSession {
        if (!restored && restoreRequested.compareAndSet(false, true)) {
            val appContext = context.applicationContext
            restoreScope.launch {
                try {
                    synchronized(operationLock) { ensureRestored(appContext) }
                } catch (failure: Exception) {
                    synchronized(storageLock) {
                        current = current.copy(isActive = false, isLoading = false,
                            lastError = "无法读取上次收集：${errorText(failure)}")
                        publish(force = true)
                    }
                    restoreRequested.set(false)
                }
            }
        }
        return mutableSession.value
    }

    // Callers run start/finish on Dispatchers.IO; no file work occurs in currentSession or enqueue.
    @WorkerThread
    fun start(context: Context): DiagnosticRecordingSession = synchronized(operationLock) operation@ {
        val appContext = context.applicationContext
        ensureRestored(appContext)
        synchronized(storageLock) {
            if (current.isActive) return@operation snapshotState()
            val root = collectionRoot(appContext)
            check(root.isDirectory || root.mkdirs()) { "无法创建收集目录" }
            val id = UUID.randomUUID().toString()
            val directory = File(root, id)
            check(directory.mkdir()) { "无法创建本次收集目录" }
            val started = DiagnosticRecordingSession(true, System.currentTimeMillis(), sessionId = id,
                hasCollection = true)
            try {
                FileOutputStream(File(directory, EVENTS_FILE)).use { it.fd.sync() }
                persistSession(directory, started)
                persistPointer(root, id)
            } catch (failure: Exception) {
                // Leave recoverable evidence in its isolated directory; never replace the old pointer on failure.
                throw failure
            }
            current = started
            lastCheckpointNanos = System.nanoTime()
            val ticket = DiagnosticCollectionTicket(id)
            currentTicket = ticket
            synchronized(enqueueGate) { openTicket = ticket }
            publish(force = true)
            try {
                pruneOldCollections(root, id)
            } catch (failure: Exception) {
                current = current.copy(lastError = "清理旧收集失败：${errorText(failure)}")
                runCatching { persistSession(directory, current) }
                publish(force = true)
            }
            snapshotState()
        }.also {
            DiagnosticLogStore.record(appContext, "diagnostic.collection", "Started collection session=${it.sessionId}")
        }
    }

    @WorkerThread
    fun finish(context: Context): DiagnosticRecordingSession = synchronized(operationLock) operation@ {
        val appContext = context.applicationContext
        ensureRestored(appContext)
        val active = synchronized(storageLock) { current.isActive }
        if (!active) return@operation synchronized(storageLock) { publish(force = true); snapshotState() }
        // Enqueue holds this same gate through trySend, so all accepted tickets precede the barrier.
        synchronized(enqueueGate) { openTicket = null }
        val drained = runCatching { DiagnosticLogStore.flushForCollection() }.getOrDefault(false)
        synchronized(storageLock) {
            val directory = sessionDirectory(appContext, requireNotNull(current.sessionId))
            current = snapshotState().copy(isActive = false, stoppedAtEpochMillis = System.currentTimeMillis(),
                lastError = if (!drained) "停止等待超时，未写入队列内容可能缺失" else current.lastError)
            try {
                FileOutputStream(File(directory, EVENTS_FILE), true).use { it.fd.sync() }
                persistSession(directory, current)
            } catch (failure: Exception) {
                current = current.copy(lastError = "收集已停止，保存状态失败：${errorText(failure)}")
            }
            publish(force = true)
            snapshotState()
        }
    }

    internal fun withCollectionTicket(enqueue: (DiagnosticCollectionTicket?) -> Unit) {
        synchronized(enqueueGate) { enqueue(openTicket) }
    }

    @WorkerThread
    internal fun appendCollectedEvent(context: Context, ticket: DiagnosticCollectionTicket?, epochMillis: Long, text: String) {
        if (ticket == null) return
        synchronized(storageLock) {
            if (ticket !== currentTicket) return
            if (!current.isActive) {
                ticket.droppedEvents.incrementAndGet()
                return
            }
            val directory = sessionDirectory(context, ticket.sessionId)
            try {
                val entry = JSONObject()
                    .put("epochMillis", epochMillis)
                    .put("textTruncated", text.length > MAX_EVENT_CHARACTERS)
                    .put("text", text.take(MAX_EVENT_CHARACTERS))
                    .toString() + "\n"
                val bytes = entry.toByteArray(Charsets.UTF_8)
                if (current.collectedBytes + bytes.size > MAX_COLLECTION_BYTES) {
                    ticket.droppedEvents.incrementAndGet()
                    closeAdmission(ticket)
                    current = snapshotState().copy(isActive = false, limitReached = true,
                        stoppedAtEpochMillis = System.currentTimeMillis(), lastError = "已达到 1 MiB 收集上限，收集自动停止")
                    FileOutputStream(File(directory, EVENTS_FILE), true).use { it.fd.sync() }
                    persistSession(directory, current)
                    publish(force = true)
                    return
                }
                val checkpoint = System.nanoTime() - lastCheckpointNanos >= PUBLISH_INTERVAL_NANOS
                FileOutputStream(File(directory, EVENTS_FILE), true).use { output ->
                    output.write(bytes)
                    if (checkpoint) output.fd.sync()
                }
                current = current.copy(eventCount = current.eventCount + 1L,
                    collectedBytes = current.collectedBytes + bytes.size,
                    droppedEventCount = ticket.droppedEvents.get())
                if (checkpoint) {
                    persistSession(directory, current)
                    lastCheckpointNanos = System.nanoTime()
                }
                publish(force = false)
            } catch (failure: Exception) {
                closeAdmission(ticket)
                current = snapshotState().copy(isActive = false, stoppedAtEpochMillis = System.currentTimeMillis(),
                    lastError = "收集写入失败：${errorText(failure)}")
                runCatching { persistSession(directory, current) }
                publish(force = true)
            }
        }
    }

    @WorkerThread
    internal fun snapshotForExport(context: Context): DiagnosticCollectionSnapshot = synchronized(operationLock) {
        val appContext = context.applicationContext
        ensureRestored(appContext)
        val drained = runCatching { DiagnosticLogStore.flushForCollection() }.getOrDefault(false)
        synchronized(storageLock) {
            val state = snapshotState()
            val id = state.sessionId ?: return@synchronized DiagnosticCollectionSnapshot(state, "status=no_collection")
            val file = File(sessionDirectory(appContext, id), EVENTS_FILE)
            val bytes = readBounded(file, MAX_COLLECTION_BYTES)
            val header = "session_id=$id\nevent_count=${state.eventCount}\ncollected_bytes=${state.collectedBytes}\n" +
                "dropped_event_count=${state.droppedEventCount}\nqueue_drained=$drained\n" +
                "captured_bytes=${bytes.size} file_truncated=${file.length() > bytes.size}\n"
            DiagnosticCollectionSnapshot(state, header + String(bytes, Charsets.UTF_8))
        }
    }

    private fun closeAdmission(ticket: DiagnosticCollectionTicket) {
        synchronized(enqueueGate) { if (openTicket === ticket) openTicket = null }
    }

    private fun snapshotState(): DiagnosticRecordingSession {
        val dropped = maxOf(current.droppedEventCount, currentTicket?.droppedEvents?.get() ?: 0L)
        return current.copy(droppedEventCount = dropped,
            lastError = current.lastError ?: if (dropped > 0L) "日志队列繁忙，$dropped 条事件未保留" else null)
    }

    private fun publish(force: Boolean) {
        val now = System.nanoTime()
        if (force || now - lastPublishedNanos >= PUBLISH_INTERVAL_NANOS) {
            publicationJob?.cancel()
            publicationJob = null
            mutableSession.value = snapshotState()
            lastPublishedNanos = now
        } else if (publicationJob?.isActive != true) {
            val id = current.sessionId
            publicationJob = restoreScope.launch {
                delay(1_000L)
                synchronized(storageLock) {
                    if (isActive && current.sessionId == id) {
                        publicationJob = null
                        publish(force = true)
                    }
                }
            }
        }
    }

    private fun ensureRestored(context: Context) {
        if (restored) return
        synchronized(storageLock) {
            if (restored) return
            val root = collectionRoot(context)
            val pointer = File(root, POINTER_FILE)
            var recoveryNote: String? = null
            val id = if (pointer.exists()) {
                runCatching { readProperties(pointer).getProperty("sessionId").takeIf { safeSessionId.matches(it) } }
                    .getOrElse { recoveryNote = "上次收集索引异常，已尝试恢复"; null }
            } else null
            val directory = id?.let { File(root, it).takeIf(File::isDirectory) }
                ?: root.listFiles().orEmpty().filter { it.isDirectory && safeSessionId.matches(it.name) }
                    .maxByOrNull { File(it, SESSION_FILE).lastModified() }
            if (directory == null) {
                current = DiagnosticRecordingSession(false, lastError = recoveryNote)
            } else {
                val properties = runCatching { readProperties(File(directory, SESSION_FILE)) }
                    .getOrElse { recoveryNote = "收集元数据异常，已恢复现有事件文件"; Properties() }
                val file = File(directory, EVENTS_FILE)
                val bytes = runCatching { readBounded(file, MAX_COLLECTION_BYTES) }
                    .getOrElse { recoveryNote = "收集事件读取失败：${errorText(it as? Exception ?: Exception(it))}"; ByteArray(0) }
                val text = String(bytes, Charsets.UTF_8)
                val validEvents = text.lineSequence().filter { it.isNotBlank() }
                    .count { runCatching { JSONObject(it).has("epochMillis") }.getOrDefault(false) }.toLong()
                val wasActive = properties.getProperty("isActive").toBoolean()
                val incompleteTail = bytes.isNotEmpty() && bytes.last() != '\n'.code.toByte()
                current = DiagnosticRecordingSession(
                    isActive = false,
                    startedAtEpochMillis = properties.getProperty("startedAt", "0").toLongOrNull()?.takeIf { it > 0L },
                    stoppedAtEpochMillis = properties.getProperty("stoppedAt", "0").toLongOrNull()?.takeIf { it > 0L },
                    sessionId = directory.name,
                    hasCollection = true,
                    eventCount = validEvents,
                    collectedBytes = file.length(),
                    droppedEventCount = properties.getProperty("droppedEvents", "0").toLongOrNull() ?: 0L,
                    limitReached = properties.getProperty("limitReached").toBoolean(),
                    interrupted = wasActive || properties.getProperty("interrupted").toBoolean(),
                    lastError = when {
                        incompleteTail -> "上次收集未完成写入，已保留现有内容"
                        wasActive -> "上次收集随进程退出中断，已保留已写入内容"
                        recoveryNote != null -> recoveryNote
                        else -> properties.getProperty("lastError")?.takeIf { it.isNotBlank() }
                    }
                )
                if (wasActive) runCatching { persistSession(directory, current) }
            }
            restored = true
            publish(force = true)
        }
    }

    private fun persistSession(directory: File, state: DiagnosticRecordingSession) {
        val properties = Properties().apply {
            setProperty("sessionId", requireNotNull(state.sessionId))
            setProperty("isActive", state.isActive.toString())
            setProperty("startedAt", (state.startedAtEpochMillis ?: 0L).toString())
            setProperty("stoppedAt", (state.stoppedAtEpochMillis ?: 0L).toString())
            setProperty("eventCount", state.eventCount.toString())
            setProperty("collectedBytes", state.collectedBytes.toString())
            setProperty("droppedEvents", state.droppedEventCount.toString())
            setProperty("limitReached", state.limitReached.toString())
            setProperty("interrupted", state.interrupted.toString())
            setProperty("lastError", state.lastError.orEmpty().take(512))
        }
        writeProperties(File(directory, SESSION_FILE), properties)
    }

    private fun persistPointer(root: File, id: String) {
        writeProperties(File(root, POINTER_FILE), Properties().apply { setProperty("sessionId", id) })
    }

    private fun writeProperties(file: File, properties: Properties) {
        val output = ByteArrayOutputStream()
        properties.store(output, "diagnostic collection")
        AtomicFileStore.writeBytes(file, output.toByteArray())
    }

    private fun readProperties(file: File): Properties = Properties().apply {
        load(ByteArrayInputStream(readBounded(file, 16 * 1024)))
    }

    private fun readBounded(file: File, limit: Int): ByteArray {
        RandomAccessFile(file, "r").use { input ->
            val bytes = ByteArray(minOf(input.length(), limit.toLong()).toInt())
            var size = 0
            while (size < bytes.size) {
                val read = input.read(bytes, size, bytes.size - size)
                if (read < 0) break
                size += read
            }
            return if (size == bytes.size) bytes else bytes.copyOf(size)
        }
    }

    private fun collectionRoot(context: Context): File = File(context.applicationContext.filesDir, COLLECTION_DIRECTORY)

    private fun sessionDirectory(context: Context, id: String): File {
        require(safeSessionId.matches(id)) { "收集会话标识无效" }
        return File(collectionRoot(context), id)
    }

    private fun pruneOldCollections(root: File, activeId: String) {
        val rootPath = root.canonicalFile
        val previous = root.listFiles().orEmpty()
            .filter { it.isDirectory && it.name != activeId && safeSessionId.matches(it.name) }
            .sortedByDescending { File(it, SESSION_FILE).lastModified() }
        previous.drop(MAX_RETAINED_COLLECTIONS - 1).forEach { directory ->
            check(directory.canonicalFile.parentFile == rootPath) { "旧收集目录位置无效" }
            // Only the two files owned by this store may be pruned; do not traverse arbitrary contents.
            for (name in listOf(EVENTS_FILE, SESSION_FILE)) {
                val file = File(directory, name)
                check(!file.exists() || (file.canonicalFile.parentFile == directory.canonicalFile && file.delete())) {
                    "无法清理旧收集文件"
                }
            }
            check(directory.delete()) { "无法清理旧收集目录" }
        }
    }

    private fun errorText(failure: Exception): String = "${failure.javaClass.simpleName}: ${failure.message.orEmpty().take(180)}"
}
"####;

#[cfg(test)]
mod tests {
    use super::*;

    fn original(path: &str) -> &'static str {
        crate::kotlin_sources::SOURCES
            .iter()
            .find(|s| s.path == path)
            .unwrap()
            .contents
    }

    #[test]
    fn queue_identity_is_captured_at_admission_and_drops_are_counted() {
        let base =
            crate::android_performance_override::render(LOG_PATH, original(LOG_PATH)).unwrap();
        let rendered = render(LOG_PATH, &base).unwrap();
        assert!(rendered
            .contains("withCollectionTicket { ticket ->\n            pendingEntries.trySend("));
        assert!(rendered.contains("collectionTicket = ticket"));
        assert!(rendered.contains("onUndeliveredElement = { entry -> entry.collectionTicket?.droppedEvents?.incrementAndGet() }"));
        assert!(rendered.contains(
            "} finally {\n            DiagnosticRecordingSessionStore.appendCollectedEvent("
        ));
        assert!(rendered.contains("internal fun flushForCollection(): Boolean = runBlocking"));
    }

    #[test]
    fn stop_closes_admission_before_draining_and_preserves_collection_files() {
        let stop = SESSION_SOURCE
            .split("fun finish(context:")
            .nth(1)
            .unwrap()
            .split("internal fun withCollectionTicket")
            .next()
            .unwrap();
        assert!(
            stop.find("openTicket = null").unwrap() < stop.find("flushForCollection()").unwrap()
        );
        assert!(
            stop.find("flushForCollection()").unwrap()
                < stop.find("stoppedAtEpochMillis =").unwrap()
        );
        assert!(!stop.contains("delete()"));
        assert!(!SESSION_SOURCE.contains("CrashLogStore.clear"));
        assert!(SESSION_SOURCE.contains("if (ticket !== currentTicket) return"));
        assert!(SESSION_SOURCE.contains(
            "if (!current.isActive) {\n                ticket.droppedEvents.incrementAndGet()"
        ));
    }

    #[test]
    fn collection_limits_durability_and_publication_throttling_are_explicit() {
        assert!(SESSION_SOURCE.contains("MAX_COLLECTION_BYTES = 1024 * 1024"));
        assert!(SESSION_SOURCE.contains("MAX_EVENT_CHARACTERS = 8 * 1024"));
        assert!(SESSION_SOURCE.contains("MAX_RETAINED_COLLECTIONS = 5"));
        assert!(SESSION_SOURCE.contains("AtomicFileStore.writeBytes(file, output.toByteArray())"));
        assert!(SESSION_SOURCE.contains("if (checkpoint) output.fd.sync()"));
        assert!(SESSION_SOURCE.contains("publish(force = false)"));
        assert!(SESSION_SOURCE.contains("now - lastPublishedNanos >= PUBLISH_INTERVAL_NANOS"));
        let current = SESSION_SOURCE
            .split("fun currentSession(context:")
            .nth(1)
            .unwrap()
            .split("@WorkerThread")
            .next()
            .unwrap();
        assert!(current.contains("restoreScope.launch"));
        assert!(!current.contains("readProperties("));
    }

    #[test]
    fn exporting_is_a_stable_snapshot_and_does_not_close_the_session() {
        let export = SESSION_SOURCE
            .split("fun snapshotForExport(context:")
            .nth(1)
            .unwrap()
            .split("private fun closeAdmission")
            .next()
            .unwrap();
        assert!(export.contains("ensureRestored(appContext)"));
        assert!(export.contains("synchronized(storageLock)"));
        assert!(!export.contains("openTicket = null"));
        assert!(!export.contains("persistSession("));
        assert!(SESSION_SOURCE.contains("interrupted = wasActive ||"));
    }
}
