- 2.21.36-timer-index-horizontal: keep two-digit timer module indexes on one horizontal line and publish the verified formal release.

# Android release acceptance record: 2.21.36-timer-index-horizontal

## Candidate identity

| Field | Recorded value |
| --- | --- |
| Package | `com.ofairyo.gridtimer` |
| Release version | `2.21.36-timer-index-horizontal` |
| Android version code | `22136` |
| Minimum / target API | `26 / 34` |
| Verification date | `2026-08-23 +08:00` |
| Formal APK | `grid_timer_app_v2.21.36-timer-index-horizontal.apk` |
| Size | `18,975,375 bytes` |
| SHA-256 | `A9C951F3681F4174883BFB85A705023B188B8818D1972F060D20D8E3344CFD76` |

## Accepted change

The timer slot badge now renders its zero-padded index with `maxLines = 1`. The shared instrument text component consequently disables soft wrapping and retains its existing bounded scale-down behavior. This keeps values such as `01` and `02` horizontal without changing the timer dial or card structure.

A source-generation regression test isolates `SlotBadge` and requires both the zero-padded two-digit value and the single-line constraint.

## Verification record

| Verification | Result |
| --- | --- |
| Rust library suite | `397 passed, 0 failed` |
| Rust source-generation suite | `47 passed, 0 failed`, including the timer badge single-line regression |
| Rust formatting | `cargo fmt --all -- --check` passed |
| Diff whitespace | `git diff --check` passed; only line-ending notices were emitted |
| Formal release build | `81` Gradle tasks completed successfully; source audit, debug lint, release vital lint and signing gates passed |
| Package identity | `com.ofairyo.gridtimer`, version code `22136`, version name `2.21.36-timer-index-horizontal` |
| APK signatures | v1 `false`, v2 `true`, v3 `true`; RSA 4096-bit signing key |
| Signing certificate SHA-256 | `4ac7b86a600bbb3215a6187018c1442efcf3ffb25cdbeea0082cab81ef4d605f` |
| APK alignment | `zipalign -c -p -v 4` passed |
| Artifact consistency | Root, Gradle output and `release_artifacts/current` APK copies are byte-identical |
| Release-set gate | Packager `finish --validate-only` passed for version name `2.21.36-timer-index-horizontal` and version code `22136` |
| Forbidden artifacts | No debug APK, Xiaomi debug APK, unversioned `app-release.apk` or AAB exists in the root, current release tree or Gradle package output |

## Runtime layout verification

The signed formal APK installed successfully on the local `TimerVerify_ATD34` Android 14 verification emulator and cold-started `com.ofairyo.gridtimer/.MainActivity`.

The running Compose hierarchy exposed each timer index as one text node. The first visible timer card reported `01` at `[116,1753][165,1796]`, a 49 by 43 pixel text box, and the second reported `02` at `[635,1753][684,1796]` with the same dimensions. Both values are wider than tall and remain on one horizontal line.

## Artifact disposition

The prior `2.21.35-release-security-hardening` root and current APK copies are preserved under `old_apks/`. The project root contains only the signed formal `2.21.36-timer-index-horizontal` APK; no debug APK or AAB was generated or retained.
