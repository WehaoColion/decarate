- 2.21.35-release-security-hardening：完成安全审计、漏洞修复、正式构建与独立产物验证。

# Android release acceptance record: 2.21.35-release-security-hardening

## Candidate identity

| Field | Recorded value |
| --- | --- |
| Package | `com.ofairyo.gridtimer` |
| Release version | `2.21.35-release-security-hardening` |
| Android version code | `22135` |
| Minimum / target API | `26 / 34` |
| Verification date | `2026-08-23 +08:00` |
| Formal APK | `grid_timer_app_v2.21.35-release-security-hardening.apk` |
| Size | `18,975,251 bytes` |
| SHA-256 | `F31C5A05757FA37C70F63CBBF831E7AB40ED32809E8E18615621BEBFB4F87DD8` |

## Security findings and remediation

| Finding | Disposition |
| --- | --- |
| AI endpoint address confusion could disguise a remote host as loopback and forward credentials | Replaced string-prefix checks with structured URL validation, rejected user information, query, fragment, control characters and backslashes, allowed HTTP only for exact loopback hosts with an explicit port, required HTTPS elsewhere, and disabled redirects. |
| Stored rich-text notes could persist active HTML and export the same payload | Added a bounded canonical Rust allowlist sanitizer across save, revision, version, encryption and export paths; added a second browser-side sanitizer before rendering; made unsupported oversized content read-only so it cannot execute or be silently overwritten. |
| Rich-text WebView had an unnecessarily broad attack surface | Disabled popup, file, content and universal file URL access; blocked network loading and navigation; disabled mixed content; added a nonce-based Content Security Policy and attachment-only local resources. |
| Media import and transcription inputs were insufficiently bounded | Added byte, image-dimension, pixel-count, duration and output-sample limits. |
| Backup and permission scope was broader than needed | Removed audio, contacts and legacy storage permissions; limited exact-alarm compatibility permission to older Android; made Wi-Fi optional; required client-side encryption for legacy cloud backup and disabled modern cloud backup when encryption capability is unavailable. |
| Release signing and metadata controls were permissive | Release packaging now fails closed when signing configuration is absent or invalid, uses APK Signature Schemes v2/v3, disables v1 signing and removes native symbols and release VCS metadata. |

## Verification record

| Verification | Result |
| --- | --- |
| Rust library suite | `397 passed, 0 failed` |
| Rust source-generation suite | `46 passed, 0 failed` |
| Rust formatting | `cargo fmt --all -- --check` passed |
| Diff whitespace | `git diff --check` passed; only line-ending notices were emitted |
| Formal Android build | Offline `:app:assembleRelease` completed successfully; release lint and signing gates passed |
| Package identity | `com.ofairyo.gridtimer`, version code `22135`, version name `2.21.35-release-security-hardening` |
| APK mode | `debuggable=false`; only the launcher activity is exported |
| APK signatures | v1 `false`, v2 `true`, v3 `true`; RSA 4096-bit signing key |
| Signing certificate SHA-256 | `4ac7b86a600bbb3215a6187018c1442efcf3ffb25cdbeea0082cab81ef4d605f` |
| APK alignment | `zipalign -c -P 16 -v 4` passed |
| Native hardening | All three ABI libraries have non-executable stacks, GNU RELRO, immediate binding and no symbol table; 64-bit libraries use 16 KiB load alignment |
| Artifact consistency | Root, Gradle output and `release_artifacts/current` APK copies are byte-identical |
| Forbidden artifacts | No debug APK, Xiaomi debug APK, unversioned `app-release.apk` or AAB exists outside `old_apks/` |

No Android device or emulator was connected during this verification, so this release record does not claim a fresh device-level WebView or upgrade test. The sanitizer, URL boundary, persistence, encryption, generation and build paths are covered by the passing Rust and source-generation suites.

## Remaining release risks

1. The APK still targets API 34. This is not a confirmed exploit in the reviewed code, but it is already below Google Play's API 35 requirement for ordinary phone-app submissions; submissions move to API 36 on 2026-08-31. Upgrade the toolchain and revalidate Android 15/16 behavior before Play distribution.
2. `READ_CALL_LOG` remains because the recent-call picker depends on it. Google Play tightly restricts this permission, so Play distribution requires an eligible core use case or removal of that feature and permission.
3. `usesCleartextTraffic=true` remains for explicit loopback and local-LAN synchronization. Public AI endpoints and public sync routes are separately validated, and note WebViews cannot access the network, but the manifest-wide cleartext opt-in remains broader than ideal.
4. The release signing properties are stored locally in the OneDrive-backed project as plaintext configuration. They are ignored by Git, but the keystore and secrets should be moved to a protected credential location and rotated if this directory has ever been shared.
5. A live dependency-vulnerability database query was not completed because the configured VPN proxy endpoint was unavailable. The VPN configuration was not disabled or changed.

## Artifact disposition

The prior `2.21.34` root and current APK copies are preserved under `old_apks/`. The only current Android deliverable is the signed formal APK named above; no debug APK or AAB was generated or retained.
