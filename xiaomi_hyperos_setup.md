# Xiaomi HyperOS App ID setup

This project already has a Xiaomi-specific notification path for HyperOS Focus notification and Super Island payloads. Client-side island notifications are sent through normal Android notifications with Xiaomi extras; `XIAOMI_APP_ID` is intentionally kept out of tracked source files because it is only needed for Mi Push or Xiaomi console onboarding.

## What is already wired

- `app/build.gradle.kts` reads `XIAOMI_APP_ID` from Gradle properties, environment variables, or `local.properties`.
- `app/src/main/AndroidManifest.xml` publishes the value through `com.xiaomi.xms.APP_ID`.
- `XiaomiIslandNotifier.kt` reads the manifest metadata for diagnostics, but does not block client-side Focus or Super Island payloads when the App ID is missing.
- `build_apk.ps1 -Configuration XiaomiDebug` builds a test APK that still includes the client-side HyperOS payload path even without `XIAOMI_APP_ID`.

## What still has to happen outside the repo for Mi Push or formal console validation

1. Create or verify the Xiaomi developer account.
2. Create the Xiaomi app record that matches the release package name.
3. Enable the Xiaomi Push or Super Island capability and obtain the official App ID.
4. Register the signing fingerprints that Xiaomi will use for authorization.

The official Xiaomi docs currently describe the flow this way:

- Xiaomi Push service page shows `developer account -> app listing -> apply for access -> use the service`.
- Xiaomi Super Island guide updated on 2026-01-29 explains both the client-side notification path and the Mi Push path.
- Xiaomi Super Island version info updated on 2025-10-23 says support is based on HyperOS version rather than model, with OS2 using Focus notifications and OS3 using Super Island.

## Project-specific package plan

- Release package: `com.ofairyo.gridtimer`
- XiaomiDebug package: `com.ofairyo.gridtimer`
- Debug package: `com.ofairyo.gridtimer.debug`

The XiaomiDebug build keeps the release package name on purpose so Xiaomi authorization can be tested on-device before release. The normal Debug build keeps the `.debug` suffix so ordinary Android debugging stays isolated.

## Recommended local configuration for console metadata

Use one of these sources:

1. `XIAOMI_APP_ID` environment variable
2. `local.properties` with `XIAOMI_APP_ID=...`
3. `local.properties` with `xiaomi.app.id=...`

Do not hardcode the real App ID in tracked files.

## Recommended signing plan

- Register the release signing fingerprint for `com.ofairyo.gridtimer`.
- Register the debug signing fingerprint as well if XiaomiDebug authorization will be tested before release.
- Avoid treating an accidental temporary release keystore as final onboarding material. Xiaomi-side authorization must match the long-term release signing key.

## Client-side validation checklist

1. Run `powershell -File .\xiaomi_setup_status.ps1`
2. Build `powershell -File .\build_apk.ps1 -Configuration XiaomiDebug`
3. Install the versioned `grid_timer_app_xiaomi_debug_v<version>.apk`
4. Start a timer on a supported HyperOS device
5. Open the in-app Xiaomi diagnostics and confirm `payloadMode` is `focus` on OS2 or `island` on OS3
6. If `missing_app_id_push_only` appears, treat it as Mi Push/console metadata only, not as a blocker for the client-side island notification path

## Official links

- https://dev.mi.com/xiaomihyperos/documentation/detail?pId=2131
- https://dev.mi.com/xiaomihyperos/documentation/detail?pId=2141
- https://dev.mi.com/xiaomihyperos/ability/mipush
