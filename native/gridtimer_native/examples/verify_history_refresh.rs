// v0.0.1 - Execute production Compose history refresh scopes in isolated JVM cases.
use std::{env, fs, path::PathBuf, process::Command};

fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let start = text.find(from).expect(from) + from.len();
    let end = start + text[start..].find(to).expect(to);
    &text[start..end]
}

fn main() {
    let root = PathBuf::from(env::args_os().nth(1).expect("project root"));
    let output = PathBuf::from(env::args_os().nth(2).expect("isolated output"));
    fs::create_dir_all(&output).unwrap();
    let metadata: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            root.join("release_artifacts/verification/v2.22.42-timer-response/test_classpath.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let compiler = metadata["compiler"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(";");
    let mut runtime = Vec::new();
    for value in metadata["runtime"].as_array().unwrap() {
        let old = value.as_str().unwrap();
        let path = old.split_once("\\app\\build\\").map_or_else(
            || PathBuf::from(old),
            |(_, suffix)| root.join("app/build").join(suffix),
        );
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("jar") {
            continue;
        }
        let path = if path.starts_with(&root) {
            let copy = output.join(format!("dependency_{}.jar", runtime.len()));
            fs::copy(&path, &copy).unwrap();
            copy
        } else {
            path
        };
        runtime.push(path.to_string_lossy().into_owned());
    }
    let runtime = runtime.join(";");
    let plugin = PathBuf::from(env::var_os("USERPROFILE").unwrap()).join(
        ".gradle/caches/modules-2/files-2.1/androidx.compose.compiler/compiler/1.5.14/78d62a968c6d23ded8476540f65b2ef850ddcaa7/compiler-1.5.14.jar");
    let production = include_str!("../src/sourcegen/android_timer_latency.rs");
    let history = between(
        production,
        "const HISTORY_SEARCH: &str = r####\"",
        "\"####;",
    );
    let helper = between(
        production,
        "const ASYNC_COMPUTATION: &str = r####\"",
        "private data class TimerDetailStats(",
    );
    let mut results = Vec::new();
    for case in [
        "baseline",
        "wording_only",
        "removed_refresh_retention",
        "removed_scope_guard",
        "removed_workspace_scope",
    ] {
        let directory = output.join(case);
        fs::create_dir_all(&directory).unwrap();
        let mut history = history.to_owned();
        let mut helper = helper.to_owned();
        match case {
            "removed_refresh_retention" => {
                history = history
                    .replace(", retentionScope = historyWorkspaceScope", "")
                    .replace(",\n        retentionScope = historyFilterScope", "");
            }
            "removed_scope_guard" => {
                helper = helper.replace("result?.scope === retentionScope", "result != null")
            }
            "removed_workspace_scope" => {
                history = history.replace("remember(workspaceKey) { Any() }", "remember { Any() }")
            }
            _ => {}
        }
        let source = format!("{HARNESS}\n{helper}\n@Composable\nprivate fun historyOutput(input: Input): Output {{\nval appData = input.data\nval uiIndex = input.index\nval workspaceKey = input.workspace\nval filterCategoryId = input.category\nval filterSlotId = input.slot\nval trimmedSearchQuery = input.query.trim()\nval summaryNow = input.now\n{history}\nreturn Output(historyLoading, filteredSessionHistory.sessions)\n}}\n");
        if case == "wording_only" {
            // Change the actual production heading in an isolated source copy.
            // This state suite intentionally does not extract or assert UI wording.
            let screen = include_str!("../src/sourcegen/kotlin_sources.rs");
            let modified = screen.replace("text = \"计时记录\"", "text = \"计时历史\"");
            assert_ne!(screen, modified);
            fs::write(directory.join("kotlin_sources.rs"), modified).unwrap();
        }
        let path = directory.join("HistoryRefreshTest.kt");
        fs::write(&path, source).unwrap();
        let trace = directory.join("AndroidTrace.kt");
        // Android tracing is a platform side effect; keep production Compose state intact.
        fs::write(&trace, "package android.os\nobject Trace { @JvmStatic fun beginSection(name: String) {}\n@JvmStatic fun endSection() {} }\n").unwrap();
        let classes = directory.join("classes");
        let compilation = Command::new("C:/tools/java/jdk-17.0.18+8/bin/java.exe")
            .args([
                "-Xmx768m",
                "-cp",
                &compiler,
                "org.jetbrains.kotlin.cli.jvm.K2JVMCompiler",
                "-no-stdlib",
                "-no-reflect",
                "-jvm-target",
                "17",
            ])
            .arg(format!("-Xplugin={}", plugin.display()))
            .args(["-classpath", &runtime, "-d"])
            .arg(&classes)
            .arg(path)
            .arg(trace)
            .output()
            .unwrap();
        fs::write(
            directory.join("compile.log"),
            [&compilation.stdout[..], &compilation.stderr[..]].concat(),
        )
        .unwrap();
        assert!(compilation.status.success(), "{case} compilation failed");
        let execution = Command::new("C:/tools/java/jdk-17.0.18+8/bin/java.exe")
            .args([
                "-Xmx512m",
                "-cp",
                &format!("{};{runtime}", classes.display()),
                "org.junit.runner.JUnitCore",
                "history.audit.HistoryRefreshTest",
            ])
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&execution.stdout).into_owned()
            + &String::from_utf8_lossy(&execution.stderr);
        fs::write(directory.join("tests.log"), &log).unwrap();
        let expected_pass = case == "baseline" || case == "wording_only";
        assert_eq!(execution.status.success(), expected_pass, "{case}: {log}");
        assert!(
            if expected_pass {
                log.contains("OK (4 tests)")
            } else {
                log.contains("java.lang.AssertionError") && log.contains("Tests run: 4")
            },
            "{case}: {log}"
        );
        results.push(serde_json::json!({"case":case,"expectedPass":expected_pass,"passed":true}));
        println!("{case}: expected result verified");
    }
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "tests":4,"cases":results,"productionModifiedByMutations":false,"passed":true
        }))
        .unwrap(),
    )
    .unwrap();
}

