// v2.22.49.8 Android - Reuse exact snapshot parsing within one startup read.
const REPOSITORY: &str = "com/ofairyo/gridtimer/data/TimerRepository.kt";

fn replace(source: &mut String, old: &str, new: &str) -> Result<(), String> {
    if source.matches(old).count() != 1 {
        return Err(format!("startup decode anchor is not unique: {old}"));
    }
    *source = source.replacen(old, new, 1);
    Ok(())
}

pub fn render(path: &str, base: &str) -> Result<String, String> {
    if path != REPOSITORY {
        return Ok(base.to_owned());
    }
    let mut source = base.to_owned();
    replace(
        &mut source,
        "    private val initializationJob = scope.launch {",
        "    private val startupDecodeScope = StartupSnapshotDecodeScope<AppData>()\n    private val initializationJob = scope.launch {",
    )?;
    replace(
        &mut source,
        "    private fun loadVerifiedWorkspace(workspaceKey: String): WorkspaceReadResult {",
        r####"    private fun loadVerifiedWorkspace(workspaceKey: String): WorkspaceReadResult =
        startupDecodeScope.withMemo { loadVerifiedWorkspaceInDecodeScope(workspaceKey) }

    private fun loadVerifiedWorkspaceInDecodeScope(workspaceKey: String): WorkspaceReadResult {"####,
    )?;
    replace(
        &mut source,
        r####"        val rawVersion = rawSchemaVersion(raw)
        val effectiveVersion = listOfNotNull(rawVersion, declaredSchemaVersion).maxOrNull()
            ?: APP_DATA_SCHEMA_VERSION
        val futureVersion = effectiveVersion.takeIf { it > APP_DATA_SCHEMA_VERSION }
        val decoder = if (effectiveVersion == APP_DATA_SCHEMA_VERSION) strictPersistedJson else json
        val decoded = runCatching { decoder.decodeFromString<AppData>(raw) }.getOrNull()"####,
        r####"        val parsed = startupDecodeScope.decode(
            raw, declaredSchemaVersion, APP_DATA_SCHEMA_VERSION, ::rawSchemaVersion
        ) { payload, version ->
            val decoder = if (version == APP_DATA_SCHEMA_VERSION) strictPersistedJson else json
            runCatching { decoder.decodeFromString<AppData>(payload) }.getOrNull()
        }
        val futureVersion = parsed.effectiveVersion.takeIf { it > APP_DATA_SCHEMA_VERSION }
        val decoded = parsed.decoded"####,
    )?;
    source.push_str(DECODE_SCOPE);
    Ok(source)
}

const DECODE_SCOPE: &str = r####"

internal data class StartupDecodedSnapshot<T>(
    val raw: String,
    val rawVersion: Int?,
    val effectiveVersion: Int,
    val decoded: T?
)

// Only the parsed payload is shared. Each caller still supplies its own source,
// authenticated revision/item count, and recovery priority. Exact text and the
// effective schema both participate, so a future declaration cannot inherit a
// supported-schema result. No proof, file stamp, or candidate is cached here.
internal class StartupSnapshotDecodeScope<T> {
    private val active = ThreadLocal<java.util.ArrayDeque<StartupDecodedSnapshot<T>>>()

    fun <R> withMemo(action: () -> R): R {
        if (active.get() != null) return action()
        active.set(java.util.ArrayDeque(2))
        try {
            return action()
        } finally {
            // Release large strings and decoded models on both success and failure.
            active.remove()
        }
    }

    fun decode(
        raw: String,
        declaredVersion: Int?,
        supportedVersion: Int,
        rawVersionOf: (String) -> Int?,
        decodePayload: (String, Int) -> T?
    ): StartupDecodedSnapshot<T> {
        val entries = active.get()
        val sameRaw = entries?.firstOrNull { it.raw == raw }
        val rawVersion = if (sameRaw != null) sameRaw.rawVersion else rawVersionOf(raw)
        val effectiveVersion = listOfNotNull(rawVersion, declaredVersion).maxOrNull()
            ?: supportedVersion
        entries?.firstOrNull { it.raw == raw && it.effectiveVersion == effectiveVersion }
            ?.let { return it }
        val parsed = StartupDecodedSnapshot(raw, rawVersion, effectiveVersion,
            decodePayload(raw, effectiveVersion))
        if (entries != null) {
            if (entries.size == 2) entries.removeFirst()
            entries.addLast(parsed)
        }
        return parsed
    }
}
"####;

