"""Validate final timer evidence and print a read-only, identity-bound JSON summary.

No build, process launch, network, fixture access, or modification of input files.
Example: python tools/summarize_windows_timer_latency.py --run <synthetic_dir>
  --run <collision_dir> --run <native_dir> --baseline-sha256 <sha>
  --candidate-sha256 <sha> [--target synthetic:committedFrameMicros:100]
Targets are explicit candidate P95 ceilings in milliseconds, never assumed.
"""
from pathlib import Path
import argparse
from collections import Counter
import hashlib
import json
import math
import re
import sys


TESTS = {
    "tests::timer_action_performance_tests::timer_action_end_to_end_release_benchmark": "synthetic",
    "tests::timer_collision_performance_tests::timer_autosave_collision_release_benchmark": "collision",
    "tests::timer_native_latency_tests::timer_native_window_end_to_end_benchmark": "native",
}
COMMON = [
    "dispatchReturnMicros", "acceptedMicros", "submittedMicros", "workerStartedMicros",
    "transformCompletedMicros", "saveCompletedMicros", "receiptReadyMicros", "receiptAppliedMicros",
    "prepareMainThreadMicros", "workerQueueMicros", "workerTransformMicros", "durableSaveMicros",
    "workerPostprocessMicros", "receiptDeliveryAndApplyMicros",
]
METRICS = {
    "synthetic": COMMON + ["feedbackFrameMicros", "committedFrameMicros", "committedFrameWorkMicros", "peakFrameMicros"],
    "native": COMMON + ["feedbackUpdateReturnMicros", "feedbackRenderedUpperBoundMicros",
                         "confirmedUpdateReturnMicros", "confirmedRenderedUpperBoundMicros", "confirmedFrameWorkMicros"],
    "collision": [
        "firstDispatchReturnMicros", "firstFrameMicros", "predecessorGateReleasedMicros",
        "predecessorObservedFrameMicros", "retryDispatchMicros", "retryFirstFrameMicros",
        "firstClickCommittedMicros", "retryClickCommittedMicros", "initialClickThroughExplicitRetryMicros",
        "acceptedMicros", "committedFrameWorkMicros", "acceptedToSubmittedMicros", "workerQueueMicros",
        "workerTransformMicros", "durableSaveMicros", "workerPostprocessMicros", "receiptDeliveryAndApplyMicros",
        "acceptedClickSaveCompletedMicros", "acceptedClickReceiptAppliedMicros",
    ],
}
NULLABLE_COLLISION = {"retryDispatchMicros", "retryFirstFrameMicros", "firstClickCommittedMicros", "retryClickCommittedMicros"}
FULL_CASES = {"synthetic": [0, 1, 8, 32, 68], "collision": [1, 68], "native": [68]}
BASIS = {
    "synthetic": "Programmatic dispatch inside egui Context::run; production frame pump and workspace rendering; no OS mouse input, native GPU submission or monitor presentation.",
    "collision": "Synthetic production frame pump, real isolated autosave, at least 100 ms deliberate SQLite write-lock gate; first rejection and later explicit retry remain separate.",
    "native": "Programmatic dispatch and full production eframe::App::update in a native GPU window; screenshot event arrival is an upper bound including GPU readback/event delivery, not OS mouse or monitor scanout latency.",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8-sig"),
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError("Nonfinite JSON: " + value)))


def number(obj, name, nullable=False, integer=False):
    require(name in obj, "Missing metric: " + name)
    value = obj[name]
    if nullable and value is None:
        return None
    require(type(value) in (int, float), "Metric is not numeric: " + name)
    require(math.isfinite(value) and value >= 0, "Metric is negative or nonfinite: " + name)
    require(not integer or type(value) is int, "Metric is not an integer: " + name)
    return value


def boolean(obj, name):
    require(name in obj and type(obj[name]) is bool, "Missing/nonboolean field: " + name)
    return obj[name]


def markers(text, marker):
    return [json.loads(line[len(marker) + 1:]) for line in text.splitlines() if line.startswith(marker + " ")]


def distribution(values):
    ordered = sorted(values)
    if not ordered:
        return {"observationCount": 0, "p50": None, "p95": None, "maximum": None}
    return {"observationCount": len(ordered), "p50": ordered[math.ceil(len(ordered) * .50) - 1],
            "p95": ordered[math.ceil(len(ordered) * .95) - 1], "maximum": ordered[-1]}


def validate_memory(process, key):
    require(key in process, "Missing final-run memory telemetry: " + key)
    snapshot = process[key]
    available = boolean(snapshot, "available")
    require(snapshot.get("queryStartedAtUtc") and snapshot.get("capturedAtUtc"), "Missing memory capture times")
    if available:
        total = number(snapshot, "totalVisibleMemoryBytes", integer=True)
        free = number(snapshot, "freePhysicalMemoryBytes", integer=True)
        require(0 < total and free <= total, "Invalid OS memory values")
    else:
        require(snapshot.get("totalVisibleMemoryBytes") is None and snapshot.get("freePhysicalMemoryBytes") is None
                and snapshot.get("error"), "Unavailable memory must be null with an error")
    return snapshot