const HARNESS: &str = r####"package history.audit
import androidx.compose.runtime.*
import androidx.compose.runtime.snapshots.Snapshot
import kotlinx.coroutines.*
import org.junit.Assert.*
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

private data class Row(val id: Int, val category: String, val slot: Int, val title: String)
private data class Data(val sessions: List<Row>, val archivedTasks: List<Row> = emptyList(), val categories: List<String> = emptyList())
private data class Index(val rows: List<Row>)
private data class History(val sessions: List<Row>)
private val EmptyArchivedSearchIndex = Index(emptyList())
private val EmptySessionSearchIndex = Index(emptyList())
private val EmptyFilteredArchives: List<Row> = ArrayList(0)
private val EmptyFilteredSessionHistory = History(emptyList())
private data class Input(val data: Data, val index: Any = Any(), val workspace: String = "account-a", val category: String? = null, val slot: Int? = null, val query: String = "", val now: Long = 1L)
private data class Output(val loading: Boolean, val rows: List<Row>)
private object ComputationPort {
    @Volatile var barrier: CountDownLatch? = null
    fun await() { check(barrier?.await(8, TimeUnit.SECONDS) != false) { "Test computation did not resume" } }
}
private fun buildArchivedHistorySearchIndex(rows: List<Row>, index: Any): Index { ComputationPort.await(); return Index(rows) }
private fun buildSessionHistorySearchIndex(rows: List<Row>, index: Any): Index { ComputationPort.await(); return Index(rows) }
private fun filterArchivedHistory(index: Index, query: String, category: String?, slot: Int?): List<Row> {
    ComputationPort.await()
    return index.rows.filter { (category == null || it.category == category) && (slot == null || it.slot == slot) && it.title.contains(query) }
}
private fun filterSessionHistory(index: Index, query: String, category: String?, slot: Int?, now: Long): History =
    History(filterArchivedHistory(index, query, category, slot))
