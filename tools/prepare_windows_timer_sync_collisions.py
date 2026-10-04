"""Prepare two isolated supplemental test builds; never edits the source trees or builds."""
from pathlib import Path
import argparse
import difflib
import hashlib
import json
import shutil


def sha(data):
    return hashlib.sha256(data).hexdigest()


def digest(path):
    return sha(path.read_bytes())


def source_inputs(root):
    package = root / "native/gridtimer_native"
    files = {
        "native/gridtimer_native/" + path.relative_to(package).as_posix(): path
        for path in package.rglob("*")
        if path.is_file() and not any(part in ("target", ".git", "__pycache__") for part in path.relative_to(package).parts)
    }
    for relative in ("app/build.gradle", "app/src/main/res/raw/focus_bell.wav", "app/src/main/res/raw/break_bell.wav"):
        files[relative] = root / relative
    return files


def recipes():
    main = "native/gridtimer_native/src/bin/timer_windows_client.rs"
    gate = lambda number: f"        #[cfg(test)]\n        if timer_sync_collision_network_gate::block({number}) {{\n            return;\n        }}\n"
    return [
        (main, "test_include", '    include!("../desktop/timer_collision_performance_tests.rs");',
         '    include!("../desktop/timer_collision_performance_tests.rs");\n    include!("../desktop/timer_sync_collision_performance_tests.rs");'),
        (main, "cfg_test_gate", "    fn start_sync_task_checked(&mut self, kind: SyncTaskKind, media_preflight_complete: bool) {\n",
         "    fn start_sync_task_checked(&mut self, kind: SyncTaskKind, media_preflight_complete: bool) {\n" + gate(0)),
        (main, "cfg_test_gate", "        phase: SyncTaskPhase,\n    ) {\n",
         "        phase: SyncTaskPhase,\n    ) {\n" + gate(1)),
        (main, "cfg_test_gate", "        if attempts.is_empty() {\n            return;\n        }\n        let attempt_keys = attempts",
         "        if attempts.is_empty() {\n            return;\n        }\n" + gate(3) + "        let attempt_keys = attempts"),
        ("native/gridtimer_native/src/desktop/legal_risk.rs", "cfg_test_gate",
         "        if checkpoint.workspace_id.is_empty() || checkpoint.workspace_capability.is_empty() {\n            return;\n        }\n        let state_path = self.state_path.clone();",
         "        if checkpoint.workspace_id.is_empty() || checkpoint.workspace_capability.is_empty() {\n            return;\n        }\n" + gate(2) + "        let state_path = self.state_path.clone();"),
        ("native/gridtimer_native/src/desktop/timer_action_performance_tests.rs", "test_visibility",
         "    fn synthetic_state(target_mib: usize) -> String {",
         "    pub(super) fn synthetic_state(target_mib: usize) -> String {"),
    ]