def validate_samples(samples, kind, variant, case):
    require(type(samples) is list and len(samples) == 60, "Expected 60 raw samples")
    for sample in samples:
        require(sample.get("variant") == variant and sample.get("caseMiB") == case, "Cross-mixed sample identity")
        number(sample, "caseMiB", integer=True)
        number(sample, "sample", integer=True)
        require(sample.get("action") in ("start", "pause"), "Invalid action")
        for metric in METRICS[kind]:
            number(sample, metric, nullable=kind == "collision" and metric in NULLABLE_COLLISION)
        if kind == "collision":
            accepted = boolean(sample, "firstDispatchAccepted")
            require(number(sample, "dispatchCount", integer=True) == (1 if accepted else 2), "Dispatch/retry count mismatch")
            for field in ("retryDispatchMicros", "retryFirstFrameMicros", "retryClickCommittedMicros"):
                require((sample[field] is None) == accepted, "Retry metrics disagree with first acceptance")
            require((sample["firstClickCommittedMicros"] is not None) == accepted, "Rejected first click cannot claim completion")
            require(boolean(sample, "queuedBehindPredecessor") == accepted, "Queue status disagrees with first acceptance")
            require(boolean(sample, "firstPendingFeedbackVisible") == accepted, "Feedback status disagrees with acceptance")
            require(sample["predecessorGateReleasedMicros"] >= 100_000, "Missing deliberate 100 ms collision gate")
        else:
            number(sample, "actionId", integer=True)
            number(sample, "stateBytes", integer=True)
            number(sample, "journalBytes", integer=True)
            boolean(sample, "feedbackVisible")
            if kind == "synthetic":
                require(number(sample, "savedStateBytes", integer=True) <= 32 * 1024 * 1024, "Managed mirror size exceeded")
            else:
                boolean(sample, "feedbackWasPending")
            clock = [sample[key] for key in ("acceptedMicros", "submittedMicros", "workerStartedMicros",
                     "transformCompletedMicros", "saveCompletedMicros", "receiptReadyMicros", "receiptAppliedMicros")]
            require(clock == sorted(clock), "Invalid stage timestamp order")
    for action in ("start", "pause"):
        group = [sample for sample in samples if sample["action"] == action]
        require(len(group) == 30 and {sample["sample"] for sample in group} == set(range(30)), "Missing or duplicate action ordinal")
    if kind != "collision":
        require(len({sample["actionId"] for sample in samples}) == 60, "Duplicate timer action ID")
        require(len({(sample["stateBytes"], sample["journalBytes"]) for sample in samples}) == 1, "Fixture identity changed inside a run")


