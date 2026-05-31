use std::collections::HashMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct ProjectVersionInfo {
    pub application_id: String,
    pub debug_suffix: String,
    pub version_name: String,
    pub version_code: i32,
}

#[derive(Clone, Debug)]
pub struct SigningProperties {
    pub store_file: String,
    pub store_password: String,
    pub key_alias: String,
    pub key_password: String,
}

#[derive(Clone, Debug)]
pub struct FingerprintSnapshot {
    pub label: String,
    pub available: bool,
    pub path: Option<PathBuf>,
    pub alias: Option<String>,
    pub owner: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub reason: Option<String>,
}

pub fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

pub fn first_non_blank(values: &[Option<String>]) -> Option<String> {
    values
        .iter()
        .filter_map(|value| value.as_ref())
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub fn read_properties(path: &Path) -> io::Result<HashMap<String, String>> {
    let mut properties = HashMap::new();
    if !path.exists() {
        return Ok(properties);
    }

    for line in fs::read_to_string(path)?.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(index) = trimmed.find('=') else {
            continue;
        };
        let key = unescape_properties_text(trimmed[..index].trim());
        let value = unescape_properties_text(trimmed[index + 1..].trim());
        properties.insert(key, value);
    }
    Ok(properties)
}

pub fn read_project_version_info(project_root: &Path) -> io::Result<ProjectVersionInfo> {
    let build_file = {
        let kts = project_root.join("app/build.gradle.kts");
        if kts.exists() {
            kts
        } else {
            project_root.join("app/build.gradle")
        }
    };
    let text = fs::read_to_string(&build_file)?;

    let version_name = extract_quoted_value(&text, "versionName")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "versionName not found"))?;
    let version_code = extract_integer_after_token(&text, "versionCode")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "versionCode not found"))?;
    let application_id = extract_quoted_value(&text, "applicationId")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "applicationId not found"))?;
    let debug_suffix =
        extract_quoted_value(&text, "applicationIdSuffix").unwrap_or_else(|| ".debug".to_string());

    Ok(ProjectVersionInfo {
        application_id,
        debug_suffix,
        version_name,
        version_code,
    })
}

pub fn load_signing_properties(project_root: &Path) -> io::Result<Option<SigningProperties>> {
    let props = read_properties(&project_root.join("release_signing.properties"))?;
    let store_file = props.get("storeFile").cloned().unwrap_or_default();
    let store_password = props.get("storePassword").cloned().unwrap_or_default();
    let key_alias = props.get("keyAlias").cloned().unwrap_or_default();
    let key_password = props.get("keyPassword").cloned().unwrap_or_default();

    if store_file.trim().is_empty()
        || store_password.trim().is_empty()
        || key_alias.trim().is_empty()
        || key_password.trim().is_empty()
    {
        return Ok(None);
    }

    Ok(Some(SigningProperties {
        store_file,
        store_password,
        key_alias,
        key_password,
    }))
}

pub fn resolve_release_signing_properties(
    project_root: &Path,
) -> io::Result<Option<SigningProperties>> {
    let env_store_file = env::var("RELEASE_STORE_FILE").ok();
    let env_store_password = env::var("RELEASE_STORE_PASSWORD").ok();
    let env_key_alias = env::var("RELEASE_KEY_ALIAS").ok();
    let env_key_password = env::var("RELEASE_KEY_PASSWORD").ok();
    if let (Some(store_file), Some(store_password), Some(key_alias), Some(key_password)) = (
        env_store_file.clone(),
        env_store_password.clone(),
        env_key_alias.clone(),
        env_key_password.clone(),
    ) {
        if !store_file.trim().is_empty()
            && !store_password.trim().is_empty()
            && !key_alias.trim().is_empty()
            && !key_password.trim().is_empty()
        {
            return Ok(Some(SigningProperties {
                store_file,
                store_password,
                key_alias,
                key_password,
            }));
        }
    }

    load_signing_properties(project_root)
}

