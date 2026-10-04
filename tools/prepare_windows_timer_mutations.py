"""Prepare isolated, reviewable timer mutations. Never builds or edits product source."""
from pathlib import Path
import argparse
import hashlib
import json
import shutil


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replace_exact(text, old, new):
    if text.count(old) != 1:
        raise RuntimeError(f"Mutation anchor count must be one, got {text.count(old)}: {old[:90]}")
    return text.replace(old, new, 1)


def mutations():
    feedback = """        self.status = match kind {
            TimerActionKind::Start => "正在开始计时",
            TimerActionKind::Pause => "正在暂停计时",
        }
"""
    premature = """        // Deliberate diagnostic fault: publish run state before a durable receipt.
        if let Some(action) = self.persistence.pending_timer_action.as_ref() {
            for slot in self.data.slots.iter_mut().filter(|slot| action.slot_ids.contains(&slot.id)) {
                slot.running_since_epoch_millis = match kind {
                    TimerActionKind::Start => Some(action.requested_at_epoch_millis),
                    TimerActionKind::Pause => None,
                };
            }
        }
"""
    duplicate = """        if self.persistence.pending_timer_action.is_some() {
            // An accepted intention is never replaced by a later click, even
            // while another draft owns the single writer.
            ctx.request_repaint();
            return;
        }
"""
    identity = """    if action.expected_slots.len() != ids.len()
        || action
            .expected_slots
            .iter()
            .map(|expected| expected.slot_id)
            .collect::<HashSet<_>>()
            != ids
        || action.expected_slots.iter().any(|expected| {
            !ids.contains(&expected.slot_id)
                || !slots.iter().any(|slot| expected.matches_slot(slot))
        })
    {
        return Err("计时记录已变化，未提交过期操作".into());
    }
"""
    return [
        {
            "name": "wording_only", "file": "src/desktop/workspace_ui.rs",
            "old": 'Some(TimerActionKind::Start) => "正在开始",',
            "new": 'Some(TimerActionKind::Start) => "正在启动",',
            "expect": "pass",
            "tests": [
                "tests::workspace_timer_intention_waits_for_draft_and_preserves_click_time_once",
                "tests::workspace_timer_failed_background_save_never_displays_a_false_run",
            ],
        },
        {
            "name": "premature_running_before_durable_receipt", "file": "src/desktop/persistence.rs",
            "old": feedback, "new": premature + feedback, "expect": "fail",
            "tests": [
                "tests::workspace_timer_failed_background_save_never_displays_a_false_run",
                "tests::workspace_timer_intention_waits_for_draft_and_preserves_click_time_once",
            ],
        },
        {
            "name": "remove_duplicate_intention_guard", "file": "src/desktop/persistence.rs",
            "old": duplicate, "new": "", "expect": "fail",
            "tests": [
                "tests::workspace_timer_repeat_click_during_save_commits_only_one_transition",
                "tests::workspace_timer_intention_waits_for_draft_and_preserves_click_time_once",
            ],
        },
        {
            "name": "remove_original_run_identity_guard", "file": "src/desktop/persistence.rs",
            "old": identity, "new": "", "expect": "fail",
            "tests": [
                "desktop_persistence::timer_action_worker_tests::timer_action_checks_original_run_identity_even_when_running_state_matches",
            ],
        },
        {
            "name": "remove_workspace_receipt_guard", "file": "src/desktop/sync_apply_worker.rs",
            "old": """        if completion.workspace != self.ai_workspace_identity()
            || completion.version != self.data_version
            || completion
                .task
""",
            "new": """        if completion.version != self.data_version
            || completion
                .task
""",
            "expect": "fail",
            "tests": [
                "tests::sync_apply_stale_receipt_is_never_applied_or_allowed_to_release_timer",
            ],
        },
    ]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default="release_artifacts/verification/windows_v1.1.0.3/timer_mutations")
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    source = repo / "native/gridtimer_native"
    output = (repo / args.output).resolve()
    if not output.is_relative_to(repo / "release_artifacts/verification"):
        raise RuntimeError("Mutation copies must stay within repository verification artifacts")
    if output.exists():
        raise RuntimeError("Keep earlier mutation evidence; use a new output directory")
    specs = mutations()
    # Validate all anchors and test names before writing any mutation checkout.
    rust_sources = [p for p in (source / "src").rglob("*.rs")]
    for spec in specs:
        text = (source / spec["file"]).read_text(encoding="utf-8")
        spec["changed"] = replace_exact(text, spec["old"], spec["new"])
        for exact_test in spec["tests"]:
            marker = "fn " + exact_test.rsplit("::", 1)[-1] + "("
            if not any(marker in path.read_text(encoding="utf-8") for path in rust_sources):
                raise RuntimeError("Missing critical business test: " + exact_test)
    if args.check_only:
        print("TIMER_MUTATION_ANCHORS_AND_TESTS_OK variants=" + str(len(specs)))
        return
    output.mkdir(parents=True)
    assets = ["app/build.gradle", "app/src/main/res/raw/focus_bell.wav", "app/src/main/res/raw/break_bell.wav"]
    source_files = {
        "native/gridtimer_native/" + path.relative_to(source).as_posix(): digest(path)
        for path in source.rglob("*")
        if path.is_file() and not any(part in ("target", ".git") for part in path.relative_to(source).parts)
    }
    source_files.update({relative: digest(repo / relative) for relative in assets})

    def copy_inputs(name):
        destination = output / name / "native/gridtimer_native"
        shutil.copytree(source, destination, ignore=shutil.ignore_patterns("target", ".git"))
        for relative in assets:
            asset = output / name / relative
            asset.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(repo / relative, asset)
        return destination

    unmodified = copy_inputs("unmodified")
    manifest = {
        "formatVersion": 2,
        "purpose": "Synthetic timer business-state mutation verification; compile sequentially with the same release toolchain as the candidate",
        "sourceRoot": str(source), "sourceCargoSha256": digest(source / "Cargo.toml"),
        "sourceLockSha256": digest(source / "Cargo.lock"), "variants": [],
        "sourceFiles": source_files,
        "unmodified": {
            "name": "unmodified", "packageDirectory": str(unmodified),
            "expectedTestOutcome": "pass", "inputFiles": source_files,
            "exactTests": sorted({test for spec in specs for test in spec["tests"]}),
        },
        "rules": [
            "Run the named tests on unmodified candidate first; all must pass.",
            "Build each isolated variant successfully before testing; a compiler error is not a killed mutation.",
            "Run each exact test with --exact --nocapture --test-threads=1 and retain exit code and output.",
            "Wording-only tests must pass. Each removed-state-guard test must produce a Rust assertion failure.",
            "An access violation, timeout, or startup failure is not accepted as a killed business mutation.",
        ],
    }
    for spec in specs:
        destination = copy_inputs(spec["name"])
        target = destination / spec["file"]
        original_hash = digest(target)
        target.write_text(spec["changed"], encoding="utf-8", newline="\n")
        inputs = dict(source_files)
        inputs["native/gridtimer_native/" + spec["file"]] = digest(target)
        manifest["variants"].append({
            "name": spec["name"], "packageDirectory": str(destination),
            "changedFile": spec["file"], "beforeSha256": original_hash,
            "afterSha256": digest(target), "expectedTestOutcome": spec["expect"],
            "exactTests": spec["tests"],
            "inputFiles": inputs,
            "buildArguments": ["test", "--release", "--locked", "--features", "desktop", "--bin", "timer_windows_client", "--no-run"],
            "testArgumentsAfterExecutable": ["<exactTests item>", "--exact", "--nocapture", "--test-threads=1"],
        })
        assert digest(source / spec["file"]) == original_hash, "Product source changed during copy"
    for relative, expected in source_files.items():
        assert digest(repo / relative) == expected, "Build inputs changed during preparation: " + relative
    for spec in [manifest["unmodified"], *manifest["variants"]]:
        copy_root = Path(spec["packageDirectory"]).parents[1]
        for relative, expected in spec["inputFiles"].items():
            assert digest(copy_root / relative) == expected, "Incomplete mutation input copy: " + relative
    (output / "mutation_manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    print(output / "mutation_manifest.json")


if __name__ == "__main__":
    main()