def inspect_run(directory, expected_hashes, binary_cache):
    candidates = [directory / name for name in ("timer_latency_manifest.json", "timer_collision_manifest.json")
                  if (directory / name).is_file()]
    require(len(candidates) == 1, "Expected one measurement manifest in " + str(directory))
    manifest_path = candidates[0]
    manifest = read_json(manifest_path)
    require(manifest.get("testName") in TESTS, "Unknown production benchmark")
    kind = TESTS[manifest["testName"]]
    require(manifest.get("completedAtUtc") and manifest.get("runId"), "Measurement is incomplete")
    require(re.fullmatch(r"[0-9a-fA-F]{64}", manifest.get("runnerSha256", "")), "Missing runner identity")
    require(manifest.get("operationsPerCase") == {"start": 30, "pause": 30}, "Wrong action sample count")
    cases = manifest.get("cases")
    require(type(cases) is list and cases and all(type(case) is int and case in FULL_CASES[kind] for case in cases)
            and len(set(cases)) == len(cases), "Invalid/mixed case list")
    for variant, expected in expected_hashes.items():
        binary = manifest[variant]
        require(binary.get("sha256", "").lower() == expected, "Cross-mixed " + variant + " binary SHA")
        path = Path(binary["path"]).resolve(strict=True)
        if path not in binary_cache:
            binary_cache[path] = digest(path)
        require(binary_cache[path] == expected, "Measured binary path was overwritten: " + str(path))
    prefix = "timer_collision" if kind == "collision" else "timer_latency"
    aggregate_path = directory / (prefix + "_samples.json")
    aggregate = read_json(aggregate_path)
    raw_all, groups, processes, fixture_pairs = [], [], [], {}
    evidence_hashes = {str(manifest_path): digest(manifest_path), str(aggregate_path): digest(aggregate_path)}
    for case in cases:
        for variant in ("baseline", "candidate"):
            label = variant + "_" + ("history_68mib" if case == 68 else f"content_{case}mib")
            process_path = directory / (label + ".process.json")
            process = read_json(process_path)
            require(process.get("variant") == variant and process.get("case") == label[len(variant) + 1:], "Wrong process identity")
            require(type(process.get("exitCode")) is int and process["exitCode"] == 0
                    and boolean(process, "timedOut") is False, "Child failed or timed out")
            number(process, "elapsedSeconds")
            validate_memory(process, "memoryBeforeStart")
            validate_memory(process, "memoryAfterExit")
            stdout_path = directory / (label + ".stdout.log")
            stderr_path = directory / (label + ".stderr.log")
            stdout = stdout_path.read_text(encoding="utf-8-sig")
            require("test result: ok. 1 passed; 0 failed" in stdout, "Missing exact successful Rust test")
            for path in (process_path, stdout_path, stderr_path):
                evidence_hashes[str(path)] = digest(path)
            if kind == "native":
                native = directory / (label + "_native")
                samples = read_json(native / "native_timer_samples.json")
                completion = read_json(native / "native_timer_completion.json")
                reopened = read_json(native / "native_timer_reopen.json")
                require(completion.get("completed") is True and completion.get("error") is None
                        and completion.get("samples") == 60 and completion.get("programmaticDispatch") is True
                        and completion.get("productionNativeUpdate") is True, "Native completion is not verified")
                require(reopened == {"verified": True, "signedOut": True, "runningSlots": 0,
                                     "additionalSessions": 30, "notesUnchanged": True}, "Native saved-state reopen failed")
                images = list(native.glob("*.png"))
                require(len(images) == 8, "Expected eight retained native screenshot artifacts")
                for path in [native / name for name in ("native_timer_samples.json", "native_timer_completion.json", "native_timer_reopen.json")] + images:
                    evidence_hashes[str(path)] = digest(path)
                fixture = {key: samples[0][key] for key in ("stateBytes", "journalBytes")}
            else:
                marker = "TIMER_COLLISION" if kind == "collision" else "TIMER_LATENCY"
                samples = markers(stdout, marker + "_SAMPLE")
                fixtures = markers(stdout, marker + "_FIXTURE")
                require(len(fixtures) == 1, "Missing unique synthetic fixture marker")
                fixture = fixtures[0]
                require(fixture.get("variant") == variant and fixture.get("caseMiB") == case
                        and fixture.get("syntheticOnly") is True and fixture.get("signedOut") is True, "Invalid isolated fixture marker")
                if kind == "collision":
                    require("TIMER_COLLISION_REOPEN_VERIFIED sessions=30 running=0 signedOut=true" in stdout, "Missing collision reopen proof")
                else:
                    finals = markers(stdout, "TIMER_LATENCY_FINAL_STATE")
                    require(len(finals) == 1 and finals[0].get("variant") == variant and finals[0].get("caseMiB") == case
                            and finals[0].get("mirrorWarning") is None and finals[0].get("managedMirrorBoundBytes") == 32 * 1024 * 1024,
                            "Missing normal managed-mirror completion")
                    require(number(finals[0], "savedStateBytes", integer=True) <= 32 * 1024 * 1024, "Final mirror bound failed")
            validate_samples(samples, kind, variant, case)
            size = number(fixture, "stateBytes", integer=True)
            journal = number(fixture, "journalBytes", integer=True)
            if case in (1, 8, 32):
                require(case * 1024 * 1024 * .9 <= size <= case * 1024 * 1024 * 1.1, "Wrong sanitized fixture size")
            if case == 68:
                require(60 * 1024 * 1024 <= journal <= 80 * 1024 * 1024, "Wrong historical journal size")
            fixture_pairs[(case, variant)] = (size, journal)
            raw_all.extend(samples)
            processes.append({"kind": kind, "caseMiB": case, "variant": variant, **process})
            for action in ("start", "pause"):
                rows = [sample for sample in samples if sample["action"] == action]
                entry = {"kind": kind, "caseMiB": case, "variant": variant, "action": action,
                         "sampleCount": len(rows), "fixtureStateBytes": size, "fixtureJournalBytes": journal,
                         "metricsMicros": {metric: distribution([row[metric] for row in rows if row[metric] is not None]) for metric in METRICS[kind]}}
                if kind == "collision":
                    entry.update(firstAcceptedCount=sum(row["firstDispatchAccepted"] for row in rows),
                                 firstRejectedCount=sum(not row["firstDispatchAccepted"] for row in rows),
                                 explicitRetryCount=sum(row["retryDispatchMicros"] is not None for row in rows))
                else:
                    entry["feedbackVisibleCount"] = sum(row["feedbackVisible"] for row in rows)
                groups.append(entry)
        require(fixture_pairs[(case, "baseline")] == fixture_pairs[(case, "candidate")], "Baseline/candidate fixture sizes differ")
    canonical = lambda sample: json.dumps(sample, sort_keys=True, separators=(",", ":"), allow_nan=False)
    require(Counter(map(canonical, aggregate)) == Counter(map(canonical, raw_all)), "Aggregate samples differ from per-process raw evidence")
    return {"kind": kind, "cases": cases, "manifest": str(manifest_path), "runId": manifest["runId"],
            "runnerSha256": manifest["runnerSha256"], "evidenceSha256": evidence_hashes}, groups, processes