pub fn resolve_android_sdk_dir(project_root: &Path) -> Option<PathBuf> {
    let local_properties = read_properties(&project_root.join("local.properties")).ok()?;
    first_non_blank(&[
        local_properties.get("sdk.dir").cloned(),
        env::var("ANDROID_SDK_ROOT").ok(),
        env::var("ANDROID_HOME").ok(),
    ])
    .map(PathBuf::from)
}

pub fn resolve_latest_ndk_directory(android_sdk_dir: &Path) -> Option<PathBuf> {
    let ndk_root = android_sdk_dir.join("ndk");
    let entries = fs::read_dir(&ndk_root).ok()?;
    let mut best: Option<(Vec<i32>, PathBuf)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let revision =
            read_ndk_revision_parts(&path).unwrap_or_else(|| parse_revision_parts(&path));
        match &best {
            Some((current, _)) if compare_revision_parts(&revision, current) <= 0 => {}
            _ => best = Some((revision, path)),
        }
    }

    best.map(|(_, path)| path)
}

pub fn archive_directory(project_root: &Path) -> PathBuf {
    project_root.join("old_apks")
}

pub fn ensure_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

pub fn versioned_artifact_name(base_name: &str, version_name: &str, extension: &str) -> String {
    format!("{base_name}_v{version_name}.{extension}")
}

pub fn archive_file(source_path: &Path, archive_dir: &Path) -> io::Result<PathBuf> {
    ensure_directory(archive_dir)?;
    let metadata = fs::metadata(source_path)?;
    let timestamp = metadata
        .modified()
        .ok()
        .and_then(system_time_to_millis)
        .unwrap_or_else(current_time_millis);
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("package");
    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();

    let mut candidate = archive_dir.join(format!("{stem}_{timestamp}{extension}"));
    let mut suffix = 1_u32;
    while candidate.exists() {
        candidate = archive_dir.join(format!("{stem}_{timestamp}_{suffix}{extension}"));
        suffix = suffix.saturating_add(1);
    }

    move_file(source_path, &candidate)?;
    Ok(candidate)
}

pub fn archive_existing_root_packages(project_root: &Path, base_name: &str) -> io::Result<()> {
    let archive_dir = archive_directory(project_root);
    if !archive_dir.exists() {
        ensure_directory(&archive_dir)?;
    }

    for entry in fs::read_dir(project_root)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with(base_name) {
            continue;
        }
        if name == format!("{base_name}.apk") || name == format!("{base_name}.aab") {
            archive_file(&path, &archive_dir)?;
            continue;
        }
        if name.starts_with(&format!("{base_name}_v"))
            && (name.ends_with(".apk") || name.ends_with(".aab"))
        {
            archive_file(&path, &archive_dir)?;
        }
    }
    Ok(())
}

pub fn replace_file_with_copy(
    source: &Path,
    destination: &Path,
    archive_dir: &Path,
) -> io::Result<()> {
    if destination.exists() {
        archive_file(destination, archive_dir)?;
    }
    if let Some(parent) = destination.parent() {
        ensure_directory(parent)?;
    }
    fs::copy(source, destination)?;
    Ok(())
}

pub fn move_file(source: &Path, destination: &Path) -> io::Result<()> {
    if let Some(parent) = destination.parent() {
        ensure_directory(parent)?;
    }
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, destination)?;
            fs::remove_file(source)?;
            Ok(())
        }
    }
}

pub fn spawn_and_check(mut command: Command, label: &str) -> io::Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Other,
            format!("{label} failed with exit code {:?}", status.code()),
        ))
    }
}

pub fn cargo_toolchain_exe() -> PathBuf {
    PathBuf::from(r"C:\tools\rustup\toolchains\stable-x86_64-pc-windows-msvc\bin\cargo.exe")
}

pub fn gradle_home() -> PathBuf {
    PathBuf::from(r"C:\tools\gradle-8.7")
}

