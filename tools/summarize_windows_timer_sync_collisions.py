"""Read-only analysis of the separately identified supplemental sync-collision run."""
from pathlib import Path
import argparse
from collections import Counter
import json
import re
import sys

from record_windows_timer_sync_builds import EXACT_TEST, verify_build
from summarize_windows_timer_latency import (
    boolean, digest, distribution, markers, number, read_json, require, validate_memory,
)

METRICS = [
    "firstDispatchReturnMicros", "handlerEnteredMicros", "firstFrameMicros", "syncObservedFrameMicros",
    "syncSettledMicros", "firstClickCommittedMicros", "acceptedMicros", "committedFrameWorkMicros", "peakFrameMicros",
    "acceptedToSubmittedMicros", "workerQueueMicros", "workerTransformMicros", "durableSaveMicros",
    "workerPostprocessMicros", "receiptDeliveryAndApplyMicros", "acceptedClickSaveCompletedMicros",
    "acceptedClickReceiptAppliedMicros",
]


def validate_sample(sample, variant, case):
    require(sample.get("variant") == variant and sample.get("caseMiB") == case
            and sample.get("action") in ("start", "pause"), "Cross-mixed supplemental sample")
    for field in ("sample", "caseMiB", "dispatchCount"):
        number(sample, field, integer=True)
    require(boolean(sample, "firstDispatchAccepted") and sample["dispatchCount"] == 1, "First click was rejected or retried")
    boolean(sample, "firstPendingFeedbackVisible")
    boolean(sample, "queuedBehindSync")
    require(boolean(sample, "syncMergedTitlePreserved") and boolean(sample, "finalRunningStateVerified"), "Saved state assertion failed")
    for metric in METRICS:
        number(sample, metric)
    require(sample["handlerEnteredMicros"] <= sample["acceptedMicros"] <= sample["firstDispatchReturnMicros"]
            <= sample["firstFrameMicros"] <= sample["firstClickCommittedMicros"] <= sample["syncSettledMicros"],
            "Invalid supplemental dispatch/frame timeline")
    require(sample["syncObservedFrameMicros"] <= sample["syncSettledMicros"]
            and sample["acceptedClickSaveCompletedMicros"] <= sample["acceptedClickReceiptAppliedMicros"]
            <= sample["firstClickCommittedMicros"], "Invalid save or sync-observation timeline")


