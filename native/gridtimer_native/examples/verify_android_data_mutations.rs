// v0.0.1 - Compile isolated production note/finance states and challenge business guards.
use std::{env, fs, path::PathBuf, process::Command};

#[path = "../src/sourcegen/kotlin_sources.rs"]
mod kotlin_sources;
#[path = "../src/sourcegen/android_note_background.rs"]
mod note;
#[path = "../src/sourcegen/android_jvm_test_sources.rs"]
mod tests;

fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let start = text.find(from).expect(from);
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
    let mut friend = String::new();
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
            let copied = output.join(format!("dependency_{}.jar", runtime.len()));
            let bytes = fs::read(&path).unwrap();
            fs::write(&copied, &bytes).unwrap();
            assert_eq!(bytes, fs::read(&copied).unwrap());
            if path.to_string_lossy().contains("runtime_app_classes_jar") {
                friend = copied.to_string_lossy().into_owned();
            }
            copied
        } else {
            path
        };
        runtime.push(path.to_string_lossy().into_owned());
    }
    assert!(!friend.is_empty());
    let runtime = runtime.join(";");
    let screen = kotlin_sources::SOURCES
        .iter()
        .find(|s| s.path.ends_with("/GridTimerScreen.kt"))
        .unwrap()
        .contents;
    let finance = format!("package com.ofairyo.gridtimer.ui\nimport androidx.compose.runtime.*\nimport com.ofairyo.gridtimer.data.FinanceProfile\n{}",
        between(screen, "internal class FinanceProfileCommitState(", "@OptIn(ExperimentalLayoutApi::class)\n@Composable\nprivate fun FinanceSheet("));
    let model = kotlin_sources::SOURCES
        .iter()
        .find(|s| s.path.ends_with("/TimerViewModel.kt"))
        .unwrap()
        .contents;
    let test_methods = between(
        tests::CONTENTS,
        "class TenfoldEditingTest {",
        "    @Test fun publishedPhaseStillRingsOnceWhenItsTransitionListWasConsumed()",
    );
    let fixture = between(
        tests::CONTENTS,
        "    private class NoteBackgroundFixture(",
        "    @Test fun noteBackgroundKeepsOrdinaryPageAcrossHomeAndAppSwitches()",
    );
    let test_count = test_methods.matches("@Test fun ").count();
    let suite = format!("package com.ofairyo.gridtimer.ui\nimport org.junit.Assert.*\nimport org.junit.Test\nimport android.util.Log\nimport com.ofairyo.gridtimer.data.*\nimport com.ofairyo.gridtimer.core.NativeOptimizerBridge\nimport kotlinx.serialization.encodeToString\nimport kotlinx.serialization.decodeFromString\nimport kotlinx.serialization.json.Json\nimport kotlinx.coroutines.*\nimport kotlinx.coroutines.sync.Mutex\nimport kotlinx.coroutines.sync.withLock\n{}\n{}\n}}\n{}",
        test_methods.replace("TenfoldEditingTest", "TenfoldDataStateBoundaryTest"), fixture, tests::finance_repository_fixture());
    let followup = env::args().nth(3).as_deref() == Some("followup");
    let cases = if followup {
        vec![
            "baseline",
            "removed_finance_durable_first",
            "removed_unlock_target_identity",
        ]
    } else {
        vec![
            "baseline",
            "wording_only",
            "removed_unlock_background_guard",
            "removed_unlock_workspace_guard",
            "removed_unlock_ticket_guard",
            "removed_finance_rollback",
            "removed_finance_generation_guard",
            "removed_finance_release",
            "removed_finance_receipt",
            "removed_finance_durable_first",
            "removed_unlock_target_identity",
        ]
    };
    let mut results = Vec::new();
    for case in cases {
        let directory = output.join(case);
        fs::create_dir_all(&directory).unwrap();
        let mut note_source = note::CONTENTS.to_owned();
        let mut finance_source = finance.clone();
        let mut suite_source = suite.clone();
        // The isolated classpath exposes NoteEntry from a dependency module;
        // Kotlin cannot smart-cast its public property across that boundary.
        // The immediately preceding production null check remains unchanged.
        let mut model_source = model.replace(
            "note.encryption.protectionRevision",
            "requireNotNull(note.encryption).protectionRevision",
        );
        match case {
            "wording_only" => {
                model_source = model_source.replace("解锁已取消，请重新输入密码", "请再次输入密码")
            }
            "removed_unlock_background_guard" => {
                note_source =
                    note_source.replace("                onBackground()", "                Unit")
            }
            "removed_unlock_workspace_guard" => {
                note_source =
                    note_source.replace("currentWorkspaceKey == ticket.workspaceKey &&", "true &&")
            }
            "removed_unlock_ticket_guard" => {
                note_source = note_source.replace(
                    "if (pending[key] !== ticket) return false",
                    "if (false) return false",
                )
            }
            "removed_finance_rollback" => {
                finance_source = finance_source.replace("        profile = upstreamProfile", "Unit")
            }
            "removed_finance_generation_guard" => {
                finance_source = finance_source.replace(
                    "if (generation != latestGeneration) return false",
                    "if (false) return false",
                )
            }
            "removed_finance_release" => {
                finance_source = finance_source.replace(
                    "        pendingProfile = null\n        if (committed)",
                    "        if (committed)",
                )
            }
            "removed_finance_receipt" => {
                finance_source = finance_source.replace(
                    "if (committed) upstreamProfile = requireNotNull(committedProfile)",
                    "Unit",
                )
            }
            "removed_finance_durable_first" => {
                suite_source = suite_source
                    .replace("publishAfterDurable = true", "publishAfterDurable = false");
            }
            "removed_unlock_target_identity" => {
                let from = note_source
                    .find("internal fun noteUnlockTargetMatches(")
                    .unwrap();
                note_source.replace_range(from.., "internal fun noteUnlockTargetMatches(requested: NoteEntry, current: NoteEntry?): Boolean = true\n");
            }
            _ => {}
        }
        let paths: Vec<_> = [
            ("NoteBackgroundLockObserver.kt", note_source),
            ("FinanceProfileCommitState.kt", finance_source),
            ("TimerViewModel.kt", model_source),
            ("TenfoldDataStateBoundaryTest.kt", suite_source),
        ]
        .into_iter()
        .filter(|(name, _)| !followup || *name != "TimerViewModel.kt")
        .map(|(name, text)| {
            let path = directory.join(name);
            fs::write(&path, text).unwrap();
            path
        })
        .collect();
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
                "-module-name",
                "app_release",
            ])
            .arg(format!("-Xfriend-paths={friend}"))
            .args(["-classpath", &runtime, "-d"])
            .arg(&classes)
            .args(paths)
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
                "com.ofairyo.gridtimer.ui.TenfoldDataStateBoundaryTest",
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
                log.contains(&format!("OK ({test_count} tests)"))
            } else {
                log.contains("java.lang.AssertionError")
                    && log.contains(&format!("Tests run: {test_count}"))
            },
            "{case}: {log}"
        );
        results.push(serde_json::json!({"case":case,"expectedPass":expected_pass,"passed":true}));
        println!("{case}: expected result verified");
    }
    fs::write(output.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({"tests":test_count,"cases":results,"followup":followup,"productionModified":false,"passed":true})).unwrap()).unwrap();
}