pub fn java_home() -> PathBuf {
    env::var("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"C:\tools\java\jdk-17.0.18+8"))
}

pub fn keytool_path() -> PathBuf {
    let candidate = java_home().join("bin/keytool.exe");
    if candidate.exists() {
        candidate
    } else {
        PathBuf::from("keytool.exe")
    }
}

pub fn apk_analyzer_path(android_sdk_dir: &Path) -> PathBuf {
    android_sdk_dir.join(r"cmdline-tools\latest\bin\apkanalyzer.bat")
}

pub fn apksigner_path(android_sdk_dir: &Path) -> PathBuf {
    android_sdk_dir.join(r"build-tools\34.0.0\apksigner.bat")
}

pub fn resolve_xiaomi_app_id(project_root: &Path) -> Option<String> {
    let local_properties = read_properties(&project_root.join("local.properties")).ok()?;
    first_non_blank(&[
        env::var("XIAOMI_APP_ID").ok(),
        local_properties.get("XIAOMI_APP_ID").cloned(),
        local_properties.get("xiaomi.app.id").cloned(),
    ])
}

pub fn current_time_millis() -> u128 {
    current_system_time()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn current_system_time() -> SystemTime {
    SystemTime::now()
}

fn system_time_to_millis(time: SystemTime) -> Option<u128> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|value| value.as_millis())
}

fn extract_quoted_value(text: &str, token: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(token) {
            continue;
        }
        let remainder = trimmed[token.len()..].trim_start();
        let quote = remainder.chars().find(|ch| *ch == '\'' || *ch == '"')?;
        let start = remainder.find(quote)? + 1;
        let tail = &remainder[start..];
        let end = tail.find(quote)?;
        return Some(tail[..end].to_string());
    }
    None
}

fn extract_integer_after_token(text: &str, token: &str) -> Option<i32> {
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with(token) {
            continue;
        }
        let remainder = trimmed[token.len()..].trim_start();
        let digits: String = remainder
            .chars()
            .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
            .collect();
        if digits.is_empty() {
            continue;
        }
        if let Ok(value) = digits.parse::<i32>() {
            return Some(value);
        }
    }
    None
}

fn parse_revision_parts(value: &Path) -> Vec<i32> {
    value
        .file_name()
        .and_then(|name| name.to_str())
        .map(parse_revision_text)
        .unwrap_or_default()
}

fn read_ndk_revision_parts(ndk_dir: &Path) -> Option<Vec<i32>> {
    let properties_path = ndk_dir.join("source.properties");
    if !properties_path.exists() {
        return None;
    }
    let text = fs::read_to_string(properties_path).ok()?;
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(revision) = trimmed.strip_prefix("Pkg.Revision=") {
            let parsed = parse_revision_text(revision);
            if !parsed.is_empty() {
                return Some(parsed);
            }
        }
    }
    None
}

fn parse_revision_text(value: &str) -> Vec<i32> {
    value
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse::<i32>().ok())
        .collect()
}

fn unescape_properties_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('t') => output.push('\t'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('f') => output.push('\u{000C}'),
            Some(':') => output.push(':'),
            Some('=') => output.push('='),
            Some(' ') => output.push(' '),
            Some('\\') => output.push('\\'),
            Some(other) => output.push(other),
            None => output.push('\\'),
        }
    }
    output
}

fn compare_revision_parts(left: &[i32], right: &[i32]) -> i32 {
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_part = left.get(index).copied().unwrap_or(0);
        let right_part = right.get(index).copied().unwrap_or(0);
        if left_part != right_part {
            return if left_part < right_part { -1 } else { 1 };
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_groovy_style_values() {
        let text = r#"
            applicationId 'com.example.app'
            applicationIdSuffix '.debug'
            versionName '1.2.3'
            versionCode 123
        "#;
        assert_eq!(
            Some("com.example.app".to_string()),
            extract_quoted_value(text, "applicationId")
        );
        assert_eq!(
            Some(".debug".to_string()),
            extract_quoted_value(text, "applicationIdSuffix")
        );
        assert_eq!(
            Some("1.2.3".to_string()),
            extract_quoted_value(text, "versionName")
        );
        assert_eq!(Some(123), extract_integer_after_token(text, "versionCode"));
    }
}
