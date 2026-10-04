"""Bind supplemental sync-collision test executables to verified source and build logs.

Read-only for source, binaries and logs. The optional new output manifest is
created exclusively; this tool never builds, runs a benchmark or modifies inputs.
"""
from pathlib import Path
import argparse
import datetime
import hashlib
import json
import re

EXACT_TEST = "tests::timer_sync_collision_performance_tests::timer_sync_receipt_collision_release_benchmark"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def child(root, relative):
    path = (root / relative).resolve()
    require(path != root and path.is_relative_to(root), "Source input escaped its declared root: " + relative)
    return path


def map_sha(values):
    return hashlib.sha256(json.dumps(values, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def verify_source(manifest_path):
    from prepare_windows_timer_sync_collisions import build_changes, source_inputs

    source = read(manifest_path)
    require(source.get("exactTest") == EXACT_TEST, "Wrong supplemental exact test")
    require(source.get("productionSourceUnchangedVerified") is True, "Missing production-source preservation proof")
    require(set(source.get("sources", {})) == {"baseline", "candidate"}, "Missing isolated source pair")
    prepare_script = Path(__file__).with_name("prepare_windows_timer_sync_collisions.py")
    require(source.get("prepareScriptSha256") == sha(prepare_script), "Supplemental preparation implementation changed")
    templates = {
        source["sharedProbeRelativePath"]: Path(__file__).with_name("timer_sync_collision_performance_tests.rs"),
        source["gateRelativePath"]: Path(__file__).with_name("timer_sync_collision_network_gate.rs"),
    }
    require(sha(templates[source["sharedProbeRelativePath"]]) == source["sharedProbeSha256"]
            and sha(templates[source["gateRelativePath"]]) == source["gateSha256"], "Shared test instrumentation changed")
    identities = {}
    for variant in ("baseline", "candidate"):
        item = source["sources"][variant]
        origin = Path(item["sourceRoot"]).resolve(strict=True)
        isolated = Path(item["isolatedRoot"]).resolve(strict=True)
        package = Path(item["packageDirectory"]).resolve(strict=True)
        require(package.is_relative_to(isolated) and isolated != origin, "Source is not an isolated package")
        require(item.get("productionSourceUnchangedVerified") is True, "Variant lacks source-preservation proof")
        for root, field in ((origin, "sourceFiles"), (isolated, "inputFiles")):
            require(type(item[field]) is dict and item[field], "Missing input hash map")
            for relative, expected in item[field].items():
                require(sha(child(root, relative)) == expected.lower(), "Source hash mismatch: " + variant + "/" + relative)
        originals, changes, annotations = build_changes(source_inputs(origin))
        expected_inputs = {relative: hashlib.sha256(value).hexdigest() for relative, value in originals.items()}
        require(expected_inputs == item["sourceFiles"], "Source inventory differs from the prepared inputs")
        expected_inputs.update({relative: hashlib.sha256(value).hexdigest() for relative, value in changes.items()})
        expected_inputs.update({relative: sha(path) for relative, path in templates.items()})
        require(expected_inputs == item["inputFiles"] and annotations == item["instrumentation"],
                "Isolated changes do not reconstruct exactly from approved test-only recipes")
        require({patch["file"] for patch in item["patches"]} == set(changes) | set(templates)
                and len(item["patches"]) == len(changes) + len(templates), "Missing or duplicate patch proof")
        for patch in item["patches"]:
            require(patch["beforeSha256"] == item["sourceFiles"].get(patch["file"])
                    and patch["afterSha256"] == item["inputFiles"][patch["file"]], "Patch before/after identity mismatch")
            patch_path = Path(patch["patchFile"])
            if not patch_path.is_absolute():
                patch_path = child(manifest_path.parent.resolve(), patch["patchFile"])
            require(sha(patch_path) == patch["patchSha256"].lower(), "Patch proof hash mismatch")
        identities[variant] = {
            "sourceRoot": str(origin), "isolatedRoot": str(isolated), "packageDirectory": str(package),
            "productionInputMapSha256": map_sha(item["sourceFiles"]),
            "isolatedInputMapSha256": map_sha(item["inputFiles"]),
        }
    for field in ("sharedProbeSha256", "gateSha256"):
        require(re.fullmatch(r"[0-9a-fA-F]{64}", source.get(field, "")), "Missing shared instrumentation identity: " + field)
    return identities, source


def verify_build(path, check_sources=True):
    build = read(path)
    require(build.get("formatVersion") == 1 and build.get("exactTest") == EXACT_TEST,
            "Wrong supplemental build identity format")
    provenance = Path(build["sourceProvenance"]["path"]).resolve(strict=True)
    require(sha(provenance) == build["sourceProvenance"]["sha256"], "Supplemental source manifest changed")
    if check_sources:
        identities, source = verify_source(provenance)
        require(identities == build["sources"], "Supplemental source-map identities changed")
        require(source["sharedProbeSha256"] == build["sharedProbeSha256"] and source["gateSha256"] == build["gateSha256"],
                "Supplemental instrumentation identities changed")
    require(set(build.get("binaries", {})) == {"baseline", "candidate"}, "Missing supplemental binary pair")
    for variant in ("baseline", "candidate"):
        item = build["binaries"][variant]
        for pair in (item, item["buildLog"]):
            require(sha(Path(pair["path"]).resolve(strict=True)) == pair["sha256"], "Supplemental binary/build log changed")
    require(build["binaries"]["baseline"]["sha256"] != build["binaries"]["candidate"]["sha256"], "Identical supplemental binaries")
    return build


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-build", type=Path)
    parser.add_argument("--source-provenance", type=Path)
    parser.add_argument("--baseline-exe", type=Path)
    parser.add_argument("--candidate-exe", type=Path)
    parser.add_argument("--baseline-build-log", type=Path)
    parser.add_argument("--candidate-build-log", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.verify_build:
        result = verify_build(args.verify_build.resolve(strict=True))
        print(json.dumps({"verified": True, "buildProvenanceSha256": sha(args.verify_build), "binaries": result["binaries"]}))
        return
    require(all((args.source_provenance, args.baseline_exe, args.candidate_exe,
                 args.baseline_build_log, args.candidate_build_log, args.output)), "Supply both executables, both build logs, source provenance and a new output path")
    manifest = args.source_provenance.resolve(strict=True)
    identities, source = verify_source(manifest)
    binaries = {}
    for variant in ("baseline", "candidate"):
        executable = getattr(args, variant + "_exe").resolve(strict=True)
        log = getattr(args, variant + "_build_log").resolve(strict=True)
        text = log.read_text(encoding="utf-8-sig")
        normalized = text.replace("\\\\", "\\").replace("\\", "/").lower()
        package = identities[variant]["packageDirectory"].replace("\\", "/").lower()
        require(package in normalized, "Build log does not identify its isolated source package: " + variant)
        require(re.search(r"Finished.*release.*optimized", text) and "timer_windows_client" in text
                and "Executable" in text, "Build log does not show a successful release client test build: " + variant)
        require(executable.suffix.lower() == ".exe", "Expected supplemental test executable")
        binaries[variant] = {"path": str(executable), "sha256": sha(executable),
                             "buildLog": {"path": str(log), "sha256": sha(log)}}
    require(binaries["baseline"]["sha256"] != binaries["candidate"]["sha256"], "Identical supplemental binaries")
    result = {"formatVersion": 1, "recordedAtUtc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "exactTest": EXACT_TEST, "sourceProvenance": {"path": str(manifest), "sha256": sha(manifest)},
              "sharedProbeSha256": source["sharedProbeSha256"], "gateSha256": source["gateSha256"],
              "sources": identities, "binaries": binaries,
              "scope": "Supplemental test instrumentation only. Binary identity differs from the core benchmark by design; source/patch proof binds the production inputs."}
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(result, output, ensure_ascii=False, indent=2)
    print(json.dumps({"path": str(args.output.resolve()), "sha256": sha(args.output)}))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, TypeError) as error:
        raise SystemExit("SUPPLEMENTAL_PROVENANCE_REJECTED: " + str(error))