def inspect(directory, expected):
    manifest_path = directory / "timer_sync_collision_manifest.json"
    manifest = read_json(manifest_path)
    require(manifest.get("testName") == EXACT_TEST and manifest.get("completedAtUtc"), "Wrong or incomplete supplemental run")
    require(manifest.get("operationsPerCase") == {"start": 30, "pause": 30}, "Wrong operation count")
    require(re.fullmatch(r"[0-9a-fA-F]{64}", manifest.get("runnerSha256", "")), "Missing supplemental runner identity")
    cases = manifest.get("cases")
    require(type(cases) is list and cases and all(type(case) is int and case in (1, 68) for case in cases)
            and len(set(cases)) == len(cases), "Invalid supplemental cases")
    proof = manifest["supplemental"]
    proof_path = Path(proof["buildProvenance"]["path"]).resolve(strict=True)
    require(digest(proof_path) == proof["buildProvenance"]["sha256"].lower(), "Supplemental build proof changed")
    # Historical reports remain readable after product source evolves; the exact
    # source/patch proof was fully checked before measurement and remains hashed.
    build = verify_build(proof_path, check_sources=False)
    require(proof["sourceProvenance"]["sha256"].lower() == build["sourceProvenance"]["sha256"].lower()
            and Path(proof["sourceProvenance"]["path"]).resolve() == Path(build["sourceProvenance"]["path"]).resolve()
            and proof["productionAndIsolatedSourceIdentities"] == build["sources"]
            and proof["sharedProbeSha256"] == build["sharedProbeSha256"] and proof["gateSha256"] == build["gateSha256"],
            "Source, patch and executable identities are not the same supplemental build")
    for variant in ("baseline", "candidate"):
        require(manifest[variant]["sha256"].lower() == expected[variant]
                and build["binaries"][variant]["sha256"].lower() == expected[variant]
                and Path(manifest[variant]["path"]).resolve() == Path(build["binaries"][variant]["path"]).resolve(),
                "Cross-mixed core/supplemental executable identity: " + variant)
    aggregate_path = directory / "timer_sync_collision_samples.json"
    aggregate = read_json(aggregate_path)
    evidence = {str(manifest_path): digest(manifest_path), str(aggregate_path): digest(aggregate_path),
                str(proof_path): digest(proof_path)}
    raw, groups, processes, fixtures = [], [], [], {}
    for case in cases:
        for variant in ("baseline", "candidate"):
            label = variant + "_" + ("history_68mib" if case == 68 else "content_1mib")
            stdout_path = directory / (label + ".stdout.log")
            stderr_path = directory / (label + ".stderr.log")
            process_path = directory / (label + ".process.json")
            process = read_json(process_path)
            require(process.get("variant") == variant and process.get("case") == label[len(variant) + 1:]
                    and process.get("executableSha256", "").lower() == expected[variant]
                    and Path(process["executable"]).resolve() == Path(manifest[variant]["path"]).resolve(), "Wrong process identity")
            require(type(process.get("exitCode")) is int and process["exitCode"] == 0
                    and boolean(process, "timedOut") is False, "Supplemental child failed or timed out")
            number(process, "elapsedSeconds")
            validate_memory(process, "memoryBeforeStart")
            validate_memory(process, "memoryAfterExit")
            text = stdout_path.read_text(encoding="utf-8-sig")
            require("test result: ok. 1 passed; 0 failed" in text, "Missing successful supplemental Rust test")
            samples = markers(text, "TIMER_SYNC_COLLISION_SAMPLE")
            fixture_rows = markers(text, "TIMER_SYNC_COLLISION_FIXTURE")
            reopened = markers(text, "TIMER_SYNC_COLLISION_REOPEN_VERIFIED")
            require(len(samples) == 60 and len(fixture_rows) == 1 and len(reopened) == 1, "Missing supplemental samples/fixture/reopen proof")
            fixture, reopen = fixture_rows[0], reopened[0]
            require(fixture.get("variant") == variant and fixture.get("caseMiB") == case
                    and fixture.get("syntheticOnly") is True and fixture.get("boundSyntheticAccount") is True
                    and fixture.get("networkDispatchBlocked") is True, "Fixture is not the isolated synthetic network-blocked account")
            require(reopen.get("sessions") == 30 and reopen.get("running") == 0 and reopen.get("boundSyntheticAccount") is True
                    and reopen.get("blockedMediaDispatches") == 60 and reopen.get("blockedRevocationDispatches") == 0,
                    "Supplemental reopen/network-block proof failed")
            for field in ("blockedSyncDispatches", "blockedMediaDispatches", "blockedLegalDispatches", "blockedRevocationDispatches"):
                number(reopen, field, integer=True)
            for field in ("stateBytes", "journalBytes"):
                number(fixture, field, integer=True)
            require(.9 * 1024 * 1024 <= fixture["stateBytes"] <= 1.1 * 1024 * 1024, "Wrong current fixture data size")
            if case == 68:
                require(60 * 1024 * 1024 <= fixture["journalBytes"] <= 80 * 1024 * 1024, "Wrong historical fixture size")
            for sample in samples:
                validate_sample(sample, variant, case)
            for action in ("start", "pause"):
                rows = [sample for sample in samples if sample["action"] == action]
                require(len(rows) == 30 and {sample["sample"] for sample in rows} == set(range(30)), "Duplicate or missing supplemental action ordinal")
                groups.append({"caseMiB": case, "variant": variant, "action": action, "sampleCount": 30,
                               "firstAcceptedCount": sum(row["firstDispatchAccepted"] for row in rows), "explicitRetryCount": 0,
                               "queuedBehindSyncCount": sum(row["queuedBehindSync"] for row in rows),
                               "firstPendingFeedbackVisibleCount": sum(row["firstPendingFeedbackVisible"] for row in rows),
                               "metricsMicros": {metric: distribution([row[metric] for row in rows]) for metric in METRICS}})
            raw.extend(samples)
            fixtures[(case, variant)] = (fixture["stateBytes"], fixture["journalBytes"])
            processes.append({"caseMiB": case, **process, "fixture": fixture, "reopen": reopen})
            for path in (stdout_path, stderr_path, process_path):
                evidence[str(path)] = digest(path)
        require(fixtures[(case, "baseline")] == fixtures[(case, "candidate")], "Supplemental fixture sizes differ across versions")
    canonical = lambda row: json.dumps(row, sort_keys=True, separators=(",", ":"), allow_nan=False)
    require(Counter(map(canonical, raw)) == Counter(map(canonical, aggregate)), "Supplemental aggregate differs from raw process evidence")
    return manifest, evidence, groups, processes


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--baseline-sha256", required=True)
    parser.add_argument("--candidate-sha256", required=True)
    parser.add_argument("--p95-target-ms", type=float)
    args = parser.parse_args()
    expected = {"baseline": args.baseline_sha256.lower(), "candidate": args.candidate_sha256.lower()}
    require(all(re.fullmatch(r"[0-9a-f]{64}", value) for value in expected.values()) and len(set(expected.values())) == 2,
            "Supply distinct exact supplemental binary SHA-256 identities")
    if args.p95_target_ms is not None:
        number({"target": args.p95_target_ms}, "target")
    manifest, evidence, groups, processes = inspect(args.run.resolve(strict=True), expected)
    comparisons = []
    for candidate in (group for group in groups if group["variant"] == "candidate"):
        baseline = next(group for group in groups if group["variant"] == "baseline"
                        and group["caseMiB"] == candidate["caseMiB"] and group["action"] == candidate["action"])
        for metric in METRICS:
            result = {"caseMiB": candidate["caseMiB"], "action": candidate["action"], "metric": metric}
            for percentile in ("p50", "p95", "maximum"):
                before = baseline["metricsMicros"][metric][percentile]
                after = candidate["metricsMicros"][metric][percentile]
                result[percentile] = {"baselineMs": before / 1000, "candidateMs": after / 1000,
                                      "candidateMinusBaselineMs": (after - before) / 1000,
                                      "reductionPercent": None if before == 0 else (before - after) / before * 100}
            if metric == "firstClickCommittedMicros" and args.p95_target_ms is not None:
                value = result["p95"]["candidateMs"]
                result["explicitTarget"] = {"candidateP95CeilingMs": args.p95_target_ms,
                                            "candidateMinusTargetMs": value - args.p95_target_ms, "met": value <= args.p95_target_ms}
            comparisons.append(result)
    print(json.dumps({"formatVersion": 1, "evidenceValidation": "passed", "releaseAcceptance": "not inferred",
                      "kind": "supplemental_sync_collision", "binarySha256": expected, "sourceProof": manifest["supplemental"],
                      "coverage": {"requiredCases": [1, 68], "observedCases": manifest["cases"],
                                   "missingCases": [case for case in (1, 68) if case not in manifest["cases"]]},
                      "measurementBasis": "Synthetic queued click starts before complete production frame pump, then one dispatch. Real SyncTaskResult validation/merge/persistence runs; new outbound network dispatch is blocked only by test instrumentation.",
                      "notes": ["Separate supplemental executable identities; do not pool with core/native/autosave timing distributions.",
                                "handlerEntered includes any synchronous sync-response work in the frame pump before the timer handler.",
                                "syncObservedFrame is the first frame showing the remote title, not a dedicated durable receipt timestamp.",
                                "syncSettled includes final job completion and supervision settling after timer confirmation.",
                                "Timer durableSave covers the timer save; prior sync merge/save can appear before handler entry or in acceptedToSubmitted.",
                                "No OS mouse, native GPU presentation, monitor scanout, or real network time is measured.",
                                "Nearest-rank P50/P95 use ranks15/29 per 30 samples. Stage percentiles must not be added."],
                      "evidenceSha256": evidence, "groups": groups, "comparisons": comparisons, "processes": processes},
                     ensure_ascii=False, indent=2, allow_nan=False))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, OSError, TypeError) as error:
        print("SYNC_COLLISION_EVIDENCE_REJECTED: " + str(error), file=sys.stderr)
        raise SystemExit(1)
