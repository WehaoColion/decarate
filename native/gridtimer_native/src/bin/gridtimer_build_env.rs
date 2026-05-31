use gridtimer_native::tooling::{
    cargo_toolchain_exe, load_signing_properties, project_root, read_project_version_info,
    resolve_android_sdk_dir, resolve_latest_ndk_directory, resolve_xiaomi_app_id,
};
use serde_json::json;
use std::env;
use std::io;
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("build env generation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let project_root = project_root();
    let android_sdk = resolve_android_sdk_dir(&project_root);
    let ndk_dir = android_sdk
        .as_ref()
        .and_then(|sdk| resolve_latest_ndk_directory(sdk));
    let signing = load_signing_properties(&project_root)?;
    let version = read_project_version_info(&project_root)?;
    let cargo = cargo_toolchain_exe();
    let linker = env::var_os("CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from("C:\\tools\\rustup\\toolchains\\stable-x86_64-pc-windows-msvc\\lib\\rustlib\\x86_64-pc-windows-msvc\\bin\\rust-lld.exe")
        });
    let lib_dir = env::var("LIB").unwrap_or_else(|_| {
        [
            "C:\\tools\\xwin\\crt\\lib\\x86_64",
            "C:\\tools\\xwin\\sdk\\lib\\ucrt\\x86_64",
            "C:\\tools\\xwin\\sdk\\lib\\um\\x86_64",
        ]
        .join(";")
    });

    let payload = json!({
        "namespace": "com.ofairyo.gridtimer",
        "applicationId": "com.ofairyo.gridtimer",
        "compileSdk": 34,
        "minSdk": 26,
        "targetSdk": 34,
        "versionCode": version.version_code,
        "versionName": version.version_name,
        "debugAppName": "Grid Timer",
        "composeCompilerExtensionVersion": "1.5.14",
        "composeBomVersion": "2024.06.00",
        "implementationDeps": [
            "androidx.core:core-ktx:1.13.1",
            "androidx.appcompat:appcompat:1.7.0",
            "androidx.lifecycle:lifecycle-runtime-ktx:2.8.4",
            "androidx.lifecycle:lifecycle-viewmodel-compose:2.8.4",
            "androidx.activity:activity-compose:1.9.3",
            "androidx.startup:startup-runtime:1.2.0",
            "androidx.profileinstaller:profileinstaller:1.4.1",
            "androidx.compose.ui:ui",
            "androidx.compose.ui:ui-tooling-preview",
            "androidx.compose.foundation:foundation",
            "androidx.compose.animation:animation",
            "androidx.compose.material3:material3",
            "androidx.compose.material:material-icons-extended",
            "org.jetbrains.kotlinx:kotlinx-coroutines-android:1.8.1",
            "org.jetbrains.kotlinx:kotlinx-serialization-json:1.6.3",
        ],
        "debugDeps": [
            "androidx.compose.ui:ui-tooling",
            "androidx.compose.ui:ui-test-manifest",
        ],
        "androidTestDeps": [
            "androidx.test.ext:junit:1.2.1",
            "androidx.test.espresso:espresso-core:3.6.1",
            "androidx.compose.ui:ui-test-junit4",
        ],
        "testDeps": [
            "junit:junit:4.13.2",
        ],
        "xiaomiAppId": resolve_xiaomi_app_id(&project_root).unwrap_or_default(),
        "androidSdk": android_sdk.as_ref().map(|value| value.display().to_string()),
        "ndkDir": ndk_dir.as_ref().map(|value| value.display().to_string()),
        "cargoExe": cargo.display().to_string(),
        "linker": linker.display().to_string(),
        "libDir": lib_dir,
        "signing": signing.map(|value| json!({
            "f": value.store_file,
            "p": value.store_password,
            "a": value.key_alias,
            "k": value.key_password,
        })),
    });

    println!("{payload}");
    Ok(())
}