def build_changes(files):
    originals = {relative: path.read_bytes() for relative, path in files.items()}
    changed = {}
    reverse = []
    annotations = []
    for relative, purpose, old, new in recipes():
        current = changed.get(relative, originals[relative]).decode("utf-8")
        newline = "\r\n" if b"\r\n" in originals[relative] else "\n"
        old, new = old.replace("\n", newline), new.replace("\n", newline)
        if current.count(old) != 1:
            raise RuntimeError(f"Expected exactly one {purpose} anchor in {relative}: {current.count(old)}")
        if purpose == "cfg_test_gate" and "#[cfg(test)]" not in new:
            raise RuntimeError("Every dispatch gate must compile out of product builds")
        changed[relative] = current.replace(old, new, 1).encode("utf-8")
        reverse.append((relative, old.encode("utf-8"), new.encode("utf-8")))
        annotations.append({"file": relative, "kind": purpose, "anchorSha256": sha(old.encode("utf-8"))})
    main = "native/gridtimer_native/src/bin/timer_windows_client.rs"
    newline = "\r\n" if b"\r\n" in originals[main] else "\n"
    suffix = (newline + '#[cfg(test)]' + newline + 'include!("../desktop/timer_sync_collision_network_gate.rs");' + newline).encode("utf-8")
    changed[main] += suffix
    annotations.append({"file": main, "kind": "cfg_test_module_include"})
    reconstructed = dict(changed)
    assert reconstructed[main].endswith(suffix)
    reconstructed[main] = reconstructed[main][:-len(suffix)]
    for relative, old, new in reversed(reverse):
        assert reconstructed[relative].count(new) == 1
        reconstructed[relative] = reconstructed[relative].replace(new, old, 1)
    assert all(reconstructed[relative] == originals[relative] for relative in changed), "Diagnostic additions must reverse to exact source bytes"
    # This includes the existing shared timing probes, never injecting candidate
    # production behavior into the baseline.
    persistence = originals["native/gridtimer_native/src/desktop/persistence.rs"].decode("utf-8")
    for field in ("submitted", "worker_started", "transform_completed", "save_completed", "receipt_ready", "receipt_applied"):
        assert f"pub {field}: Option<Instant>" in persistence, f"Missing pre-existing shared probe: {field}"
    return originals, changed, annotations


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", default="release_artifacts/verification/windows_v1.1.0.3/timer_sync_supplemental_1")
    parser.add_argument("--baseline", default="release_artifacts/verification/windows_v1.1.0.3/baseline_source")
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    output = (repo / args.output).resolve()
    baseline = (repo / args.baseline).resolve()
    if not output.is_relative_to(repo / "release_artifacts/verification"):
        raise RuntimeError("Supplemental source copies must stay within verification artifacts")
    if output.exists():
        raise RuntimeError("Evidence is append-only; select a new output directory")
    templates = {
        "native/gridtimer_native/src/desktop/timer_sync_collision_performance_tests.rs": repo / "tools/timer_sync_collision_performance_tests.rs",
        "native/gridtimer_native/src/desktop/timer_sync_collision_network_gate.rs": repo / "tools/timer_sync_collision_network_gate.rs",
    }
    template_bytes = {relative: path.read_bytes() for relative, path in templates.items()}
    sources = {}
    for name, root in (("baseline", baseline), ("candidate", repo)):
        files = source_inputs(root)
        originals, changed, annotations = build_changes(files)
        if set(templates) & set(files):
            raise RuntimeError("The source trees must not already contain supplemental files")
        sources[name] = (root, files, originals, changed, annotations)
    if args.check_only:
        print("TIMER_SYNC_SUPPLEMENTAL_ANCHORS_OK variants=2 productionSourceUnchangedVerified=true")
        return
    output.mkdir(parents=True)
    manifest = {
        "formatVersion": 1,
        "purpose": "Supplemental synthetic sync completion collision; separate baseline/candidate binaries, never OS-input or network timing",
        "exactTest": "tests::timer_sync_collision_performance_tests::timer_sync_receipt_collision_release_benchmark",
        "sharedProbeSha256": digest(templates[next(iter(templates))]),
        "gateSha256": digest(templates[list(templates)[1]]),
        "sharedProbeRelativePath": list(templates)[0],
        "gateRelativePath": list(templates)[1],
        "prepareScriptSha256": digest(Path(__file__)),
        "productionSourceUnchangedVerified": True,
        "sources": {},
        "buildArguments": ["test", "--release", "--locked", "--features", "desktop", "--bin", "timer_windows_client", "--no-run"],
        "pathRules": "sourceFiles paths are relative to sourceRoot; inputFiles paths are relative to isolatedRoot; patchFile is absolute",
        "rules": [
            "Build sequentially; use different captured executable paths and record both executable SHA-256 values.",
            "Touch isolated src/lib.rs and src/bin/timer_windows_client.rs before each build when sharing a Cargo target; verify source bytes are unchanged.",
            "Only cfg(test) gates suppress new outbound dispatch; SyncTaskResult validation, merge, journal/checkpoint persistence and receipt application are original production code.",
            "The candidate product tree and baseline source tree remain byte-for-byte unchanged by preparation.",
            "Run both caseMiB 1 and 68, each with 30 starts and 30 pauses, in isolated LOCALAPPDATA and workspace paths.",
            "Do not combine these supplemental executable identities with the earlier core benchmark identities.",
        ],
    }
    for name, (root, files, originals, changed, annotations) in sources.items():
        isolated = output / name
        for relative, source in files.items():
            target = isolated / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
        patches = []
        for relative, new_bytes in {**changed, **template_bytes}.items():
            target = isolated / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(new_bytes)
            old_bytes = originals.get(relative, b"")
            patch = "".join(difflib.unified_diff(
                old_bytes.decode("utf-8").splitlines(keepends=True),
                new_bytes.decode("utf-8").splitlines(keepends=True),
                fromfile="source/" + relative, tofile="isolated/" + relative,
            ))
            patch_path = output / "patches" / name / (relative.replace("/", "__") + ".patch")
            patch_path.parent.mkdir(parents=True, exist_ok=True)
            patch_path.write_text(patch, encoding="utf-8", newline="\n")
            patches.append({"file": relative, "beforeSha256": sha(old_bytes) if relative in originals else None,
                            "afterSha256": sha(new_bytes), "patchFile": str(patch_path), "patchSha256": digest(patch_path)})
        source_hashes = {relative: sha(data) for relative, data in originals.items()}
        input_hashes = dict(source_hashes)
        input_hashes.update({relative: sha(data) for relative, data in {**changed, **template_bytes}.items()})
        for relative, expected in source_hashes.items():
            assert digest(root / relative) == expected, "Source changed while preparing: " + relative
        for relative, expected in input_hashes.items():
            assert digest(isolated / relative) == expected, "Incomplete isolated copy: " + relative
        manifest["sources"][name] = {
            "sourceRoot": str(root), "isolatedRoot": str(isolated),
            "packageDirectory": str(isolated / "native/gridtimer_native"),
            "sourceFiles": source_hashes, "inputFiles": input_hashes,
            "patches": patches, "instrumentation": annotations,
            "productionSourceUnchangedVerified": True,
        }
    for name, (_, files, originals, _, _) in sources.items():
        assert all(path.read_bytes() == originals[relative] for relative, path in files.items()), name + " source changed"
    manifest_path = output / "sync_collision_manifest.json"
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    print(manifest_path)


if __name__ == "__main__":
    main()
