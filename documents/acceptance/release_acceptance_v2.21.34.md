- Version 2.21.34 - record the protection-state safety release candidate and current verification evidence.

# Release acceptance record: 2.21.34-protection-state-safety

## Candidate identity

| Field | Recorded value |
| --- | --- |
| Release version | `2.21.34-protection-state-safety` |
| Android version code | `22134` |
| Verification timestamp | `2026-08-23 03:57:24 +08:00` |
| Git HEAD used as base | `57ac62f9c0c82b553582489bd1e5d0ff675f5c4d` |
| Tracked binary diff object | `a17b5c6af7e9e581ee294138cced8f2aafaaf056` |
| Implementation manifest | 28 files under the active build inputs, SHA-256 `5c5f9baaf232aa79eb01563a1eab384a9adb48d4d3f47fca9d781119a58e99be` |
| Worktree status at fingerprint time | 18 modified, 34 deleted, 137 untracked entries; the repository was intentionally not reset, cleaned, staged, or committed |
| Evidence root | `release_artifacts/verification/v2.21.34/` |

This is the current candidate record. The earlier `sync_identity_rebind_acceptance_v2.21.23.md` remains a historical contract/template for the older target and its unfilled example table is not evidence for this release.

## First independent review remediation map

| First-round item | Current disposition and executable evidence |
| --- | --- |
| C-01, encryption disable could not converge | Added a monotonic protection-state generation and merge rules for authenticated encryption, decryption, and replay transitions; covered by the Rust library suite and generated-source suite. |
| C-02, old crypto session could roll back a rotated key | Rotation now revokes the old generation's sibling sessions, zeroizes revoked material, and durable note writes reject a lower protection generation; covered by crypto and persistence tests. |
| C-03, mixed artifact versions | APK, server, launcher, Windows client, filenames, and health response all identify `2.21.34-protection-state-safety`; exact hashes are recorded below. |
| C-04, evidence table was unfilled | This current candidate record contains concrete values and evidence paths. The old 2.21.23 document is explicitly historical. |
| G-01, no Android behavior tests | The release instrumentation suite now executes 15 production-path tests on API 34, including persistence failure injection, force-stop recovery, encryption recovery, password rotation, and identity rebind. |
| C-05, `onCleared` did not flush | Lifecycle shutdown now returns safely on the main thread while a FIFO worker drains accepted writes, flushes, and closes in order; release instrumentation executes this order. |
| C-06, failed password rotation could not retry | Password change uses prepare, durable write, commit, and abort; the Android test covers abort, direct retry, commit, and old-session revocation. |
| C-07, formatting failed | `cargo fmt --all -- --check` passes. |
| R-01, desktop semantic summary missed note versions | The summary now counts active/deleted versions and version attachments; regression coverage is in the Rust suite and Android version-stack tests. |
| R-02, encrypted notes lacked safe journal recovery | The recovery journal accepts only redacted authenticated ciphertext, verifies binding, and rejects protection-state rollback; release instrumentation executes this behavior. |
| R-03, README toolchain was not reproducible | README now names the MSVC toolchain and current commands used by this release. |
| R-04, warning noise | Desktop all-target checks and all three Android ABI builds complete with zero warnings after target-specific `cfg`/allow justification. |
| First-round runtime gaps | Current candidate launcher, local health, public health, APK upgrade, force-stop, instrumentation, and packaged identity-rebind/restart flows were executed and retained below. |

## Test and static-check record

| Verification | Result |
| --- | --- |
| Rust library suite | `373 passed, 0 failed, 0 ignored` |
| Windows client suite | `92 passed, 0 failed, 0 ignored` |
| Source-generation suite | `45 passed, 0 failed, 0 ignored` |
| Exact server identity-rebind filter | `2 passed, 0 failed`, 371 filtered |
| Exact Windows identity-rebind filter | `6 passed, 0 failed`, 86 filtered |
| Android JVM host tests | Gradle `testDebugUnitTest` completed as `NO-SOURCE`; no host-only substitute was counted as behavior evidence |
| Android release instrumentation | `15 tests, 0 failures, 0 errors, 0 skipped`, `5.791 s`; JUnit SHA-256 `6C88718AAE0CCE8C59A84F48A49CB8D82B2ACC03748B0D8965014E67A2651B71` |
| External Android force-stop phases | prepare `OK (1 test)`, host `am force-stop`, verify `OK (1 test)` |
| Source audit | `source audit ok` |
| Formatting | `cargo fmt --all -- --check`: exit 0 |
| Static checks | MSVC desktop all-target/all-feature check: exit 0, zero warnings; Android `aarch64`, `armv7`, and `x86_64` native builds: zero warnings |
| Diff whitespace | `git diff --check`: exit 0; only line-ending conversion notices were printed by Git |
| Android build gates | `clean`, `testDebugUnitTest`, `lintDebug`, and `assembleRelease` completed in the formal packager build |

Android raw results and the force-stop summary are in `release_artifacts/verification/v2.21.34/android_instrumentation_results.xml`, `android_instrumentation_summary.md`, and `android_upgrade_force_stop_summary.md`.

## Formal artifacts

