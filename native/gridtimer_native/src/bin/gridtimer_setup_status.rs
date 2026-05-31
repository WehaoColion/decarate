use gridtimer_native::tooling::{
    first_non_blank, keytool_path, load_signing_properties, project_root,
    read_project_version_info, read_properties, FingerprintSnapshot,
};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    if let Err(error) = run() {
        eprintln!("status generation failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let output_path =
        parse_output_path().unwrap_or_else(|| project_root().join("xiaomi_setup_status.md"));
    let markdown = generate_markdown(&project_root())?;
    fs::write(output_path, markdown)
}

fn parse_output_path() -> Option<PathBuf> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--output-path" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

fn generate_markdown(project_root: &Path) -> io::Result<String> {
    let version = read_project_version_info(project_root)?;
    let local_properties = read_properties(&project_root.join("local.properties"))?;
    let release_signing = load_signing_properties(project_root)?;
    let configured_app_id = first_non_blank(&[
        env::var("XIAOMI_APP_ID").ok(),
        local_properties.get("XIAOMI_APP_ID").cloned(),
        local_properties.get("xiaomi.app.id").cloned(),
    ]);

    let app_id_source = if env::var("XIAOMI_APP_ID")
        .ok()
        .as_deref()
        .is_some_and(|value| value == "__UNCONFIGURED_XIAOMI_APP_ID__")
    {
        "placeholder XIAOMI_APP_ID"
    } else if env::var("XIAOMI_APP_ID").ok().is_some() {
        "environment variable XIAOMI_APP_ID"
    } else if local_properties.contains_key("XIAOMI_APP_ID") {
        "local.properties -> XIAOMI_APP_ID"
    } else if local_properties.contains_key("xiaomi.app.id") {
        "local.properties -> xiaomi.app.id"
    } else {
        "not configured"
    };

    let release_package = version.application_id.clone();
    let debug_package = format!("{release_package}{}", version.debug_suffix);
    let xiaomi_debug_package = release_package.clone();

    let release_fp = fingerprint_snapshot(
        "Release signing",
        release_signing.as_ref(),
        project_root,
        false,
    );
    let debug_fp = fingerprint_snapshot("Debug signing", None, project_root, true);

    let mut lines = Vec::new();
    push_line(&mut lines, "# Xiaomi HyperOS setup status");
    push_line(&mut lines, "");
    push_line(
        &mut lines,
        &format!("Generated: {}", current_timestamp_label()),
    );
    push_line(&mut lines, "");
    push_line(&mut lines, "## Current App ID state");
    push_line(
        &mut lines,
        &format!("- Configured: {}", configured_app_id.is_some()),
    );
    push_line(&mut lines, &format!("- Source: {}", app_id_source));
    if let Some(value) = configured_app_id {
        push_line(&mut lines, &format!("- Value: {value}"));
    } else {
        push_line(&mut lines, "- Value: missing");
        push_line(&mut lines, "- Note: Client-side HyperOS Focus/Super Island notification extras are still enabled. App ID is only tracked here for Mi Push or Xiaomi console onboarding.");
    }
    push_line(&mut lines, "");
    push_line(&mut lines, "## Package names to register");
    push_line(&mut lines, &format!("- Release package: {release_package}"));
    push_line(
        &mut lines,
        &format!("- XiaomiDebug package: {xiaomi_debug_package}"),
    );
    push_line(&mut lines, &format!("- Debug package: {debug_package}"));
    push_line(&mut lines, "");
    push_line(&mut lines, "## Signing fingerprints");
    push_fingerprint(&mut lines, &release_fp);
    push_fingerprint(&mut lines, &debug_fp);
    push_line(&mut lines, "## Recommended Xiaomi onboarding flow");
    push_line(&mut lines, "1. Finish Xiaomi developer real-name verification and create the app record that matches the release package.");
    push_line(&mut lines, "2. For client-side testing, install XiaomiDebug and start a timer; the app now selects the focus or island payload locally.");
    push_line(&mut lines, "3. For Mi Push or formal Xiaomi console onboarding, enable the Xiaomi Push or Super Island product family in the console.");
    push_line(
        &mut lines,
        &format!("4. Register the release signing fingerprint for {release_package}."),
    );
    push_line(&mut lines, &format!("5. Register the debug signing fingerprint for {xiaomi_debug_package} if you want XiaomiDebug enrolled too."));
    push_line(&mut lines, "6. Configure XIAOMI_APP_ID through the environment or local.properties only when a Xiaomi console app id is available.");
    push_line(&mut lines, "7. Check the in-app Xiaomi diagnostics panel. payloadMode should become focus or island depending on the device path.");
    push_line(&mut lines, "");
    push_line(&mut lines, "## Xiaomi 15 Pro targeting note");
    push_line(&mut lines, "- Xiaomi's official Super Island version document updated on 2025-10-23 says support depends on the OS build, not the handset name alone.");
    push_line(&mut lines, "- OS2 devices use Focus notifications. OS3 devices use Super Island. The same device can move between the two after a system update.");
    push_line(&mut lines, "- That means Xiaomi 15 Pro validation should target the latest HyperOS build on the device, not the marketing name alone.");
    push_line(&mut lines, "");
    push_line(&mut lines, "## Official references");
    push_line(&mut lines, "- Xiaomi Super Island developer guide: https://dev.mi.com/xiaomihyperos/documentation/detail?pId=1556");
    push_line(&mut lines, "- Xiaomi Super Island version info: https://dev.mi.com/xiaomihyperos/documentation/detail?pId=1580");
    push_line(&mut lines, "- Xiaomi Push service onboarding flow: https://dev.mi.com/xiaomihyperos/ability/detail/3121");
    push_line(&mut lines, "");

    Ok(lines.join("\n"))
}

fn fingerprint_snapshot(
    label: &str,
    signing: Option<&gridtimer_native::tooling::SigningProperties>,
    project_root: &Path,
    is_debug: bool,
) -> FingerprintSnapshot {
    let (keystore_path, store_password, alias, key_password) = if is_debug {
        (
            PathBuf::from(
                env::var("USERPROFILE").unwrap_or_else(|_| r"C:\Users\Public".to_string()),
            )
            .join(".android/debug.keystore"),
            String::from("android"),
            String::from("androiddebugkey"),
            String::from("android"),
        )
    } else if let Some(signing) = signing {
        (
            resolve_project_path(project_root, &signing.store_file),
            signing.store_password.clone(),
            signing.key_alias.clone(),
            signing.key_password.clone(),
        )
    } else {
        return FingerprintSnapshot {
            label: label.to_string(),
            available: false,
            path: None,
            alias: None,
            owner: None,
            sha1: None,
            sha256: None,
            reason: Some("Keystore path is not configured.".to_string()),
        };
    };

    if !keystore_path.exists() {
        return FingerprintSnapshot {
            label: label.to_string(),
            available: false,
            path: Some(keystore_path),
            alias: Some(alias),
            owner: None,
            sha1: None,
            sha256: None,
            reason: Some("Keystore not found.".to_string()),
        };
    }

    let keytool = keytool_path();
    if !keytool.exists() {
        return FingerprintSnapshot {
            label: label.to_string(),
            available: false,
            path: Some(keystore_path),
            alias: Some(alias),
            owner: None,
            sha1: None,
            sha256: None,
            reason: Some("keytool was not found.".to_string()),
        };
    }

    let output = Command::new(keytool)
        .arg("-list")
        .arg("-v")
        .arg("-keystore")
        .arg(&keystore_path)
        .arg("-storepass")
        .arg(store_password)
        .arg("-alias")
        .arg(alias.clone())
        .arg("-keypass")
        .arg(key_password)
        .output();

    let Ok(output) = output else {
        return FingerprintSnapshot {
            label: label.to_string(),
            available: false,
            path: Some(keystore_path),
            alias: Some(alias),
            owner: None,
            sha1: None,
            sha256: None,
            reason: Some("keytool invocation failed.".to_string()),
        };
    };

    if !output.status.success() {
        return FingerprintSnapshot {
            label: label.to_string(),
            available: false,
            path: Some(keystore_path),
            alias: Some(alias),
            owner: None,
            sha1: None,
            sha256: None,
            reason: Some(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        };
    }

    let text = String::from_utf8_lossy(&output.stdout);
    FingerprintSnapshot {
        label: label.to_string(),
        available: true,
        path: Some(keystore_path),
        alias: Some(alias),
        owner: text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("Owner:")
                .map(|value| value.trim().to_string())
        }),
        sha1: text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("SHA1:")
                .map(|value| value.trim().to_string())
        }),
        sha256: text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("SHA256:")
                .map(|value| value.trim().to_string())
        }),
        reason: None,
    }
}