def compare(groups, targets):
    indexed = {(group["kind"], group["caseMiB"], group["action"], group["variant"]): group for group in groups}
    comparisons = []
    for key, current in sorted(indexed.items()):
        kind, case, action, variant = key
        if variant != "candidate":
            continue
        previous = indexed[(kind, case, action, "baseline")]
        for metric in METRICS[kind]:
            old = previous["metricsMicros"][metric]
            new = current["metricsMicros"][metric]
            row = {"kind": kind, "caseMiB": case, "action": action, "metric": metric,
                   "baselineObservationCount": old["observationCount"], "candidateObservationCount": new["observationCount"]}
            for quantile in ("p50", "p95", "maximum"):
                before, after = old[quantile], new[quantile]
                row[quantile] = {"baselineMs": None if before is None else before / 1000,
                                 "candidateMs": None if after is None else after / 1000,
                                 "candidateMinusBaselineMs": None if before is None or after is None else (after - before) / 1000,
                                 "reductionPercent": None if before in (None, 0) or after is None else (before - after) / before * 100}
            if (kind, metric) in targets:
                target = targets[(kind, metric)]
                value = row["p95"]["candidateMs"]
                row["explicitTarget"] = {"candidateP95CeilingMs": target, "candidateMinusTargetMs": None if value is None else value - target,
                                         "met": None if value is None else value <= target}
            comparisons.append(row)
    return comparisons


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, action="append", required=True)
    parser.add_argument("--baseline-sha256", required=True)
    parser.add_argument("--candidate-sha256", required=True)
    parser.add_argument("--target", action="append", default=[], metavar="KIND:METRIC:P95_MS")
    args = parser.parse_args()
    hashes = {"baseline": args.baseline_sha256.lower(), "candidate": args.candidate_sha256.lower()}
    require(all(re.fullmatch(r"[0-9a-f]{64}", value) for value in hashes.values()) and len(set(hashes.values())) == 2,
            "Supply distinct exact baseline and candidate SHA-256 identities")
    targets = {}
    for argument in args.target:
        kind, metric, limit = argument.split(":")
        require(kind in METRICS and metric in METRICS[kind], "Unknown target metric")
        ceiling = number({"ceiling": float(limit)}, "ceiling")
        require((kind, metric) not in targets, "Duplicate target")
        targets[(kind, metric)] = ceiling
    runs, groups, processes, binary_cache, seen = [], [], [], {}, set()
    for directory in args.run:
        run, new_groups, new_processes = inspect_run(directory.resolve(strict=True), hashes, binary_cache)
        for case in run["cases"]:
            identity = (run["kind"], case)
            require(identity not in seen, "Repeated case/kind: report repeated trials separately, do not pool")
            seen.add(identity)
        runs.append(run)
        groups.extend(new_groups)
        processes.extend(new_processes)
    coverage = {kind: {"requiredCases": cases, "observedCases": sorted(case for current, case in seen if current == kind),
                       "missingCases": [case for case in cases if (kind, case) not in seen]} for kind, cases in FULL_CASES.items()}
    report = {"formatVersion": 1, "evidenceValidation": "passed", "releaseAcceptance": "not inferred from timing evidence",
              "binarySha256": hashes, "measurementBasis": BASIS,
              "percentileMethod": "nearest rank; 30 samples per action: P50 rank 15, P95 rank 29; stage percentiles must not be added",
              "notes": ["Baseline pending frame return is not equivalent to a visible accepted-state label.",
                        "Collision rejected first clicks have no first-click completion latency; retry metrics remain nullable and separately counted.",
                        "Saved memory counters describe before/after conditions only; they do not measure peak memory or prove absence of pressure during the run.",
                        "Neither native screenshots nor synthetic frames measure OS mouse delivery or physical monitor presentation.",
                        "Fixture preparation is included in process elapsed time, not per-action latency."],
              "coverage": coverage, "runs": runs, "groups": groups, "comparisons": compare(groups, targets), "processes": processes}
    print(json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, TypeError) as error:
        print("TIMER_EVIDENCE_REJECTED: " + str(error), file=sys.stderr)
        raise SystemExit(1)