| Artifact | Size | SHA-256 |
| --- | ---: | --- |
| `grid_timer_app_v2.21.34-protection-state-safety.apk` | 19,800,019 | `D3097831BE41CC63E48CB36025F001E9C5AD14787CD6C091683619EFF8404D54` |
| `grid_timer_sync_launcher_v2.21.34-protection-state-safety.exe` | 3,973,632 | `2E0D31B91AB188BD7472296FE6090819968F80C45D3D4836D0BC7C2CD8F0F5B7` |
| `grid_timer_sync_server_v2.21.34-protection-state-safety.exe` | 4,881,920 | `821364AC29F11E620FC3A2C4433CC0747A6CDADE88EC243E1367253CFC2FDA87` |
| `grid_timer_windows_client_v2.21.34-protection-state-safety.exe` | 11,043,840 | `DCFC76C68DDA35E6C430462FA2C0C9259155FF17F73A967941DDFAF6E0641D24` |
| `tools/cloudflared.exe` | 54,159,168 | `CCB0756DE288D3C2C076D19764CA53E0849A10F2DD9C23F8656AC42BDEB45001` |

The root APK and `release_artifacts/current` APK are byte-identical. `aapt` reports package `com.ofairyo.gridtimer`, `versionCode=22134`, `versionName=2.21.34-protection-state-safety`, and label `十倍率`. APK Signature Schemes v2 and v3 verify, the signing certificate SHA-256 is `4ac7b86a600bbb3215a6187018c1442efcf3ffb25cdbeea0082cab81ef4d605f`, and `zipalign -c -v 4` passes.

The formal packager's post-build validation passed for the exact version name and code. A recursive scan found no debug APK, Xiaomi debug APK, unversioned `app-release.apk`, AAB, or second formal APK outside `old_apks/`.

## Actual runtime record

### Isolated packaged server

- Persistent project address: `http://127.0.0.1:33498/`; listener was exactly `127.0.0.1`.
- `/health`: HTTP `200`, exact process `grid_timer_sync_server_v2.21.34-protection-state-safety.exe`, exact build `2.21.34-protection-state-safety`, `backupStatus=ok`.
- Isolated SQLite: SHA-256 `DEDB1E191429A02EA59DF66FFADD66EB0596268992DB1927313A6B2DA89B7AE5`, `integrity_check=ok`, zero foreign-key violations.
- Only the path-verified candidate PID was stopped; its isolated database and logs remain under `isolated_server/`.

### Packaged launcher and public route

- The exact candidate launcher started the exact candidate server and bundled cloudflared.
- Local `/health`: HTTP `200`, exact release build, healthy backup.
- Public `/health` at verification time: `https://lives-timber-books-depend.trycloudflare.com/health`, HTTP `200`, same process name, build ID, published URL, and backup state as local health.
- VPN and system proxy settings were not disabled or changed.
- The candidate run used isolated data and disabled only its own autostart registration. The user's prior `2.21.22` service tree and registry value were restored and reverified after the candidate run.
- Full process, database, log, and restoration evidence: `launcher_runtime_summary.md` and `launcher_runtime/`.

### Android upgrade and recovery

- A signed 2.21.33 formal APK was clean-installed on `TimerVerify_ATD34` and given note markers `Audit22134` and `RecoveryBody_JTAIL42`.
- The signed 2.21.34 APK was installed in place with `adb install -r`.
- Both markers remained after the upgrade and remained again after a second force-stop/cold-start cycle.
- Package-scoped crash-pattern count was zero.
- Raw hierarchy and log hashes are recorded in `android_upgrade_force_stop_summary.md`.

### Identity rebind, receipt acknowledgement, and restart

- Exact Rust server/core rebind tests: 2 passed.
- Exact Windows production-client rebind tests: 6 passed.
- Android production-path rebind instrumentation: 4 passed.
- The packaged server was also exercised over real HTTP through identity A, ordinary mismatch, identity-B rebind baseline, local/server merge, exact receipt acknowledgement, new proof issuance, server restart, and a proof-bound second sync.
- The before database contains only `server-marker-22134`; the final database contains `server-marker-22134`, `local-marker-22134`, and `restart-marker-22134`, has revision 3, an acknowledged generation-zero token with no receipt, `integrity_check=ok`, and zero foreign-key violations.
- The packaged-network harness and client durability tests are deliberately reported as layered evidence; it does not pretend that the GUI binary itself emitted the retained HTTP transcript.
- Full evidence: `identity_rebind_runtime_summary.md` and `identity_rebind_runtime/`.

## Six-dimensional main-thread assessment before revalidation

| Dimension | Main-thread assessment |
| --- | --- |
| Requirements completeness | All first-round release blockers have an implementation, executable check, and current artifact/runtime record. |
| Logic correctness | Protection transitions, crypto rotation, durable ordering, recovery journal, and identity rebind now have positive and fail-closed tests. |
| Boundary cases | Read-only/I/O failure, queue shutdown, force-stop, encrypted rollback, forged rebind, nonzero/forced variants, write failures, and target conflicts are covered. |
| Code quality | Formatting, whitespace, source audit, static checks, and warning policy pass. |
| Test coverage | Current Rust, Windows, source generation, and Android release instrumentation suites pass; behavior evidence is not limited to generated-source string assertions. |
| Actual runtime | Formal APK upgrade/recovery, isolated packaged server, packaged launcher, public route, database integrity, and packaged protocol rebind/restart all passed. |

## Independent review state

The same independent review task `01a02a49-8981-7dc0-a60c-638838fc7a1e` returned `结论：不通过` in cycle 1 and supplied C-01 through C-07, G-01, and R-01 through R-04. This record and the referenced candidate evidence are the cycle-2 submission. The release remains blocked by process until that same read-only reviewer adjudicates the six dimensions again. Its cycle-2 verdict is not prewritten into this record; if it passes, the main thread will append the exact verdict and timestamp without changing the candidate binaries.