fn push_fingerprint(lines: &mut Vec<String>, snapshot: &FingerprintSnapshot) {
    push_line(lines, &format!("### {}", snapshot.label));
    if snapshot.available {
        if let Some(path) = &snapshot.path {
            push_line(lines, &format!(" - Keystore: {}", path.display()));
        }
        if let Some(alias) = &snapshot.alias {
            push_line(lines, &format!(" - Alias: {alias}"));
        }
        if let Some(owner) = &snapshot.owner {
            if !owner.is_empty() {
                push_line(lines, &format!(" - Owner: {owner}"));
            }
        }
        if let Some(sha1) = &snapshot.sha1 {
            push_line(lines, &format!(" - SHA1: {sha1}"));
        }
        if let Some(sha256) = &snapshot.sha256 {
            push_line(lines, &format!(" - SHA256: {sha256}"));
        }
    } else {
        push_line(lines, " - Status: unavailable");
        if let Some(reason) = &snapshot.reason {
            push_line(lines, &format!(" - Reason: {reason}"));
        }
    }
    push_line(lines, "");
}

fn push_line(lines: &mut Vec<String>, line: &str) {
    lines.push(line.to_string());
}

fn current_timestamp_label() -> String {
    let millis = gridtimer_native::tooling::current_time_millis();
    format!("{millis}")
}

fn resolve_project_path(project_root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        project_root.join(path)
    }
}
