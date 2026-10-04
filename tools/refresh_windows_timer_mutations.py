"""Refresh a never-executed format-2 preparation, retaining each prior revision.

No Cargo, no product edits, no asset recopy, and no file deletion. The tool only
updates changed source inputs in already prepared copies and republishes their
complete hash manifest after validation. A partial refresh fails closed.
"""
from pathlib import Path
import argparse
import copy
import datetime
import difflib
import hashlib
import json
import os

from prepare_windows_timer_mutations import mutations, replace_exact


def sha(data):
    return hashlib.sha256(data).hexdigest()


def checked_path(root, relative):
    result = (root / relative).resolve()
    if not result.is_relative_to(root) or result == root:
        raise RuntimeError("Path escaped its prepared root: " + relative)
    return result


def atomic_replace(path, data):
    temporary = path.with_name(path.name + ".refresh_writing")
    if temporary.exists():
        raise RuntimeError("Prior incomplete refresh temporary is retained: " + str(temporary))
    temporary.write_bytes(data)
    os.replace(temporary, path)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--prepared", required=True)
    parser.add_argument("--check-only", action="store_true")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[1]
    prepared = (repo / args.prepared).resolve()
    if not prepared.is_relative_to(repo / "release_artifacts/verification"):
        raise RuntimeError("Preparation must be inside repository verification artifacts")
    manifest_path = prepared / "mutation_manifest.json"
    old_bytes = manifest_path.read_bytes()
    old = json.loads(old_bytes)
    previous_revision = old.get("revision", 1)
    if old.get("formatVersion") != 2 or type(previous_revision) is not int or previous_revision < 1:
        raise RuntimeError("Only a versioned format-2 preparation can be refreshed")
    next_revision = previous_revision + 1
    if Path(old["sourceRoot"]).resolve() != repo / "native/gridtimer_native":
        raise RuntimeError("Preparation belongs to a different product source")
    specs = {spec["name"]: spec for spec in mutations()}
    if {item["name"] for item in old["variants"]} != set(specs):
        raise RuntimeError("Prepared mutation set differs from the current definitions")
    variants = [old["unmodified"], *old["variants"]]
    allowed_children = {item["name"] for item in variants} | {"mutation_manifest.json"}
    if previous_revision > 1:
        allowed_children |= {f"mutation_manifest_revision{number}.json" for number in range(1, previous_revision + 1)}
        allowed_children |= {f"refresh_revision_{number}" for number in range(2, previous_revision + 1)}
    actual_children = {child.name for child in prepared.iterdir()}
    if actual_children != allowed_children:
        raise RuntimeError("Preparation is not pristine and unexecuted; unexpected files: " + str(actual_children - allowed_children))
    for forbidden in ["execution", "build_workspace", "build_workspace_active.json", "run_manifest.json"]:
        if (prepared / forbidden).exists():
            raise RuntimeError("Execution evidence exists; refusing refresh: " + forbidden)
    if previous_revision > 1:
        retained = (prepared / f"mutation_manifest_revision{previous_revision}.json").read_bytes()
        if retained != old_bytes:
            raise RuntimeError("Current manifest differs from its retained revision")
        for number in range(2, previous_revision + 1):
            revision_bytes = (prepared / f"mutation_manifest_revision{number}.json").read_bytes()
            revision = json.loads(revision_bytes)
            preceding_name = f"mutation_manifest_revision{number - 1}.json"
            preceding_bytes = (prepared / preceding_name).read_bytes()
            expected_chain = {"path": preceding_name, "sha256": sha(preceding_bytes)}
            if revision.get("revision") != number or revision.get("supersedesManifest") != expected_chain:
                raise RuntimeError("Retained manifest revision chain is inconsistent")
            completion = json.loads((prepared / f"refresh_revision_{number}/refresh_complete.json").read_bytes())
            if (completion.get("verified") is not True or
                    completion.get(f"revision{number}ManifestSha256") != sha(revision_bytes) or
                    completion.get("previousManifestSha256") != sha(preceding_bytes)):
                raise RuntimeError("Retained refresh evidence does not match its manifests")

    verified_old_inputs = 0
    for item in variants:
        variant_root = checked_path(prepared, item["name"])
        expected_package = variant_root / "native/gridtimer_native"
        if Path(item["packageDirectory"]).resolve() != expected_package:
            raise RuntimeError("Unexpected package location: " + item["name"])
        existing = {path.relative_to(variant_root).as_posix() for path in variant_root.rglob("*") if path.is_file()}
        if existing != set(item["inputFiles"]):
            raise RuntimeError("Unexpected/missing files in prepared variant: " + item["name"])
        for relative, expected in item["inputFiles"].items():
            if sha(checked_path(variant_root, relative).read_bytes()) != expected:
                raise RuntimeError("Old prepared input changed: " + item["name"] + "/" + relative)
            verified_old_inputs += 1
        if item["name"] != "unmodified":
            spec = specs[item["name"]]
            if item["changedFile"] != spec["file"] or item["exactTests"] != spec["tests"] or item["expectedTestOutcome"] != spec["expect"]:
                raise RuntimeError("Mutation definition changed: " + item["name"])
            old_source = checked_path(prepared / "unmodified/native/gridtimer_native", spec["file"]).read_text(encoding="utf-8")
            reconstructed = replace_exact(old_source, spec["old"], spec["new"]).encode("utf-8")
            if sha(reconstructed) != item["afterSha256"]:
                raise RuntimeError("Old mutation cannot be reproduced exactly: " + item["name"])
    if old["unmodified"]["inputFiles"] != old["sourceFiles"]:
        raise RuntimeError("Old pristine input manifest disagrees with the source manifest")

    package = repo / "native/gridtimer_native"
    current_names = {
        "native/gridtimer_native/" + path.relative_to(package).as_posix()
        for path in package.rglob("*")
        if path.is_file() and not any(part in ("target", ".git") for part in path.relative_to(package).parts)
    } | {"app/build.gradle", "app/src/main/res/raw/focus_bell.wav", "app/src/main/res/raw/break_bell.wav"}
    if current_names != set(old["sourceFiles"]):
        raise RuntimeError("Input names changed; this narrow refresh cannot add or remove files")
    current = {relative: checked_path(repo, relative).read_bytes() for relative in old["sourceFiles"]}
    hashes = {relative: sha(data) for relative, data in current.items()}
    changed = [relative for relative, digest in hashes.items() if digest != old["sourceFiles"][relative]]
    if not changed:
        raise RuntimeError("No changed source inputs to refresh")
    if any(not relative.startswith("native/gridtimer_native/src/") or not relative.endswith(".rs") for relative in changed):
        raise RuntimeError("Only existing Rust source files can be refreshed; assets/manifests must remain unchanged")

    updated = copy.deepcopy(old)
    updated["revision"] = next_revision
    updated["sourceFiles"] = hashes
    updated["sourceCargoSha256"] = hashes["native/gridtimer_native/Cargo.toml"]
    updated["sourceLockSha256"] = hashes["native/gridtimer_native/Cargo.lock"]
    updated["supersedesManifest"] = {"path": f"mutation_manifest_revision{previous_revision}.json", "sha256": sha(old_bytes)}
    writes = []
    for item in [updated["unmodified"], *updated["variants"]]:
        desired = dict(current)
        inputs = dict(hashes)
        if item["name"] != "unmodified":
            spec = specs[item["name"]]
            relative = "native/gridtimer_native/" + spec["file"]
            changed_text = replace_exact(checked_path(repo, relative).read_text(encoding="utf-8"), spec["old"], spec["new"])
            desired[relative] = changed_text.encode("utf-8")
            inputs[relative] = sha(desired[relative])
            item["beforeSha256"] = hashes[relative]
            item["afterSha256"] = inputs[relative]
        item["inputFiles"] = inputs
        previous = next(value for value in variants if value["name"] == item["name"])
        for relative, digest in inputs.items():
            if digest != previous["inputFiles"][relative]:
                writes.append({"variant": item["name"], "path": relative,
                               "beforeSha256": previous["inputFiles"][relative], "afterSha256": digest,
                               "data": desired[relative]})
    if {entry["path"] for entry in writes} != set(changed):
        raise RuntimeError("Reapplying a mutation changed unrelated inputs")
    report = {
        "revision": next_revision, "previousManifestSha256": sha(old_bytes),
        "preparedDirectory": str(prepared), "verifiedOldInputCount": verified_old_inputs,
        "sourceChanges": [{"path": relative, "beforeSha256": old["sourceFiles"][relative], "afterSha256": hashes[relative], "newBytes": len(current[relative])} for relative in changed],
        "copyUpdates": [{key: value for key, value in entry.items() if key != "data"} for entry in writes],
        "retainedAssetCopies": True, "productSourceEdited": False, "cargoInvoked": False,
        "toolSha256": sha(Path(__file__).read_bytes()),
    }
    print(json.dumps({"sourceChanges": report["sourceChanges"], "copyUpdates": len(writes), "verifiedOldInputCount": verified_old_inputs}, indent=2), flush=True)
    if args.check_only:
        print("UNEXECUTED_MUTATION_REFRESH_CHECK_OK", flush=True)
        return

    evidence = prepared / f"refresh_revision_{next_revision}"
    evidence.mkdir()
    retained_old = prepared / f"mutation_manifest_revision{previous_revision}.json"
    if not retained_old.exists():
        retained_old.write_bytes(old_bytes)
    (evidence / "mutation_definitions.json").write_text(json.dumps(list(specs.values()), ensure_ascii=False, indent=2), encoding="utf-8")
    for relative in changed:
        old_source = checked_path(prepared / "unmodified", relative).read_bytes()
        backup = evidence / "source_before" / relative
        backup.parent.mkdir(parents=True, exist_ok=True)
        backup.write_bytes(old_source)
        diff = "".join(difflib.unified_diff(
            old_source.decode("utf-8").splitlines(True), current[relative].decode("utf-8").splitlines(True),
            fromfile=f"revision{previous_revision}/" + relative, tofile=f"revision{next_revision}/" + relative))
        (evidence / (Path(relative).name + ".diff")).write_text(diff, encoding="utf-8", newline="\n")
    (evidence / "refresh_plan.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    for entry in writes:
        path = checked_path(checked_path(prepared, entry["variant"]), entry["path"])
        if sha(path.read_bytes()) != entry["beforeSha256"]:
            raise RuntimeError("Prepared source changed during refresh: " + str(path))
        atomic_replace(path, entry["data"])
    verified_new_inputs = 0
    for item in [updated["unmodified"], *updated["variants"]]:
        for relative, expected in item["inputFiles"].items():
            if sha(checked_path(prepared / item["name"], relative).read_bytes()) != expected:
                raise RuntimeError("Refreshed input hash mismatch: " + item["name"] + "/" + relative)
            verified_new_inputs += 1
    for relative, expected in hashes.items():
        if sha(checked_path(repo, relative).read_bytes()) != expected:
            raise RuntimeError("Frozen product source changed during refresh: " + relative)
    updated["refreshedAtUtc"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    updated["refreshEvidence"] = f"refresh_revision_{next_revision}/refresh_complete.json"
    new_bytes = json.dumps(updated, ensure_ascii=False, indent=2).encode("utf-8")
    (prepared / f"mutation_manifest_revision{next_revision}.json").write_bytes(new_bytes)
    report.update({"verifiedNewInputCount": verified_new_inputs, "completedAtUtc": updated["refreshedAtUtc"],
                   f"revision{next_revision}ManifestSha256": sha(new_bytes), "verified": True})
    (evidence / "refresh_complete.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    atomic_replace(manifest_path, new_bytes)
    print(f"UNEXECUTED_MUTATION_REFRESH_REVISION{next_revision}_VERIFIED", flush=True)
    print(manifest_path, flush=True)
    print("SHA256 " + sha(new_bytes), flush=True)


if __name__ == "__main__":
    main()