pub const TEST_PATH: &str = "com/ofairyo/gridtimer/data/StartupSnapshotDecodeTest.kt";
pub const TEST_CONTENTS: &str = r####"package com.ofairyo.gridtimer.data

import org.junit.Assert.*
import org.junit.Test

class StartupSnapshotDecodeTest {
    @Test fun identicalMirrorsReuseParsingButChangedBytesAndFutureDeclarationsDoNot() {
        val scope = StartupSnapshotDecodeScope<Any>()
        var headers = 0
        var decodes = 0
        fun read(raw: String, declared: Int? = null) = scope.decode(raw, declared, 15,
            { headers++; 15 }, { _, _ -> decodes++; Any() })
        scope.withMemo {
            val primary = read("same")
            assertSame(primary.decoded, read(String(charArrayOf('s', 'a', 'm', 'e')), 15).decoded)
            assertEquals(1, headers)
            assertEquals(1, decodes)
            val future = read("same", 16)
            assertEquals(16, future.effectiveVersion)
            assertNotSame(primary.decoded, future.decoded)
            assertEquals(2, decodes)
            // These texts have the same Java hash; equality must use all bytes.
            val firstCollision = read("Aa")
            assertNotSame(firstCollision.decoded, read("BB").decoded)
            assertEquals(4, decodes)
        }
    }

    @Test fun unknownRawSchemaAndRejectedPayloadsRemainRejectedWithoutReparsing() {
        val scope = StartupSnapshotDecodeScope<String>()
        var headers = 0
        var decodes = 0
        scope.withMemo {
            repeat(3) {
                val parsed = scope.decode("malformed", null, 15,
                    { headers++; null }, { _, _ -> decodes++; null })
                assertNull(parsed.decoded)
                assertEquals(15, parsed.effectiveVersion)
            }
            assertEquals(1, headers)
            assertEquals(1, decodes)
            val future = scope.decode("malformed", 16, 15,
                { error("The exact raw header is already known") }, { _, _ -> decodes++; null })
            assertNull(future.decoded)
            assertEquals(16, future.effectiveVersion)
            assertEquals(2, decodes)
        }
    }

    @Test fun rawFutureSchemaIsNeverLoweredByASupportedDeclaration() {
        val scope = StartupSnapshotDecodeScope<Int>()
        scope.withMemo {
            val first = scope.decode("future", null, 15, { 17 }, { _, version -> version })
            val second = scope.decode("future", 15, 15, { error("unexpected parse") },
                { _, _ -> error("unexpected decode") })
            assertEquals(17, second.effectiveVersion)
            assertEquals(17, second.decoded)
            assertSame(first, second)
        }
    }

    @Test fun onlyTwoResultsAreRetainedAndFailuresCannotLeakThemToTheNextRead() {
        val scope = StartupSnapshotDecodeScope<Any>()
        var decodes = 0
        fun read(raw: String) = scope.decode(raw, null, 15, { 15 }, { _, _ -> decodes++; Any() })
        var first: Any? = null
        assertTrue(runCatching {
            scope.withMemo {
                first = read("a").decoded
                read("b")
                read("c")
                assertNotSame(first, read("a").decoded)
                assertEquals(4, decodes)
                error("aborted workspace read")
            }
        }.isFailure)
        assertNotSame(first, scope.withMemo { read("a").decoded })
        assertEquals(5, decodes)
        assertNotSame(read("a").decoded, read("a").decoded)
        assertEquals(7, decodes)
    }

    @Test fun nestedReadsShareOnlyTheirThreadAndOuterScopeLifetime() {
        val scope = StartupSnapshotDecodeScope<Any>()
        fun read() = scope.decode("same", null, 15, { 15 }, { _, _ -> Any() }).decoded
        var outer: Any? = null
        scope.withMemo {
            outer = read()
            assertSame(outer, scope.withMemo { read() })
            val other = java.util.concurrent.atomic.AtomicReference<Any?>()
            val thread = Thread { other.set(scope.withMemo { read() }) }
            thread.start()
            thread.join(5_000)
            assertFalse(thread.isAlive)
            assertNotNull(other.get())
            assertNotSame(outer, other.get())
            assertSame(outer, read())
        }
        assertNotSame(outer, scope.withMemo { read() })
    }
}
"####;
