# DecaRate

DecaRate is a Rust-powered Android and Windows focus timer. The Android app is generated from the Rust source tree, uses native Rust logic through JNI, and includes focus timing, note management, finance records, sync support, notification effects, and Xiaomi HyperOS notification experiments.

## Project layout

- `app/` contains the Android Gradle project and static Android resources.
- `native/gridtimer_native/` contains the Rust crate, JNI bridge, generated Android source emitters, desktop client, sync server, launcher, packaging helpers, and source audit tools.
- `release_artifacts/current/` contains the current public APK and Windows executables.
- `old_apks/` contains historical public release artifacts.
- `documents/thesis/` contains English-path copies of related Word documents.
- `xiaomi_hyperos_setup.md` documents the Xiaomi HyperOS onboarding notes.

## Build requirements

- JDK 17
- Android SDK with API 34
- Android NDK 27.1.12297006
- Rust stable toolchain
- `cargo-ndk`
- Git LFS for binary release artifacts

The current Gradle script expects the local Rust and Android toolchain paths used by the original workstation. If your paths differ, update `app/build.gradle` or provide equivalent local tooling paths before building.

## Local configuration

The repository intentionally does not include local signing credentials or machine-specific configuration. Create these files locally when needed:

- `local.properties` with `sdk.dir=...`
- `release_signing.properties` with `storeFile`, `storePassword`, `keyAlias`, and `keyPassword`
- a release keystore matching your own signing setup

Do not commit keystores, passwords, API keys, personal machine paths, or generated setup reports.

## Build commands

From the repository root, use a local Gradle installation:

```powershell
gradle :app:assembleRelease
```

The Gradle build runs the Rust source generator, builds the native libraries, runs the source audit helper, and produces a versioned release APK when signing is configured.

Useful Rust commands from `native/gridtimer_native/`:

```powershell
cargo test
cargo run --bin timer_sync_server
cargo run --features desktop --bin timer_windows_client
```

## Public artifacts

Current public artifacts are stored in `release_artifacts/current/`. Historical builds are kept in `old_apks/` for traceability. Large binaries and Word/PDF documents are tracked with Git LFS. Some binary filenames retain the earlier `grid_timer` prefix for release compatibility.

## Files intentionally not published

The repository excludes generated build outputs, Gradle and Cargo caches, local temporary folders, signing credentials, keystores, local SDK paths, generated setup reports with personal paths, and tool-specific traces.