private class EmptyApplier : AbstractApplier<Unit>(Unit) {
    override fun insertTopDown(index: Int, instance: Unit) {}
    override fun insertBottomUp(index: Int, instance: Unit) {}
    override fun move(from: Int, to: Int, count: Int) {}
    override fun remove(index: Int, count: Int) {}
    override fun onClear() {}
}
private class Harness(val scope: CoroutineScope) {
    val clock = BroadcastFrameClock()
    val recomposer = Recomposer(scope.coroutineContext + clock)
    val composition = Composition(EmptyApplier(), recomposer)
    var input by mutableStateOf(Input(Data(listOf(Row(1, "a", 4, "first"), Row(2, "b", 5, "second")))))
    val outputs = mutableListOf<Output>()
    val runner = scope.launch(clock) { recomposer.runRecomposeAndApplyChanges() }
    init { composition.setContent { val result = historyOutput(input); SideEffect { outputs.add(result) } } }
    suspend fun pump() {
        Snapshot.sendApplyNotifications()
        repeat(12) { yield(); clock.sendFrame(System.nanoTime()); delay(2) }
    }
    suspend fun ready(ids: List<Int>) {
        withTimeout(4000) { while (outputs.lastOrNull()?.let { !it.loading && it.rows.map(Row::id) == ids } != true) pump() }
    }
    suspend fun close() {
        ComputationPort.barrier?.countDown()
        ComputationPort.barrier = null
        composition.dispose(); recomposer.cancel(); runner.cancelAndJoin()
    }
}
class HistoryRefreshTest {
    @Test fun minuteRefreshKeepsCompletedRows() = runBlocking {
        val h = Harness(this)
        try {
            h.ready(listOf(1, 2))
            ComputationPort.barrier = CountDownLatch(1)
            h.outputs.clear(); h.input = h.input.copy(now = 60_001L); h.pump()
            assertTrue(h.outputs.isNotEmpty())
            assertTrue(h.outputs.all { !it.loading && it.rows.map(Row::id) == listOf(1, 2) })
            ComputationPort.barrier!!.countDown(); h.ready(listOf(1, 2))
        } finally { h.close() }
    }
    @Test fun refreshedIndexesKeepRowsUntilNewRowsAreReady() = runBlocking {
        val h = Harness(this)
        try {
            h.ready(listOf(1, 2))
            ComputationPort.barrier = CountDownLatch(1)
            h.outputs.clear(); h.input = h.input.copy(data = h.input.data.copy(sessions = h.input.data.sessions.drop(1)), index = Any()); h.pump()
            assertTrue(h.outputs.isNotEmpty())
            assertTrue(h.outputs.all { !it.loading && it.rows.map(Row::id) == listOf(1, 2) })
            ComputationPort.barrier!!.countDown(); h.ready(listOf(2))
            assertTrue(h.outputs.all { !it.loading && it.rows.isNotEmpty() })
        } finally { h.close() }
    }
    @Test fun changingEachFilterImmediatelyHidesThePreviousResults() = runBlocking {
        for (change in listOf<(Input) -> Input>({ it.copy(category = "b") }, { it.copy(slot = 5) }, { it.copy(query = "second") })) {
            val h = Harness(this)
            try {
                h.ready(listOf(1, 2))
                ComputationPort.barrier = CountDownLatch(1)
                h.outputs.clear(); h.input = change(h.input); h.pump()
                assertTrue(h.outputs.isNotEmpty())
                assertTrue(h.outputs.all { it.loading && it.rows.isEmpty() })
                ComputationPort.barrier!!.countDown(); h.ready(listOf(2))
            } finally { h.close() }
        }
    }
    @Test fun accountChangeClearsRowsEvenWhenInputReferencesAreIdentical() = runBlocking {
        val h = Harness(this)
        try {
            h.ready(listOf(1, 2))
            ComputationPort.barrier = CountDownLatch(1)
            h.outputs.clear(); h.input = h.input.copy(workspace = "account-b"); h.pump()
            assertTrue(h.outputs.isNotEmpty())
            assertTrue(h.outputs.all { it.loading && it.rows.isEmpty() })
            ComputationPort.barrier!!.countDown(); h.ready(listOf(1, 2))
        } finally { h.close() }
    }
}
"####;
