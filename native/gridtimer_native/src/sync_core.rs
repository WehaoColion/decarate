// v1.0.3.15 Windows - Recover recently used active sessions before media and data sync.
// v1.0.3.7 Windows - Sanitize tombstoned historical media before ownership checks.
// v1.0.3.4 Windows - Keep sync available when legacy media removal needs recovery.
// v0.0.2 - Negotiate private metadata support and verify account-bound exchanges.
// v0.0.1 - Report retained attachment references as an explicit deletion conflict.
// v1.0.1 - Log startup stages so recovery delays can be located without restarting.
// v2.22.57 - Share the configured recovery directory with the startup supervisor.
// v2.22.56 - Enforce independent privacy fences before recovery; bound and verify local HTTP.
// v2.22.34 - Bind storage errors and report preserved legacy attachment gaps.
// Unit-test builds retain opt-in discovery/UPnP probes for targeted fixtures.
// Production builds still enforce the normal dead-code lint.
#![cfg_attr(test, allow(dead_code))]
// Android links only the client-facing sync/JNI surface from this shared module.
// Desktop server, discovery, backup, and HTTP helpers deliberately remain available
// to the non-Android binaries and their tests, so they are unused only on Android.
#![cfg_attr(target_os = "android", allow(dead_code, unused_imports))]

#[path = "sync_peer_policy.rs"]
mod peer_policy;
pub use peer_policy::LocalHttpPeerVerification;

#[path = "sync_http_deadline.rs"]
mod http_deadline;
#[cfg(target_os = "windows")]
#[path = "sync_windows_http.rs"]
mod windows_http;

#[cfg(not(target_os = "android"))]
#[path = "sync_backup_privacy.rs"]
mod backup_privacy;
#[cfg(not(target_os = "android"))]
#[path = "sync_backup_worker.rs"]
mod backup_worker;
#[cfg(not(target_os = "android"))]
#[path = "sync_legal_reports.rs"]
mod legal_reports;
#[cfg(not(target_os = "android"))]
#[path = "sync_private_media.rs"]
mod private_media;

#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
pub fn desktop_private_media_exchange(
    server_url: &str,
    user_id: &str,
    token: &str,
    server_instance_id: &str,
    account_namespace: &str,
    generation: i64,
    restore_receipt: &str,
    queries: Vec<crate::private_media_protocol::PrivateMediaQuery>,
) -> crate::private_media_protocol::PrivateMediaExchangeOutcome {
    private_media::exchange_client(
        server_url,
        user_id,
        token,
        server_instance_id,
        account_namespace,
        generation,
        restore_receipt,
        queries,
    )
}

use crate::app_data;
use crate::product_identity;
#[cfg(not(target_os = "android"))]
use crate::server_store::{
    valid_media_attachment_id, valid_media_mime_type, AuthenticatedToken,
    BoundSnapshotImportOutcome, DiscoveryTokenKeyLookup, MediaMetadata, MediaUpsertOutcome,
    NewStoredUser, PendingTokenActivation, ServerStoreOpenOptions, SnapshotHistoryQuotaStatus,
    SqliteServerStore, StoreError, StoredUser, SyncRequestOutcome, TokenAuthentication,
    ACCOUNT_APP_DATA_HARD_LIMIT_BYTES, DEFAULT_LEGACY_TOKEN_TTL_MILLIS, MAX_MEDIA_BYTES,
};
pub use crate::sync_identity::{account_namespace_identifier, ACCOUNT_NAMESPACE_DOMAIN};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{
    IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs,
    UdpSocket,
};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::{atomic::AtomicBool, mpsc};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SYNC_SERVER_BUILD_ID: &str = product_identity::PRODUCT.app_version;
pub const SYNC_PROTOCOL_VERSION: i64 = product_identity::PRODUCT.sync_protocol_version;

const DEFAULT_SYNC_PORT: u16 = 8917;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BODY_BYTES: usize = 24 * 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 24 * 1024 * 1024;
const SYNC_PAYLOAD_WARNING_BYTES: usize = 18 * 1024 * 1024;
const SYNC_PAYLOAD_CRITICAL_BYTES: usize = 22 * 1024 * 1024;
const SYNC_CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const SYNC_RESPONSE_READ_TIMEOUT: Duration = Duration::from_secs(90);
const SYNC_WRITE_TIMEOUT: Duration = Duration::from_secs(20);
const SYNC_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const SYNC_LOGOUT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const SYNC_DISCOVERY_WORKERS: usize = 32;
#[cfg(test)]
const SYNC_DISCOVERY_CONNECT_TIMEOUT: Duration = Duration::from_millis(220);
#[cfg(test)]
const SYNC_DISCOVERY_IO_TIMEOUT: Duration = Duration::from_millis(450);
#[cfg(test)]
const SYNC_DISCOVERY_TOTAL_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(test)]
const UPNP_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(test)]
const UPNP_HTTP_TIMEOUT: Duration = Duration::from_secs(4);
const SYNC_BEACON_PORT: u16 = 8918;
const MAX_ACTIVE_CONNECTIONS: usize = 32;
const LOGIN_RATE_WINDOW_MILLIS: i64 = 60_000;
const LOGIN_RATE_MAX_ATTEMPTS: usize = 12;
const TOKEN_TTL_MILLIS: i64 = 30 * 24 * 60 * 60 * 1_000;
const TOKEN_ACTIVATION_LEASE_MILLIS: i64 = 10 * 60 * 1_000;
const PENDING_TOKEN_RECOVERY_WINDOW_MILLIS: i64 = 48 * 60 * 60 * 1_000;
const ACTIVE_TOKEN_RECOVERY_GRACE_MILLIS: i64 = 48 * 60 * 60 * 1_000;
const ACTIVE_TOKEN_RECENT_ACTIVITY_MILLIS: i64 = 7 * 24 * 60 * 60 * 1_000;
const DESKTOP_ACTIVE_TOKEN_RECOVERY_GRACE_MILLIS: i64 = 72 * 60 * 60 * 1_000;
const DISCOVERY_NONCE_MIN_BYTES: usize = 16;
const DISCOVERY_NONCE_MAX_BYTES: usize = 256;
const STARTUP_BACKUP_RETAIN_COUNT: usize = 100;
const STARTUP_BACKUP_RETAIN_MILLIS: i64 = 90 * 24 * 60 * 60 * 1_000;
const STARTUP_BACKUP_MIN_INTERVAL_MILLIS: i64 = 24 * 60 * 60 * 1_000;
const STARTUP_BACKUP_MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
const STARTUP_BACKUP_STATE_FILE: &str = "server_store_startup_state_v1.json";
const STARTUP_BACKUP_STATE_VERSION: u32 = 2;
const RUNTIME_BACKUP_INTERVAL_MILLIS: i64 = 6 * 60 * 60 * 1_000;
const RUNTIME_BACKUP_MIN_INTERVAL_MILLIS: i64 = 60 * 1_000;
const RUNTIME_BACKUP_MAX_ATTEMPTS: usize = 6;
const RUNTIME_BACKUP_RETRY_BASE_MILLIS: u64 = 100;
const RUNTIME_BACKUP_RETRY_MAX_MILLIS: u64 = 1_600;
const RUNTIME_BACKUP_RETAIN_COUNT: usize = 120;
const RUNTIME_BACKUP_RETAIN_MILLIS: i64 = 90 * 24 * 60 * 60 * 1_000;
const RUNTIME_BACKUP_MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const RUNTIME_BACKUP_MANIFEST_VERSION: u32 = 1;
const RUNTIME_BACKUP_MANIFEST_SUFFIX: &str = ".manifest.json";
const RUNTIME_BACKUP_PENDING_VERSION: u32 = 1;
const RUNTIME_BACKUP_PENDING_PREFIX: &str = ".gridtimer_runtime_backup_pending_";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalReportManifestItem {
    pub report_id: String,
    pub created_at_epoch_millis: i64,
    pub sha256: String,
    pub size_bytes: i64,
    pub source_workspace_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegalReportTombstone {
    pub report_id: String,
    pub deleted_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncClientResult {
    pub ok: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub product_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub service_role: String,
    #[serde(default)]
    pub sync_protocol_version: i64,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub server_instance_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_process_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_build_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_git_commit: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_source_snapshot_sha256: String,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub server_process_id: u32,
    #[serde(default)]
    pub account_namespace: String,
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub workspace_proof: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub token_id: String,
    #[serde(default)]
    pub app_data_json: Option<String>,
    #[serde(default)]
    pub server_updated_at_epoch_millis: i64,
    #[serde(default)]
    pub client_updated_at_epoch_millis: i64,
    #[serde(default)]
    pub current_generation: i64,
    #[serde(default)]
    pub restore_required: bool,
    #[serde(default)]
    pub baseline_merge_required: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub workspace_identity_rebound: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub restore_receipt: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub resolved_server_url: String,
    #[serde(default)]
    pub public_server_url: String,
    #[serde(default)]
    pub public_access_message: String,
    #[serde(default)]
    pub backup_status: String,
    #[serde(default)]
    pub last_backup_success_at_epoch_millis: i64,
    #[serde(default)]
    pub last_backup_failure_at_epoch_millis: i64,
    #[serde(default)]
    pub backup_message: String,
    #[serde(default)]
    pub backup_privacy: BackupPrivacyStatus,
    #[serde(default)]
    pub snapshot_history_usage_bytes: i64,
    #[serde(default)]
    pub snapshot_history_projected_bytes: i64,
    #[serde(default)]
    pub snapshot_history_limit_bytes: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snapshot_history_warning: String,
    #[serde(default)]
    pub sync_payload_usage_bytes: i64,
    #[serde(default)]
    pub sync_payload_limit_bytes: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sync_payload_warning: String,
    #[serde(default)]
    pub app_data_current_bytes: i64,
    #[serde(default)]
    pub app_data_projected_bytes: i64,
    #[serde(default)]
    pub app_data_limit_bytes: i64,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default)]
    pub current_committed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media_items: Vec<MediaManifestItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legacy_media_references: Vec<LegacyMediaReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_item: Option<MediaManifestItem>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media_content_base64: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legal_reports: Vec<LegalReportManifestItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub legal_tombstones: Vec<LegalReportTombstone>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub legal_report_chunk_base64: String,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub legal_report_total_bytes: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub legal_report_sha256: String,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub legal_report_created_at_epoch_millis: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub legal_report_source_workspace_id: String,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub legal_report_received_bytes: usize,
    #[cfg(not(target_os = "android"))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_media_exchange: Option<crate::private_media_protocol::PrivateMediaReply>,
    #[cfg(not(target_os = "android"))]
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub private_media_protocol_version: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub nonce: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proof: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaManifestItem {
    pub attachment_id: String,
    pub sha256: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub updated_at_epoch_millis: i64,
    pub deleted_at_epoch_millis: i64,
}

/// Existing account references with no content hash. This is not a media blob
/// and must never be treated as proof that bytes can be downloaded.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyMediaReference {
    pub attachment_id: String,
    pub mime_type: String,
    pub size_bytes: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackupPrivacyStatus {
    pub status: String,
    pub last_success_at_epoch_millis: i64,
    pub last_failure_at_epoch_millis: i64,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
struct ServerRuntimeInfo {
    public_server_url: String,
    public_access_message: String,
    backup_status: String,
    last_backup_success_at_epoch_millis: i64,
    last_backup_failure_at_epoch_millis: i64,
    backup_message: String,
    backup_privacy: BackupPrivacyStatus,
    #[cfg(not(target_os = "android"))]
    backup_privacy_wake: Option<crate::server_store::BackupPrivacySubscription>,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupBackupState {
    #[serde(default)]
    state_version: u32,
    backup_file_name: String,
    source_sha256: String,
    backup_sha256: String,
    backup_size_bytes: u64,
    created_at_epoch_millis: i64,
    #[serde(default)]
    server_instance_id: String,
    #[serde(default)]
    target_store_fingerprint: String,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeBackupManifest {
    manifest_version: u32,
    backup_file_name: String,
    target_store_fingerprint: String,
    server_instance_id: String,
    backup_sha256: String,
    backup_size_bytes: u64,
    created_at_epoch_millis: i64,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PendingRuntimeBackup {
    state_version: u32,
    target_store_fingerprint: String,
    destination_file_name: String,
    temp_file_prefix: String,
    created_at_epoch_millis: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterRequest {
    #[serde(default)]
    request_id: String,
    email: String,
    password: String,
    device_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    #[serde(default)]
    request_id: String,
    email: String,
    password: String,
    device_name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSnapshotRequest {
    #[serde(default)]
    request_id: String,
    app_data_json: String,
    client_updated_at_epoch_millis: i64,
    #[serde(default)]
    acknowledged_generation: i64,
    #[serde(default)]
    restore_receipt: String,
    #[serde(default)]
    server_instance_id: String,
    #[serde(default)]
    account_namespace: String,
    #[serde(default)]
    workspace_id: String,
    #[serde(default)]
    workspace_proof: String,
    #[serde(default)]
    allow_workspace_identity_rebind: bool,
    #[serde(default)]
    previous_server_instance_id: String,
    #[serde(default)]
    previous_account_namespace: String,
    device_name: String,
    #[serde(default)]
    force_upload: bool,
    #[serde(default)]
    force_download: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscoveryProofRequest {
    user_id: String,
    token_id: String,
    nonce: String,
    #[serde(default)]
    client_proof: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaUploadRequest {
    request_id: String,
    #[serde(default)]
    acknowledged_generation: i64,
    #[serde(default)]
    restore_receipt: String,
    attachment_id: String,
    sha256: String,
    mime_type: String,
    size_bytes: i64,
    updated_at_epoch_millis: i64,
    #[serde(default)]
    restore_deleted: bool,
    content_base64: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaDownloadRequest {
    #[serde(default)]
    acknowledged_generation: i64,
    #[serde(default)]
    restore_receipt: String,
    attachment_id: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaDeleteRequest {
    request_id: String,
    #[serde(default)]
    acknowledged_generation: i64,
    #[serde(default)]
    restore_receipt: String,
    attachment_id: String,
    deleted_at_epoch_millis: i64,
}

#[cfg(not(target_os = "android"))]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DesktopMediaManifestRequest {
    #[serde(default)]
    acknowledged_generation: i64,
    #[serde(default)]
    restore_receipt: String,
}

#[cfg(test)]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerStore {
    users: Vec<ServerUser>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerUser {
    id: String,
    email: String,
    password_salt: String,
    password_hash: String,
    created_at_epoch_millis: i64,
    updated_at_epoch_millis: i64,
    app_data_json: String,
    tokens: Vec<ServerToken>,
}

#[cfg(test)]
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerToken {
    token: String,
    device_name: String,
    created_at_epoch_millis: i64,
    last_seen_at_epoch_millis: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncUrlScheme {
    Http,
    Https,
}

impl SyncUrlScheme {
    fn default_port(self) -> u16 {
        match self {
            SyncUrlScheme::Http => 80,
            SyncUrlScheme::Https => 443,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            SyncUrlScheme::Http => "http",
            SyncUrlScheme::Https => "https",
        }
    }
}

struct ParsedBaseUrl {
    scheme: SyncUrlScheme,
    host: String,
    port: u16,
    base_path: String,
}

#[derive(Debug)]
enum SyncClientIoError {
    Connect(io::Error),
    Send(io::Error),
    Read(io::Error),
}

pub fn default_server_url() -> String {
    format!("http://127.0.0.1:{DEFAULT_SYNC_PORT}")
}

/// Optional report exchange shared by Windows and Android clients.
pub fn legal_reports_request_json(
    server_url: &str,
    token: &str,
    operation: &str,
    request_json: &str,
) -> String {
    let endpoint = match operation {
        "manifest" => "/v1/legal-reports/manifest",
        "upload" => "/v1/legal-reports/upload",
        "download" => "/v1/legal-reports/download",
        "delete" => "/v1/legal-reports/delete",
        _ => return encode_result(&error_result("Unsupported legal report operation.")),
    };
    let body = match serde_json::from_str::<Value>(request_json) {
        Ok(value) if value.is_object() => value,
        _ => return encode_result(&error_result("Invalid legal report request.")),
    };
    encode_result(&post_json(server_url, endpoint, Some(token), body))
}

pub fn register_account_json(
    server_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> String {
    encode_result(&register_account_result(
        server_url,
        email,
        password,
        device_name,
    ))
}

pub fn login_account_json(
    server_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> String {
    encode_result(&login_account_result(
        server_url,
        email,
        password,
        device_name,
    ))
}

pub fn sync_app_data_json(
    server_url: &str,
    token: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    device_name: &str,
) -> String {
    sync_app_data_with_generation_json(
        server_url,
        token,
        app_data_json,
        client_updated_at_epoch_millis,
        device_name,
        0,
        "",
    )
}

pub fn sync_app_data_with_generation_json(
    server_url: &str,
    token: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    device_name: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
) -> String {
    sync_app_data_with_workspace_capability_json(
        server_url,
        token,
        "",
        "",
        "",
        "",
        app_data_json,
        client_updated_at_epoch_millis,
        acknowledged_generation,
        restore_receipt,
        device_name,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn sync_app_data_with_workspace_capability_json(
    server_url: &str,
    token: &str,
    server_instance_id: &str,
    account_namespace: &str,
    workspace_id: &str,
    workspace_proof: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    acknowledged_generation: i64,
    restore_receipt: &str,
    device_name: &str,
) -> String {
    encode_result(&sync_app_data_result(
        server_url,
        token,
        server_instance_id,
        account_namespace,
        workspace_id,
        workspace_proof,
        app_data_json,
        client_updated_at_epoch_millis,
        device_name,
        acknowledged_generation,
        restore_receipt,
        false,
        false,
    ))
}

pub fn upload_local_app_data_json(
    server_url: &str,
    token: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    device_name: &str,
) -> String {
    upload_local_app_data_with_generation_json(
        server_url,
        token,
        app_data_json,
        client_updated_at_epoch_millis,
        device_name,
        0,
        "",
    )
}

pub fn upload_local_app_data_with_generation_json(
    server_url: &str,
    token: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    device_name: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
) -> String {
    upload_local_app_data_with_workspace_capability_json(
        server_url,
        token,
        "",
        "",
        "",
        "",
        app_data_json,
        client_updated_at_epoch_millis,
        acknowledged_generation,
        restore_receipt,
        device_name,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn upload_local_app_data_with_workspace_capability_json(
    server_url: &str,
    token: &str,
    server_instance_id: &str,
    account_namespace: &str,
    workspace_id: &str,
    workspace_proof: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    acknowledged_generation: i64,
    restore_receipt: &str,
    device_name: &str,
) -> String {
    encode_result(&sync_app_data_result(
        server_url,
        token,
        server_instance_id,
        account_namespace,
        workspace_id,
        workspace_proof,
        app_data_json,
        client_updated_at_epoch_millis,
        device_name,
        acknowledged_generation,
        restore_receipt,
        true,
        false,
    ))
}

pub fn download_account_app_data_json(
    server_url: &str,
    token: &str,
    _app_data_json: &str,
    _client_updated_at_epoch_millis: i64,
    device_name: &str,
) -> String {
    download_account_app_data_with_generation_json(
        server_url,
        token,
        _app_data_json,
        _client_updated_at_epoch_millis,
        device_name,
        0,
        "",
    )
}

pub fn download_account_app_data_with_generation_json(
    server_url: &str,
    token: &str,
    _app_data_json: &str,
    _client_updated_at_epoch_millis: i64,
    device_name: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
) -> String {
    download_account_app_data_with_workspace_capability_json(
        server_url,
        token,
        "",
        "",
        "",
        "",
        _app_data_json,
        _client_updated_at_epoch_millis,
        acknowledged_generation,
        restore_receipt,
        device_name,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn download_account_app_data_with_workspace_capability_json(
    server_url: &str,
    token: &str,
    server_instance_id: &str,
    account_namespace: &str,
    workspace_id: &str,
    workspace_proof: &str,
    _app_data_json: &str,
    _client_updated_at_epoch_millis: i64,
    acknowledged_generation: i64,
    restore_receipt: &str,
    device_name: &str,
) -> String {
    let download_probe_json = app_data::default_app_data_json(0);
    encode_result(&sync_app_data_result(
        server_url,
        token,
        server_instance_id,
        account_namespace,
        workspace_id,
        workspace_proof,
        &download_probe_json,
        0,
        device_name,
        acknowledged_generation,
        restore_receipt,
        false,
        true,
    ))
}

/// Loads the authenticated account media manifest through the same hardened
/// transport used by desktop app-data sync.
#[cfg(not(target_os = "android"))]
pub fn desktop_media_manifest(
    server_url: &str,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
) -> SyncClientResult {
    if let Some(error) = validate_desktop_media_auth(
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        return error;
    }
    let request = DesktopMediaManifestRequest {
        acknowledged_generation: acknowledged_generation.max(0),
        restore_receipt: restore_receipt.trim().to_string(),
    };
    let result = post_json(
        server_url,
        "/v1/media/manifest",
        Some(token),
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    );
    validate_desktop_media_manifest_response(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    )
}

/// Uploads one attachment after validating the declared size and SHA-256
/// against the supplied bytes. Server-side restore barriers and tombstones are
/// preserved by forwarding the current generation and restore receipt.
#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
pub fn desktop_media_upload(
    server_url: &str,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
    attachment_id: &str,
    sha256: &str,
    mime_type: &str,
    size_bytes: i64,
    updated_at_epoch_millis: i64,
    content: &[u8],
    restore_deleted: bool,
) -> SyncClientResult {
    if let Some(error) = validate_desktop_media_auth(
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        return error;
    }
    let attachment_id = attachment_id.trim();
    if attachment_id.is_empty() {
        return desktop_media_client_error("Media attachmentId is required.");
    }
    let mime_type = mime_type.trim();
    if mime_type.is_empty() {
        return desktop_media_client_error("Media MIME type is required.");
    }
    if size_bytes < 1 || size_bytes as u128 > MAX_MEDIA_BYTES as u128 {
        return desktop_media_client_error("Media attachment is too large.");
    }
    if content.is_empty() || content.len() > MAX_MEDIA_BYTES {
        return desktop_media_client_error("Media attachment is too large.");
    }
    if size_bytes != content.len() as i64 {
        return desktop_media_client_error("attachment size does not match its content");
    }
    let sha256 = sha256.trim();
    if !valid_media_sha256(sha256) {
        return desktop_media_client_error("Media SHA-256 is invalid.");
    }
    let computed_sha256 = hex_bytes(&Sha256::digest(content));
    if !computed_sha256.eq_ignore_ascii_case(sha256) {
        return desktop_media_client_error("attachment SHA-256 does not match its content");
    }
    if updated_at_epoch_millis <= 0 {
        return desktop_media_client_error(
            "Media updatedAtEpochMillis must be a positive revision.",
        );
    }
    let request = MediaUploadRequest {
        request_id: random_token(18),
        acknowledged_generation: acknowledged_generation.max(0),
        restore_receipt: restore_receipt.trim().to_string(),
        attachment_id: attachment_id.to_string(),
        sha256: computed_sha256.clone(),
        mime_type: mime_type.to_string(),
        size_bytes,
        updated_at_epoch_millis,
        restore_deleted,
        content_base64: BASE64_STANDARD.encode(content),
    };
    let result = post_json(
        server_url,
        "/v1/media/upload",
        Some(token),
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    );
    validate_desktop_media_upload_response(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
        attachment_id,
        &computed_sha256,
        mime_type,
        size_bytes,
    )
}

/// Downloads one attachment and rejects a successful response unless its
/// attachment id, encoded length, and SHA-256 all match the returned metadata.
#[cfg(not(target_os = "android"))]
pub fn desktop_media_download(
    server_url: &str,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
    attachment_id: &str,
) -> SyncClientResult {
    if let Some(error) = validate_desktop_media_auth(
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        return error;
    }
    let attachment_id = attachment_id.trim();
    if attachment_id.is_empty() {
        return desktop_media_client_error("attachmentId is required.");
    }
    let request = MediaDownloadRequest {
        acknowledged_generation: acknowledged_generation.max(0),
        restore_receipt: restore_receipt.trim().to_string(),
        attachment_id: attachment_id.to_string(),
    };
    let result = post_json(
        server_url,
        "/v1/media/download",
        Some(token),
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    );
    validate_desktop_media_download_response(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
        attachment_id,
    )
}

/// Records a remote attachment tombstone using the authenticated account's
/// current restore barrier.
#[cfg(not(target_os = "android"))]
pub fn desktop_media_delete(
    server_url: &str,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
    attachment_id: &str,
    deleted_at_epoch_millis: i64,
) -> SyncClientResult {
    if let Some(error) = validate_desktop_media_auth(
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        return error;
    }
    let attachment_id = attachment_id.trim();
    if attachment_id.is_empty() {
        return desktop_media_client_error("Media attachmentId is required.");
    }
    if deleted_at_epoch_millis <= 0 {
        return desktop_media_client_error(
            "Media deletedAtEpochMillis must be a positive revision.",
        );
    }
    let request = MediaDeleteRequest {
        request_id: random_token(18),
        acknowledged_generation: acknowledged_generation.max(0),
        restore_receipt: restore_receipt.trim().to_string(),
        attachment_id: attachment_id.to_string(),
        deleted_at_epoch_millis,
    };
    let result = post_json(
        server_url,
        "/v1/media/delete",
        Some(token),
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    );
    validate_desktop_media_delete_response(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
        attachment_id,
        deleted_at_epoch_millis,
    )
}

pub fn app_data_revision_millis(raw: &str, fallback: i64) -> i64 {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return fallback.max(0);
    };
    app_data_revision_in_value(&value, fallback)
}

pub(crate) fn app_data_revision_in_value(value: &Value, fallback: i64) -> i64 {
    max_revision_in_value(value).unwrap_or_else(|| {
        if has_meaningful_app_data(value) {
            fallback.max(0)
        } else {
            0
        }
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AppDataSchemaStatus {
    Supported,
    Future(String),
    Invalid,
}

/// Reads only the generic JSON envelope. A future document must be identified
/// before serde deserializes it into today's typed model and drops unknown fields.
fn inspect_app_data_schema(raw: &str) -> AppDataSchemaStatus {
    if raw.trim().is_empty() {
        return AppDataSchemaStatus::Supported;
    }
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(raw) else {
        return AppDataSchemaStatus::Invalid;
    };
    let Some(version) = root.get("schemaVersion") else {
        return AppDataSchemaStatus::Supported;
    };
    let Value::Number(number) = version else {
        return AppDataSchemaStatus::Invalid;
    };
    if let Some(version) = number.as_i64() {
        if version > i64::from(app_data::APP_DATA_SCHEMA_VERSION) {
            AppDataSchemaStatus::Future(version.to_string())
        } else {
            AppDataSchemaStatus::Supported
        }
    } else if let Some(version) = number.as_u64() {
        if version > app_data::APP_DATA_SCHEMA_VERSION as u64 {
            AppDataSchemaStatus::Future(version.to_string())
        } else {
            AppDataSchemaStatus::Supported
        }
    } else {
        AppDataSchemaStatus::Invalid
    }
}

pub fn sanitize_sync_app_data(raw: &str, now: i64) -> Option<String> {
    if inspect_app_data_schema(raw) != AppDataSchemaStatus::Supported {
        return None;
    }
    app_data::sanitize_app_data_json(raw, now)
}

pub fn merge_sync_app_data_json(account_json: &str, local_json: &str, now: i64) -> Option<String> {
    #[cfg(not(target_os = "android"))]
    {
        return merge_sync_app_data_json_with_media_intents(
            account_json,
            local_json,
            now,
            &crate::note_media_intent::NoteAttachmentDetachments::new(),
            &std::collections::BTreeMap::new(),
        )
        .ok()
        .map(|plan| plan.app_data_json);
    }
    #[cfg(target_os = "android")]
    {
        if inspect_app_data_schema(account_json) != AppDataSchemaStatus::Supported
            || inspect_app_data_schema(local_json) != AppDataSchemaStatus::Supported
        {
            return None;
        }
        let mut account_value = serde_json::from_str::<Value>(account_json).ok()?;
        let mut local_value = serde_json::from_str::<Value>(local_json).ok()?;
        crate::finance_precision_guard::reconcile_peer(&account_value, &mut local_value).ok()?;
        merge_app_data_values(&mut account_value, &local_value);
        crate::knowledge::resolve_parent_cycles(&mut account_value);
        app_data::sanitize_app_data_json(&account_value.to_string(), now)
    }
}

#[cfg(not(target_os = "android"))]
pub(crate) struct NoteMediaMergePlan {
    pub app_data_json: String,
    pub detachments: crate::note_media_intent::NoteAttachmentDetachments,
    pub normalization: crate::note_media_intent::LegacyMediaNormalization,
}

/// Build a side-effect-free candidate. Storage must repeat the intent checks
/// against its transaction's current policy before authorizing the write.
#[cfg(not(target_os = "android"))]
pub(crate) fn merge_sync_app_data_json_with_media_intents(
    account_json: &str,
    local_raw_json: &str,
    now: i64,
    durable_detachments: &crate::note_media_intent::NoteAttachmentDetachments,
    proven_global_deletions: &std::collections::BTreeMap<String, i64>,
) -> Result<NoteMediaMergePlan, String> {
    use crate::note_media_intent::{
        infer_snapshot_detachments, infer_transition_detachments, merge_detachments,
        normalize_legacy_media_tombstones, project_current_detachments,
    };
    if inspect_app_data_schema(account_json) != AppDataSchemaStatus::Supported
        || inspect_app_data_schema(local_raw_json) != AppDataSchemaStatus::Supported
    {
        return Err("media intent merge requires a supported app data schema".into());
    }
    let account_json = if account_json.trim().is_empty() {
        app_data::default_app_data_json(0)
    } else {
        account_json.to_owned()
    };
    let before: Value =
        serde_json::from_str(&account_json).map_err(|_| "invalid account app data")?;
    let mut local: Value =
        serde_json::from_str(local_raw_json).map_err(|_| "invalid incoming app data")?;
    crate::finance_precision_guard::reconcile_peer(&before, &mut local)?;
    let mut detachments = durable_detachments.clone();
    merge_detachments(&mut detachments, &infer_snapshot_detachments(&before)?)?;
    merge_detachments(
        &mut detachments,
        &infer_transition_detachments(&before, &local)?,
    )?;
    // The opposite direction matters when an old peer sends a note which this
    // account already permanently deleted. Both values are same-account input.
    merge_detachments(
        &mut detachments,
        &infer_transition_detachments(&local, &before)?,
    )?;
    let mut merged = before.clone();
    merge_app_data_values(&mut merged, &local);
    project_current_detachments(&mut merged, &detachments)?;
    let normalization = normalize_legacy_media_tombstones(
        &before,
        &local,
        &mut merged,
        &detachments,
        proven_global_deletions,
    )?;
    if !normalization.unresolved_ids.is_empty() {
        return Err(
            "attachment deletion intent is ambiguous; original references require recovery".into(),
        );
    }
    crate::knowledge::resolve_parent_cycles(&mut merged);
    let app_data_json = app_data::sanitize_app_data_json(&merged.to_string(), now)
        .ok_or("app data could not be projected safely")?;
    Ok(NoteMediaMergePlan {
        app_data_json,
        detachments,
        normalization,
    })
}

#[cfg(not(target_os = "android"))]
pub fn run_sync_server(bind_addr: &str, store_path: &Path) -> io::Result<()> {
    // Claim the public endpoint before touching the database. A stale supervisor
    // must not be able to initialize or migrate the same store concurrently with
    // the server that already owns the endpoint.
    let listener = TcpListener::bind(bind_addr)?;
    let startup_started = std::time::Instant::now();
    eprintln!("sync startup: opening and validating storage");
    let (store, sqlite_path, startup_backup) = initialize_sqlite_store(store_path)?;
    let store = Arc::new(store);
    let recovery_directory = configured_runtime_backup_directory(&sqlite_path);
    eprintln!(
        "sync startup: initial recovery backup ({} ms elapsed)",
        startup_started.elapsed().as_millis()
    );
    let initial_runtime_backup = perform_initial_runtime_backup(&store, &recovery_directory)?;
    eprintln!(
        "sync startup: ready after {} ms",
        startup_started.elapsed().as_millis()
    );
    let mut configured_public_access = configured_public_runtime_info();
    configured_public_access.backup_status = "ok".to_string();
    configured_public_access.last_backup_success_at_epoch_millis =
        initial_runtime_backup.created_at_epoch_millis;
    configured_public_access.backup_message = format!(
        "Verified recovery backup completed before the server accepted requests ({} bytes).",
        initial_runtime_backup.size_bytes
    );
    configured_public_access.backup_privacy = BackupPrivacyStatus {
        status: "ok".into(),
        last_success_at_epoch_millis: now_millis(),
        message: "Managed startup, runtime and schema-14 archives checked during initialization."
            .into(),
        ..BackupPrivacyStatus::default()
    };
    let has_configured_public_access = !configured_public_access.public_server_url.is_empty();
    let runtime_info = Arc::new(Mutex::new(configured_public_access));
    let login_rate_limiter = Arc::new(Mutex::new(LoginRateLimiter::default()));
    let active_connections = Arc::new(AtomicUsize::new(0));
    println!("sync server listening on http://{bind_addr}");
    println!("sync store: {}", sqlite_path.display());
    println!("sync startup backup: {}", startup_backup.display());
    println!(
        "sync runtime recovery backups: {}",
        recovery_directory.display()
    );
    println!(
        "sync initial runtime recovery backup: {}",
        initial_runtime_backup.destination.display()
    );
    let _backup_worker = backup_worker::start(
        Arc::clone(&store),
        recovery_directory,
        Arc::clone(&runtime_info),
        configured_runtime_backup_interval(),
    )?;
    if has_configured_public_access {
        if let Ok(locked) = runtime_info.lock() {
            println!("sync public access: {}", locked.public_access_message);
        }
    } else if public_access_candidate_port(bind_addr).is_some() {
        println!("sync public access: waiting for an authenticated HTTPS tunnel");
    }
    if let Some(port) = public_access_candidate_port(bind_addr) {
        start_sync_beacon(port, Arc::clone(&runtime_info));
    }

    for incoming in listener.incoming() {
        let mut stream = incoming?;
        if active_connections
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE_CONNECTIONS).then_some(active + 1)
            })
            .is_err()
        {
            let _ = write_json_response(&mut stream, 503, &busy_server_result());
            continue;
        }
        let store = Arc::clone(&store);
        let runtime_info = Arc::clone(&runtime_info);
        let login_rate_limiter = Arc::clone(&login_rate_limiter);
        let active_connections = Arc::clone(&active_connections);
        thread::spawn(move || {
            let _permit = ActiveConnectionPermit(active_connections);
            if let Err(error) = handle_connection(stream, store, runtime_info, login_rate_limiter) {
                eprintln!("request failed: {error}");
            }
        });
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn initialize_sqlite_store(
    legacy_store_path: &Path,
) -> io::Result<(SqliteServerStore, PathBuf, PathBuf)> {
    initialize_sqlite_store_with_runtime_recovery_directory(legacy_store_path, None)
}

#[cfg(not(target_os = "android"))]
fn initialize_sqlite_store_with_runtime_recovery_directory(
    legacy_store_path: &Path,
    runtime_recovery_directory: Option<&Path>,
) -> io::Result<(SqliteServerStore, PathBuf, PathBuf)> {
    let now = now_millis();
    let sqlite_path = if legacy_store_path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("sqlite3"))
    {
        legacy_store_path.to_path_buf()
    } else {
        legacy_store_path.with_extension("sqlite3")
    };
    let runtime_recovery_directory = runtime_recovery_directory
        .map(Path::to_path_buf)
        .unwrap_or_else(|| configured_runtime_backup_directory(&sqlite_path));
    let recovered_from_backup =
        recover_missing_sqlite_store_if_needed(&sqlite_path, &runtime_recovery_directory, now)?;
    if let Some(source) = recovered_from_backup.as_ref() {
        eprintln!(
            "restored missing sync database from verified backup: {}",
            source.display()
        );
    }
    let opened = SqliteServerStore::open_with_options(ServerStoreOpenOptions {
        database_path: sqlite_path.clone(),
        legacy_json_path: None,
        now_epoch_millis: now,
        legacy_token_ttl_millis: DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
    })
    .map_err(store_io_error)?;
    if let Some(report) = opened.pre_schema_migration_backup.as_ref() {
        eprintln!(
            "verified pre-schema-migration backup: {} ({} bytes, sha256 {})",
            report.destination.display(),
            report.size_bytes,
            report.sha256
        );
    }
    let store = opened.store;
    let mut force_startup_backup = opened.schema_migrated || recovered_from_backup.is_some();
    // Restore consumed-import receipts before scanning legacy JSON again.
    // Signed ownership survives recovery from a database predating a rewrite.
    store.clean_legacy_privacy_files().map_err(store_io_error)?;
    let legacy_directory = legacy_store_path.parent().unwrap_or_else(|| Path::new("."));
    let legacy_paths =
        SqliteServerStore::discover_legacy_json_files(legacy_directory).map_err(store_io_error)?;
    let legacy_report = store
        .import_legacy_files_with_revision_merge(
            &legacy_paths,
            now,
            DEFAULT_LEGACY_TOKEN_TTL_MILLIS,
            merge_migration_app_data_json,
        )
        .map_err(store_io_error)?;
    force_startup_backup |= !legacy_report.files.is_empty();
    force_startup_backup |=
        import_windows_client_snapshot(&store, legacy_store_path, now).map_err(store_io_error)?;
    store.validate_integrity().map_err(store_io_error)?;
    let archive_started = std::time::Instant::now();
    eprintln!("sync startup: validating managed archive privacy");
    backup_privacy::clean(&store, &runtime_recovery_directory)?;
    eprintln!(
        "sync startup: archive privacy complete after {} ms; checking startup recovery copy",
        archive_started.elapsed().as_millis()
    );
    let startup_backup = ensure_startup_backup(&store, &sqlite_path, now, force_startup_backup)?;
    let pruned_media = store
        .prune_deleted_media_content(now)
        .map_err(store_io_error)?;
    if pruned_media > 0 {
        eprintln!("garbage-collected {pruned_media} expired deleted media blobs after backup");
    }
    store.validate_integrity().map_err(store_io_error)?;
    if let Err(error) = prune_old_startup_backups(&sqlite_path, now, &startup_backup) {
        eprintln!("could not prune old startup backups: {error}");
    }
    Ok((store, sqlite_path, startup_backup))
}

#[cfg(not(target_os = "android"))]
fn store_io_error(error: StoreError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(not(target_os = "android"))]
fn recover_missing_sqlite_store_if_needed(
    sqlite_path: &Path,
    runtime_backup_directory: &Path,
    now: i64,
) -> io::Result<Option<PathBuf>> {
    backup_privacy::resume(sqlite_path, runtime_backup_directory)?;
    if sqlite_path.exists() {
        return Ok(None);
    }
    let parent = sqlite_path.parent().unwrap_or_else(|| Path::new("."));
    let stale_wal = sync_sqlite_sidecar_path(sqlite_path, "-wal");
    let stale_shm = sync_sqlite_sidecar_path(sqlite_path, "-shm");
    if stale_wal.exists() || stale_shm.exists() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "sync database is missing but SQLite sidecars remain beside {}; refusing to initialize or overwrite recoverable evidence",
                sqlite_path.display()
            ),
        ));
    }

    let state_exists = parent.exists() && startup_backup_state_path(sqlite_path).exists();
    let mut owned_backup_evidence = false;
    if parent.exists() {
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && startup_backup_timestamp(&entry.file_name()).is_some()
            {
                owned_backup_evidence = true;
                break;
            }
        }
    }
    let valid_startup_state = if parent.exists() {
        load_valid_startup_backup_state(sqlite_path)?
    } else {
        None
    };
    let mut startup_candidates = if parent.exists() {
        verified_startup_backups(sqlite_path)?
    } else {
        Vec::new()
    };
    let mut expected_server_instance_id = valid_startup_state
        .as_ref()
        .map(|(state, _)| state.server_instance_id.clone());
    if expected_server_instance_id.is_none() && !state_exists {
        let startup_identities = startup_candidates
            .iter()
            .map(|(_, report)| report.server_instance_id.as_str())
            .collect::<HashSet<_>>();
        if startup_identities.len() == 1 {
            expected_server_instance_id = startup_identities.into_iter().next().map(str::to_string);
        }
    }
    if let Some(expected) = expected_server_instance_id.as_deref() {
        startup_candidates.retain(|(_, report)| report.server_instance_id == expected);
    } else {
        startup_candidates.clear();
    }
    let startup_candidate = startup_candidates
        .into_iter()
        .next()
        .map(|(timestamp, report)| (report, timestamp));
    let (runtime_backup_evidence, runtime_candidate) = latest_verified_runtime_backup(
        runtime_backup_directory,
        sqlite_path,
        expected_server_instance_id.as_deref(),
    )?;
    let prior_store_evidence = state_exists || owned_backup_evidence || runtime_backup_evidence;
    let candidate = match (startup_candidate, runtime_candidate) {
        (Some(startup), Some(runtime)) => {
            if runtime.1 > startup.1
                || (runtime.1 == startup.1 && runtime.0.destination > startup.0.destination)
            {
                Some(runtime)
            } else {
                Some(startup)
            }
        }
        (Some(candidate), None) | (None, Some(candidate)) => Some(candidate),
        (None, None) => None,
    };
    let Some((verified, timestamp)) = candidate else {
        if prior_store_evidence {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "sync database is missing and prior backup evidence exists, but no startup or runtime backup passed full integrity verification; refusing to create an empty database at {}",
                    sqlite_path.display()
                ),
            ));
        }
        return Ok(None);
    };

    restore_verified_backup_to_missing_database(
        &verified.destination,
        sqlite_path,
        timestamp,
        &verified.sha256,
        verified.size_bytes,
        now,
    )?;
    Ok(Some(verified.destination))
}

#[cfg(not(target_os = "android"))]
fn latest_verified_runtime_backup(
    backup_directory: &Path,
    sqlite_path: &Path,
    expected_server_instance_id: Option<&str>,
) -> io::Result<(
    bool,
    Option<(crate::server_store::VerifiedBackupReport, i64)>,
)> {
    if !backup_directory.exists() {
        return Ok((false, None));
    }
    let target_store_fingerprint = runtime_backup_target_fingerprint(sqlite_path)?;
    let mut evidence = false;
    let mut candidates = Vec::new();
    for entry in fs::read_dir(backup_directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if runtime_backup_manifest_identity_and_timestamp(&entry.file_name()).is_some() {
            evidence = true;
            continue;
        }
        let Some((server_instance_id, timestamp)) =
            runtime_backup_identity_and_timestamp(&entry.file_name())
        else {
            continue;
        };
        evidence = true;
        let path = entry.path();
        let Ok(report) = SqliteServerStore::verify_existing_backup(&path, timestamp) else {
            continue;
        };
        if report.server_instance_id != server_instance_id {
            continue;
        }
        let manifest_path = runtime_backup_manifest_path(&path);
        if manifest_path.exists() {
            if let Some(manifest) = load_valid_runtime_backup_manifest(&path, &report, timestamp)? {
                if manifest.target_store_fingerprint == target_store_fingerprint
                    && expected_server_instance_id
                        .is_none_or(|expected| report.server_instance_id == expected)
                {
                    candidates.push((timestamp, path, report));
                }
            }
        } else if expected_server_instance_id
            .is_some_and(|expected| report.server_instance_id == expected)
        {
            // A pre-manifest runtime copy is usable only when an independently
            // verified local startup state/backup anchors the target identity.
            candidates.push((timestamp, path, report));
        }
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    Ok((
        evidence,
        candidates
            .into_iter()
            .next()
            .map(|(timestamp, _, report)| (report, timestamp)),
    ))
}

#[cfg(not(target_os = "android"))]
fn restore_verified_backup_to_missing_database(
    backup_path: &Path,
    sqlite_path: &Path,
    backup_timestamp: i64,
    expected_sha256: &str,
    expected_size_bytes: u64,
    now: i64,
) -> io::Result<()> {
    if sqlite_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "refusing to overwrite an existing sync database during recovery",
        ));
    }
    let source = SqliteServerStore::verify_existing_backup(backup_path, backup_timestamp)
        .map_err(store_io_error)?;
    if source.sha256 != expected_sha256 || source.size_bytes != expected_size_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "verified startup backup changed before recovery",
        ));
    }
    let parent = sqlite_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = sqlite_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("server_store.sqlite3");
    let temp_path = (0..10_000_u32)
        .map(|sequence| {
            parent.join(format!(
                ".{file_name}.restore_incomplete_{now}_{sequence}.sqlite3"
            ))
        })
        .find(|candidate| !candidate.exists())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "could not allocate a temporary sync database recovery path",
            )
        })?;

    let recovery_result = (|| -> io::Result<()> {
        let mut input = fs::File::open(backup_path)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        let copied = SqliteServerStore::verify_existing_backup(&temp_path, backup_timestamp)
            .map_err(store_io_error)?;
        if copied.sha256 != expected_sha256 || copied.size_bytes != expected_size_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recovered sync database does not match the verified backup",
            ));
        }
        let protected = SqliteServerStore::apply_external_privacy_to_recovery_copy(
            &temp_path,
            sqlite_path,
            backup_timestamp,
        )
        .map_err(store_io_error)?;
        if sqlite_path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "sync database appeared during recovery; refusing to overwrite it",
            ));
        }
        fs::rename(&temp_path, sqlite_path)?;
        let published = SqliteServerStore::verify_existing_backup(sqlite_path, backup_timestamp)
            .map_err(store_io_error)?;
        if published.sha256 != protected.sha256 || published.size_bytes != protected.size_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "published sync database failed post-recovery verification",
            ));
        }
        Ok(())
    })();
    if recovery_result.is_err() && temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    recovery_result
}

#[cfg(not(target_os = "android"))]
fn sync_sqlite_sidecar_path(database_path: &Path, suffix: &str) -> PathBuf {
    let mut path = database_path.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

#[cfg(not(target_os = "android"))]
pub fn configured_runtime_backup_directory(sqlite_path: &Path) -> PathBuf {
    if let Some(configured) = std::env::var_os("GRID_TIMER_SYNC_BACKUP_DIR") {
        let configured = PathBuf::from(configured);
        if !configured.as_os_str().is_empty() {
            return configured;
        }
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data)
            .join("GridTimerRecovery")
            .join("sync_server_backups");
    }
    sqlite_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("GridTimerRecovery")
        .join("sync_server_backups")
}

#[cfg(not(target_os = "android"))]
fn configured_runtime_backup_interval() -> Duration {
    let millis = std::env::var("GRID_TIMER_SYNC_BACKUP_INTERVAL_MILLIS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(RUNTIME_BACKUP_INTERVAL_MILLIS)
        .clamp(RUNTIME_BACKUP_MIN_INTERVAL_MILLIS, 7 * 24 * 60 * 60 * 1_000);
    Duration::from_millis(millis as u64)
}

#[cfg(not(target_os = "android"))]
fn perform_initial_runtime_backup(
    store: &SqliteServerStore,
    backup_directory: &Path,
) -> io::Result<crate::server_store::VerifiedBackupReport> {
    retry_transient_runtime_backup(
        "initial runtime recovery backup",
        RUNTIME_BACKUP_MAX_ATTEMPTS,
        || perform_runtime_backup(store, backup_directory, now_millis()),
        |delay| thread::sleep(delay),
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "verified initial runtime recovery backup did not complete before the server could accept requests: {error}"
            ),
        )
    })
}

#[cfg(not(target_os = "android"))]
fn retry_transient_runtime_backup<T>(
    label: &str,
    max_attempts: usize,
    mut operation: impl FnMut() -> io::Result<T>,
    mut wait: impl FnMut(Duration),
) -> io::Result<T> {
    let max_attempts = max_attempts.max(1);
    for attempt in 1..=max_attempts {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) if attempt < max_attempts && is_transient_runtime_backup_error(&error) => {
                let delay = runtime_backup_retry_delay(attempt);
                eprintln!(
                    "{label} attempt {attempt}/{max_attempts} failed transiently: {error}; retrying in {} ms",
                    delay.as_millis()
                );
                wait(delay);
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("at least one runtime backup attempt always runs")
}

#[cfg(not(target_os = "android"))]
fn runtime_backup_retry_delay(failed_attempt: usize) -> Duration {
    let exponent = u32::try_from(failed_attempt.saturating_sub(1))
        .unwrap_or(u32::MAX)
        .min(20);
    let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
    Duration::from_millis(
        RUNTIME_BACKUP_RETRY_BASE_MILLIS
            .saturating_mul(multiplier)
            .min(RUNTIME_BACKUP_RETRY_MAX_MILLIS),
    )
}

#[cfg(not(target_os = "android"))]
fn is_transient_runtime_backup_error(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    ) {
        return true;
    }
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(32 | 33 | 170)) {
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_BUSY.
        return true;
    }
    let message = error.to_string().to_ascii_lowercase();
    [
        "disk i/o error",
        "database is locked",
        "database table is locked",
        "database is busy",
        "resource busy",
        "sharing violation",
        "temporarily unavailable",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(not(target_os = "android"))]
fn run_post_backup_maintenance(
    store: &SqliteServerStore,
    now_epoch_millis: i64,
) -> Result<usize, StoreError> {
    let pruned = store.prune_deleted_media_content(now_epoch_millis)?;
    store.validate_integrity()?;
    Ok(pruned)
}

#[cfg(not(target_os = "android"))]
fn perform_scheduled_runtime_backup(
    store: &SqliteServerStore,
    backup_directory: &Path,
    runtime_info: &Mutex<ServerRuntimeInfo>,
) {
    let now = now_millis();
    match retry_transient_runtime_backup(
        "runtime recovery backup",
        RUNTIME_BACKUP_MAX_ATTEMPTS,
        || perform_runtime_backup(store, backup_directory, now_millis()),
        |delay| thread::sleep(delay),
    ) {
        Ok(report) => {
            let maintenance = run_post_backup_maintenance(&store, report.created_at_epoch_millis);
            if let Err(error) = &maintenance {
                eprintln!(
                    "verified runtime backup succeeded, but post-backup maintenance failed: {error}"
                );
            }
            if let Ok(mut info) = runtime_info.lock() {
                info.last_backup_success_at_epoch_millis = report.created_at_epoch_millis;
                match maintenance {
                    Ok(pruned) => {
                        info.backup_status = "ok".to_string();
                        info.backup_message = format!(
                            "Verified recovery backup completed ({} bytes); post-backup integrity passed and {} expired media blob(s) were pruned.",
                            report.size_bytes, pruned
                        );
                    }
                    Err(error) => {
                        info.backup_status = "warning".to_string();
                        info.backup_message = format!(
                            "Verified recovery backup completed ({} bytes), but post-backup maintenance needs attention: {}",
                            report.size_bytes,
                            error.to_string().chars().take(320).collect::<String>()
                        );
                    }
                }
            }
        }
        Err(error) => {
            eprintln!("runtime recovery backup failed: {error}");
            if let Ok(mut info) = runtime_info.lock() {
                info.backup_status = "error".to_string();
                info.last_backup_failure_at_epoch_millis = now;
                info.backup_message = format!(
                    "Runtime recovery backup failed: {}",
                    error.to_string().chars().take(384).collect::<String>()
                );
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn perform_runtime_backup(
    store: &SqliteServerStore,
    backup_directory: &Path,
    now: i64,
) -> io::Result<crate::server_store::VerifiedBackupReport> {
    fs::create_dir_all(backup_directory)?;
    let server_instance_id = store.server_instance_id().map_err(store_io_error)?;
    let target_store_fingerprint = runtime_backup_target_fingerprint(store.database_path())?;
    cleanup_pending_runtime_backup(backup_directory, &target_store_fingerprint)?;
    backup_privacy::clean(store, backup_directory)?;
    let destination = unique_runtime_backup_path(backup_directory, &server_instance_id, now)?;
    let pending = PendingRuntimeBackup {
        state_version: RUNTIME_BACKUP_PENDING_VERSION,
        target_store_fingerprint: target_store_fingerprint.clone(),
        destination_file_name: destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        temp_file_prefix: runtime_backup_temp_file_prefix(&destination, now),
        created_at_epoch_millis: now,
    };
    write_pending_runtime_backup(backup_directory, &pending)?;
    let report = match store.create_verified_backup(&destination, now) {
        Ok(report) => report,
        Err(error) => {
            let backup_error = store_io_error(error);
            return match cleanup_pending_runtime_backup(
                backup_directory,
                &target_store_fingerprint,
            ) {
                Ok(_) => Err(backup_error),
                Err(cleanup_error) => Err(io::Error::new(
                    backup_error.kind(),
                    format!(
                        "{backup_error}; additionally could not clean the interrupted backup: {cleanup_error}"
                    ),
                )),
            };
        }
    };
    let manifest = RuntimeBackupManifest {
        manifest_version: RUNTIME_BACKUP_MANIFEST_VERSION,
        backup_file_name: report
            .destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        target_store_fingerprint: target_store_fingerprint.clone(),
        server_instance_id: report.server_instance_id.clone(),
        backup_sha256: report.sha256.clone(),
        backup_size_bytes: report.size_bytes,
        created_at_epoch_millis: now,
    };
    if let Err(error) = write_runtime_backup_manifest(&report.destination, &manifest) {
        let _ = fs::remove_file(&report.destination);
        let _ = cleanup_pending_runtime_backup(backup_directory, &target_store_fingerprint);
        return Err(error);
    }
    cleanup_pending_runtime_backup(backup_directory, &target_store_fingerprint)?;
    if let Err(error) = prune_runtime_backups(
        backup_directory,
        &server_instance_id,
        &target_store_fingerprint,
        now,
        &report.destination,
    ) {
        // Rotation is maintenance. A verified copy must remain successful even
        // when deleting an older copy is temporarily blocked.
        eprintln!("could not rotate runtime recovery backups: {error}");
    }
    Ok(report)
}

#[cfg(not(target_os = "android"))]
fn pending_runtime_backup_path(
    backup_directory: &Path,
    target_store_fingerprint: &str,
) -> io::Result<PathBuf> {
    if !valid_runtime_backup_binding(target_store_fingerprint) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pending runtime backup requires a valid target fingerprint",
        ));
    }
    Ok(backup_directory.join(format!(
        "{RUNTIME_BACKUP_PENDING_PREFIX}{target_store_fingerprint}.json"
    )))
}

#[cfg(not(target_os = "android"))]
fn runtime_backup_temp_file_prefix(destination: &Path, now: i64) -> String {
    let destination_key = hex_bytes(&Sha256::digest(
        destination.as_os_str().to_string_lossy().as_bytes(),
    ));
    format!(".{}.incomplete_{now}_", &destination_key[..16])
}

#[cfg(not(target_os = "android"))]
fn write_pending_runtime_backup(
    backup_directory: &Path,
    pending: &PendingRuntimeBackup,
) -> io::Result<()> {
    let destination =
        pending_runtime_backup_path(backup_directory, &pending.target_store_fingerprint)?;
    let raw = serde_json::to_vec_pretty(pending).map_err(io::Error::other)?;
    let temp = backup_directory.join(format!(
        ".runtime-backup-pending-{}-{}.tmp",
        std::process::id(),
        random_token(8)
    ));
    let write_result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&raw)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &destination)
    })();
    if write_result.is_err() && temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

#[cfg(not(target_os = "android"))]
fn cleanup_pending_runtime_backup(
    backup_directory: &Path,
    target_store_fingerprint: &str,
) -> io::Result<usize> {
    let marker_path = pending_runtime_backup_path(backup_directory, target_store_fingerprint)?;
    let raw = match fs::read(&marker_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let pending = serde_json::from_slice::<PendingRuntimeBackup>(&raw).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("pending runtime backup marker is invalid: {error}"),
        )
    })?;
    let destination = backup_directory.join(&pending.destination_file_name);
    let parsed_timestamp =
        runtime_backup_identity_and_timestamp(std::ffi::OsStr::new(&pending.destination_file_name))
            .map(|(_, timestamp)| timestamp);
    if pending.state_version != RUNTIME_BACKUP_PENDING_VERSION
        || pending.target_store_fingerprint != target_store_fingerprint
        || destination.file_name().and_then(|value| value.to_str())
            != Some(pending.destination_file_name.as_str())
        || parsed_timestamp != Some(pending.created_at_epoch_millis)
        || pending.temp_file_prefix
            != runtime_backup_temp_file_prefix(&destination, pending.created_at_epoch_millis)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pending runtime backup marker failed identity validation",
        ));
    }

    let mut removed = 0_usize;
    for entry in fs::read_dir(backup_directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if pending_runtime_backup_temp_name(name, &pending.temp_file_prefix) {
            fs::remove_file(entry.path())?;
            removed = removed.saturating_add(1);
        }
    }
    fs::remove_file(marker_path)?;
    Ok(removed)
}

#[cfg(not(target_os = "android"))]
fn pending_runtime_backup_temp_name(file_name: &str, prefix: &str) -> bool {
    let Some(remainder) = file_name.strip_prefix(prefix) else {
        return false;
    };
    [
        ".sqlite3",
        ".sqlite3-wal",
        ".sqlite3-shm",
        ".sqlite3-journal",
    ]
    .iter()
    .any(|suffix| {
        remainder.strip_suffix(suffix).is_some_and(|sequence| {
            !sequence.is_empty() && sequence.bytes().all(|byte| byte.is_ascii_digit())
        })
    })
}

#[cfg(not(target_os = "android"))]
fn unique_runtime_backup_path(
    backup_directory: &Path,
    server_instance_id: &str,
    now: i64,
) -> io::Result<PathBuf> {
    for sequence in 0..10_000_u32 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("_{sequence}")
        };
        let path = backup_directory.join(format!(
            "sync_server_{server_instance_id}_runtime_{now}{suffix}.sqlite3"
        ));
        if !path.exists() && !runtime_backup_manifest_path(&path).exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate runtime backup path",
    ))
}

#[cfg(not(target_os = "android"))]
fn runtime_backup_timestamp(file_name: &std::ffi::OsStr, server_instance_id: &str) -> Option<i64> {
    let (candidate_server_instance_id, timestamp) =
        runtime_backup_identity_and_timestamp(file_name)?;
    (candidate_server_instance_id == server_instance_id).then_some(timestamp)
}

#[cfg(not(target_os = "android"))]
fn runtime_backup_identity_and_timestamp(file_name: &std::ffi::OsStr) -> Option<(String, i64)> {
    let file_name = file_name.to_str()?;
    let stem = file_name
        .strip_prefix("sync_server_")?
        .strip_suffix(".sqlite3")?;
    let (server_instance_id, stem) = stem.split_once("_runtime_")?;
    if server_instance_id.len() != 64
        || !server_instance_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let (timestamp, sequence) = stem
        .split_once('_')
        .map(|(timestamp, sequence)| (timestamp, Some(sequence)))
        .unwrap_or((stem, None));
    if timestamp.is_empty()
        || !timestamp.bytes().all(|byte| byte.is_ascii_digit())
        || sequence.is_some_and(|value| {
            value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    Some((server_instance_id.to_string(), timestamp.parse().ok()?))
}

#[cfg(not(target_os = "android"))]
fn runtime_backup_manifest_path(backup_path: &Path) -> PathBuf {
    let mut file_name = backup_path.file_name().unwrap_or_default().to_os_string();
    file_name.push(RUNTIME_BACKUP_MANIFEST_SUFFIX);
    backup_path.with_file_name(file_name)
}

#[cfg(not(target_os = "android"))]
fn runtime_backup_manifest_identity_and_timestamp(
    file_name: &std::ffi::OsStr,
) -> Option<(String, i64)> {
    let file_name = file_name.to_str()?;
    let backup_file_name = file_name.strip_suffix(RUNTIME_BACKUP_MANIFEST_SUFFIX)?;
    runtime_backup_identity_and_timestamp(std::ffi::OsStr::new(backup_file_name))
}

#[cfg(not(target_os = "android"))]
fn runtime_backup_target_fingerprint(sqlite_path: &Path) -> io::Result<String> {
    let absolute = if sqlite_path.is_absolute() {
        sqlite_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(sqlite_path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                let _ = normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let mut stable_path = normalized.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        stable_path = stable_path.to_lowercase();
    }
    let mut hasher = Sha256::new();
    hasher.update(b"gridtimer-runtime-backup-target-v1\0");
    hasher.update(stable_path.as_bytes());
    Ok(hex_bytes(&hasher.finalize()))
}

#[cfg(not(target_os = "android"))]
fn valid_runtime_backup_binding(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(not(target_os = "android"))]
fn load_valid_runtime_backup_manifest(
    backup_path: &Path,
    report: &crate::server_store::VerifiedBackupReport,
    timestamp: i64,
) -> io::Result<Option<RuntimeBackupManifest>> {
    let manifest_path = runtime_backup_manifest_path(backup_path);
    let raw = match fs::read(&manifest_path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let manifest = match serde_json::from_slice::<RuntimeBackupManifest>(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let expected_file_name = backup_path.file_name().and_then(|value| value.to_str());
    if manifest.manifest_version != RUNTIME_BACKUP_MANIFEST_VERSION
        || expected_file_name != Some(manifest.backup_file_name.as_str())
        || Path::new(&manifest.backup_file_name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(manifest.backup_file_name.as_str())
        || !valid_runtime_backup_binding(&manifest.target_store_fingerprint)
        || !valid_runtime_backup_binding(&manifest.server_instance_id)
        || !valid_runtime_backup_binding(&manifest.backup_sha256)
        || manifest.server_instance_id != report.server_instance_id
        || manifest.backup_sha256 != report.sha256
        || manifest.backup_size_bytes != report.size_bytes
        || manifest.created_at_epoch_millis != timestamp
    {
        return Ok(None);
    }
    Ok(Some(manifest))
}

#[cfg(not(target_os = "android"))]
fn write_runtime_backup_manifest(
    backup_path: &Path,
    manifest: &RuntimeBackupManifest,
) -> io::Result<()> {
    let destination = runtime_backup_manifest_path(backup_path);
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let raw = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
    let temp = parent.join(format!(
        ".{}.{}.{}.tmp",
        destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("runtime-backup-manifest"),
        std::process::id(),
        random_token(8)
    ));
    let write_result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&raw)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, &destination)
    })();
    if write_result.is_err() && temp.exists() {
        let _ = fs::remove_file(&temp);
    }
    write_result
}

#[cfg(not(target_os = "android"))]
fn remove_runtime_backup_with_manifest(backup_path: &Path) -> io::Result<()> {
    fs::remove_file(backup_path)?;
    let manifest_path = runtime_backup_manifest_path(backup_path);
    match fs::remove_file(manifest_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(not(target_os = "android"))]
fn prune_runtime_backups(
    backup_directory: &Path,
    server_instance_id: &str,
    target_store_fingerprint: &str,
    now: i64,
    verified_backup_to_keep: &Path,
) -> io::Result<usize> {
    prune_runtime_backups_with_limits(
        backup_directory,
        server_instance_id,
        target_store_fingerprint,
        now,
        verified_backup_to_keep,
        RUNTIME_BACKUP_RETAIN_COUNT,
        RUNTIME_BACKUP_RETAIN_MILLIS,
        RUNTIME_BACKUP_MAX_TOTAL_BYTES,
    )
}

#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
fn prune_runtime_backups_with_limits(
    backup_directory: &Path,
    server_instance_id: &str,
    target_store_fingerprint: &str,
    now: i64,
    verified_backup_to_keep: &Path,
    retain_count: usize,
    retain_millis: i64,
    max_total_bytes: u64,
) -> io::Result<usize> {
    if !valid_runtime_backup_binding(server_instance_id)
        || !valid_runtime_backup_binding(target_store_fingerprint)
        || retain_count == 0
        || retain_millis < 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime backup rotation requires valid identity binding and positive limits",
        ));
    }
    let mut backups = Vec::new();
    for entry in fs::read_dir(backup_directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(timestamp) = runtime_backup_timestamp(&entry.file_name(), server_instance_id)
        else {
            continue;
        };
        let path = entry.path();
        if !runtime_backup_manifest_path(&path).is_file() {
            continue;
        }
        let Ok(report) = SqliteServerStore::verify_existing_backup(&path, timestamp) else {
            continue;
        };
        if report.server_instance_id != server_instance_id {
            continue;
        }
        let Some(manifest) = load_valid_runtime_backup_manifest(&path, &report, timestamp)? else {
            continue;
        };
        if manifest.target_store_fingerprint != target_store_fingerprint {
            continue;
        }
        backups.push((timestamp, path, report.size_bytes));
    }
    backups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let protected_index = backups
        .iter()
        .position(|(_, path, _)| path == verified_backup_to_keep)
        .or_else(|| (!backups.is_empty()).then_some(0));
    let cutoff = now.saturating_sub(retain_millis);
    let mut removed = vec![false; backups.len()];
    let mut deleted = 0;
    for (index, (timestamp, path, _)) in backups.iter().enumerate() {
        if Some(index) != protected_index && index >= retain_count && *timestamp < cutoff {
            remove_runtime_backup_with_manifest(path)?;
            removed[index] = true;
            deleted += 1;
        }
    }
    let mut total_bytes = backups
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed[*index])
        .map(|(_, (_, _, size))| *size)
        .sum::<u64>();
    for index in (0..backups.len()).rev() {
        if total_bytes <= max_total_bytes {
            break;
        }
        if removed[index] || Some(index) == protected_index {
            continue;
        }
        remove_runtime_backup_with_manifest(&backups[index].1)?;
        removed[index] = true;
        total_bytes = total_bytes.saturating_sub(backups[index].2);
        deleted += 1;
    }
    Ok(deleted)
}

#[cfg(not(target_os = "android"))]
fn ensure_startup_backup(
    store: &SqliteServerStore,
    sqlite_path: &Path,
    now: i64,
    force: bool,
) -> io::Result<PathBuf> {
    let source_sha256 = store.stable_database_sha256().map_err(store_io_error)?;
    if let Some((state, backup_path)) = load_valid_startup_backup_state(sqlite_path)? {
        let unchanged = state.source_sha256 == source_sha256;
        let within_interval =
            now.saturating_sub(state.created_at_epoch_millis) < STARTUP_BACKUP_MIN_INTERVAL_MILLIS;
        if !force && (unchanged || within_interval) {
            return Ok(backup_path);
        }
    } else if !force {
        if let Some((report, timestamp)) = latest_verified_startup_backup(sqlite_path)? {
            if now.saturating_sub(timestamp) < STARTUP_BACKUP_MIN_INTERVAL_MILLIS {
                return Ok(report.destination);
            }
        }
    }

    let destination = unique_startup_backup_path(sqlite_path, now)?;
    let report = store
        .create_verified_backup(&destination, now)
        .map_err(store_io_error)?;
    let state = StartupBackupState {
        state_version: STARTUP_BACKUP_STATE_VERSION,
        backup_file_name: report
            .destination
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_string(),
        source_sha256,
        backup_sha256: report.sha256,
        backup_size_bytes: report.size_bytes,
        created_at_epoch_millis: now,
        server_instance_id: report.server_instance_id,
        target_store_fingerprint: runtime_backup_target_fingerprint(sqlite_path)?,
    };
    write_startup_backup_state(sqlite_path, &state)?;
    Ok(report.destination)
}

#[cfg(not(target_os = "android"))]
fn startup_backup_state_path(sqlite_path: &Path) -> PathBuf {
    sqlite_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(STARTUP_BACKUP_STATE_FILE)
}

#[cfg(not(target_os = "android"))]
fn load_valid_startup_backup_state(
    sqlite_path: &Path,
) -> io::Result<Option<(StartupBackupState, PathBuf)>> {
    let state_path = startup_backup_state_path(sqlite_path);
    if !state_path.exists() {
        return Ok(None);
    }
    let raw = match fs::read(&state_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut state = match serde_json::from_slice::<StartupBackupState>(&raw) {
        Ok(state) => state,
        Err(_) => return Ok(None),
    };
    if state.source_sha256.len() != 64
        || state.backup_sha256.len() != 64
        || !state
            .source_sha256
            .bytes()
            .chain(state.backup_sha256.bytes())
            .all(|value| value.is_ascii_hexdigit())
        || startup_backup_timestamp(std::ffi::OsStr::new(&state.backup_file_name)).is_none()
        || Path::new(&state.backup_file_name)
            .file_name()
            .and_then(|value| value.to_str())
            != Some(state.backup_file_name.as_str())
    {
        return Ok(None);
    }
    let backup_path = sqlite_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&state.backup_file_name);
    let report = match SqliteServerStore::verify_existing_backup(
        &backup_path,
        state.created_at_epoch_millis,
    ) {
        Ok(report) => report,
        Err(_) => return Ok(None),
    };
    if report.sha256 != state.backup_sha256 || report.size_bytes != state.backup_size_bytes {
        return Ok(None);
    }
    let target_store_fingerprint = runtime_backup_target_fingerprint(sqlite_path)?;
    match state.state_version {
        0 => {
            if !state.server_instance_id.is_empty() || !state.target_store_fingerprint.is_empty() {
                return Ok(None);
            }
            // V1 states predate explicit identity fields. The state already
            // binds one exact local file by name, size and hash, so its verified
            // database identity can safely be derived without guessing.
            state.server_instance_id = report.server_instance_id;
            state.target_store_fingerprint = target_store_fingerprint;
        }
        STARTUP_BACKUP_STATE_VERSION => {
            if !valid_runtime_backup_binding(&state.server_instance_id)
                || !valid_runtime_backup_binding(&state.target_store_fingerprint)
                || state.server_instance_id != report.server_instance_id
                || state.target_store_fingerprint != target_store_fingerprint
            {
                return Ok(None);
            }
        }
        _ => return Ok(None),
    }
    Ok(Some((state, backup_path)))
}

#[cfg(not(target_os = "android"))]
fn latest_verified_startup_backup(
    sqlite_path: &Path,
) -> io::Result<Option<(crate::server_store::VerifiedBackupReport, i64)>> {
    Ok(verified_startup_backups(sqlite_path)?
        .into_iter()
        .next()
        .map(|(timestamp, report)| (report, timestamp)))
}

#[cfg(not(target_os = "android"))]
fn verified_startup_backups(
    sqlite_path: &Path,
) -> io::Result<Vec<(i64, crate::server_store::VerifiedBackupReport)>> {
    let parent = sqlite_path.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            Some((startup_backup_timestamp(&entry.file_name())?, entry.path()))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let mut verified = Vec::new();
    for (timestamp, path) in candidates {
        if let Ok(report) = SqliteServerStore::verify_existing_backup(&path, timestamp) {
            verified.push((timestamp, report));
        }
    }
    Ok(verified)
}

#[cfg(not(target_os = "android"))]
fn write_startup_backup_state(sqlite_path: &Path, state: &StartupBackupState) -> io::Result<()> {
    let destination = startup_backup_state_path(sqlite_path);
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let raw = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let temp = parent.join(format!(
        ".{STARTUP_BACKUP_STATE_FILE}.{}.tmp",
        std::process::id()
    ));
    let mut file = fs::File::create(&temp)?;
    file.write_all(&raw)?;
    file.sync_all()?;
    drop(file);
    if destination.exists() {
        fs::remove_file(&destination)?;
    }
    match fs::rename(&temp, &destination) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

#[cfg(not(target_os = "android"))]
fn unique_startup_backup_path(sqlite_path: &Path, now: i64) -> io::Result<PathBuf> {
    let parent = sqlite_path.parent().unwrap_or_else(|| Path::new("."));
    for sequence in 0..10_000_u32 {
        let suffix = if sequence == 0 {
            String::new()
        } else {
            format!("_{sequence}")
        };
        let path = parent.join(format!("server_store_startup_{now}{suffix}.sqlite3"));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate startup backup path",
    ))
}

#[cfg(not(target_os = "android"))]
fn prune_old_startup_backups(
    sqlite_path: &Path,
    now: i64,
    verified_backup_to_keep: &Path,
) -> io::Result<usize> {
    prune_startup_backups_with_limits(
        sqlite_path,
        now,
        STARTUP_BACKUP_RETAIN_COUNT,
        STARTUP_BACKUP_RETAIN_MILLIS,
        STARTUP_BACKUP_MAX_TOTAL_BYTES,
        Some(verified_backup_to_keep),
    )
}

#[cfg(not(target_os = "android"))]
fn prune_startup_backups_with_limits(
    sqlite_path: &Path,
    now: i64,
    retain_count: usize,
    retain_millis: i64,
    max_total_bytes: u64,
    verified_backup_to_keep: Option<&Path>,
) -> io::Result<usize> {
    let parent = sqlite_path.parent().unwrap_or_else(|| Path::new("."));
    let mut backups = fs::read_dir(parent)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_type = entry.file_type().ok()?;
            if !file_type.is_file() {
                return None;
            }
            let timestamp = startup_backup_timestamp(&entry.file_name())?;
            let size = entry.metadata().ok()?.len();
            Some((timestamp, entry.path(), size))
        })
        .collect::<Vec<_>>();
    backups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let protected_index = verified_backup_to_keep
        .and_then(|protected| backups.iter().position(|(_, path, _)| path == protected))
        .or_else(|| (!backups.is_empty()).then_some(0));
    let cutoff = now.saturating_sub(retain_millis);
    let mut removed = vec![false; backups.len()];
    let mut deleted = 0;
    for (index, (timestamp, path, _)) in backups.iter().enumerate() {
        if Some(index) != protected_index && index >= retain_count.max(1) && *timestamp < cutoff {
            fs::remove_file(path)?;
            removed[index] = true;
            deleted += 1;
        }
    }
    let mut total_bytes = backups
        .iter()
        .enumerate()
        .filter(|(index, _)| !removed[*index])
        .map(|(_, (_, _, size))| *size)
        .sum::<u64>();
    for index in (0..backups.len()).rev() {
        if total_bytes <= max_total_bytes {
            break;
        }
        if removed[index] || Some(index) == protected_index {
            continue;
        }
        fs::remove_file(&backups[index].1)?;
        removed[index] = true;
        total_bytes = total_bytes.saturating_sub(backups[index].2);
        deleted += 1;
    }
    Ok(deleted)
}

fn startup_backup_timestamp(file_name: &std::ffi::OsStr) -> Option<i64> {
    let file_name = file_name.to_str()?;
    let stem = file_name
        .strip_prefix("server_store_startup_")?
        .strip_suffix(".sqlite3")?;
    let (timestamp, sequence) = stem
        .split_once('_')
        .map(|(timestamp, sequence)| (timestamp, Some(sequence)))
        .unwrap_or((stem, None));
    if timestamp.is_empty()
        || !timestamp.bytes().all(|value| value.is_ascii_digit())
        || sequence.is_some_and(|value| {
            value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return None;
    }
    timestamp.parse::<i64>().ok()
}

#[cfg(not(target_os = "android"))]
fn import_windows_client_snapshot(
    store: &SqliteServerStore,
    legacy_store_path: &Path,
    now: i64,
) -> Result<bool, StoreError> {
    let client_root = match std::env::var_os("GRID_TIMER_WINDOWS_CLIENT_DIR") {
        Some(path) => PathBuf::from(path),
        None => {
            let is_default_sync_directory = legacy_store_path
                .parent()
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.eq_ignore_ascii_case("GridTimerSync"));
            if !is_default_sync_directory {
                return Ok(false);
            }
            let Some(base) =
                std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("USERPROFILE"))
            else {
                return Ok(false);
            };
            PathBuf::from(base).join("GridTimerClient")
        }
    };
    import_windows_client_snapshot_from_root(store, &client_root, now)
}

#[cfg(not(target_os = "android"))]
fn import_windows_client_snapshot_from_root(
    store: &SqliteServerStore,
    client_root: &Path,
    now: i64,
) -> Result<bool, StoreError> {
    let sync_path = client_root.join("sync_account.json");
    if !sync_path.exists() {
        return Ok(false);
    }
    let sync_raw = fs::read_to_string(&sync_path).map_err(StoreError::Io)?;
    let configured_user_id = tolerant_json_string(&sync_raw, "userId").unwrap_or_default();
    let configured_token = tolerant_json_string(&sync_raw, "token").unwrap_or_default();
    let user_by_id = if configured_user_id.trim().is_empty() {
        None
    } else {
        store.find_user_by_id(configured_user_id.trim())?
    };
    let user_by_token = if configured_token.trim().is_empty() {
        None
    } else {
        match store.authenticate_token(configured_token.trim(), now)? {
            TokenAuthentication::Active(token) => Some(token.user_id),
            _ => None,
        }
    };
    if let (Some(user), Some(token_user_id)) = (&user_by_id, &user_by_token) {
        if user.id != *token_user_id {
            return Err(StoreError::Integrity(
                "Windows sync account userId and token identify different users".to_string(),
            ));
        }
    }
    let user_id = user_by_id
        .map(|user| user.id)
        .or(user_by_token)
        .unwrap_or_default();
    if user_id.is_empty() {
        if configured_user_id.trim().is_empty() && configured_token.trim().is_empty() {
            return Ok(false);
        }
        return Err(StoreError::Integrity(
            "Windows sync account could not be matched to a migrated user".to_string(),
        ));
    }

    let scoped = windows_scoped_state_path(&client_root, &user_id);
    let mut changed = false;
    if scoped.exists() {
        let raw = fs::read_to_string(&scoped).map_err(StoreError::Io)?;
        ensure_migration_schema_supported(&raw, "Windows scoped timer state")?;
        let sanitized = sanitize_sync_app_data(&raw, now).ok_or_else(|| {
            StoreError::Integrity(format!("invalid Windows timer state: {}", scoped.display()))
        })?;
        let source_updated_at = fs::metadata(&scoped)
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .and_then(system_time_to_millis)
            .unwrap_or(now);
        let report = match store.import_account_snapshot_with_revision_merge(
            &user_id,
            &sanitized,
            source_updated_at,
            now,
            merge_migration_app_data_json,
        ) {
            Ok(report) => Some(report),
            Err(error) if deferred_legacy_media_intent(&error) => {
                eprintln!(
                    "deferred legacy scoped Windows timer state: ambiguous media removal remains in its original file"
                );
                None
            }
            Err(error) => return Err(error),
        };
        changed |= report.is_some_and(|report| report.changed);
    }

    let global = client_root.join("timer_state.json");
    if !global.exists() {
        return Ok(changed);
    }
    let raw_bytes = fs::read(&global).map_err(StoreError::Io)?;
    let content_sha256 = hex_bytes(&Sha256::digest(&raw_bytes));
    let raw = String::from_utf8(raw_bytes).map_err(|_| {
        StoreError::Integrity(format!(
            "invalid UTF-8 in Windows timer state: {}",
            global.display()
        ))
    })?;
    ensure_migration_schema_supported(&raw, "Windows global timer state")?;
    let sanitized = sanitize_sync_app_data(&raw, now).ok_or_else(|| {
        StoreError::Integrity(format!("invalid Windows timer state: {}", global.display()))
    })?;
    let Some(global_owner_id) = resolve_legacy_global_snapshot_owner(store, &sanitized)? else {
        eprintln!(
            "skipped the legacy global Windows timer state because no unique content owner could be proven"
        );
        return Ok(changed);
    };
    if global_owner_id != user_id {
        eprintln!(
            "legacy global Windows timer state content belongs to a different account than the active desktop login"
        );
    }
    let source_updated_at = fs::metadata(&global)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_to_millis)
        .unwrap_or(now);
    let global_changed = match store.import_bound_account_snapshot_with_revision_merge(
        "windows_global_timer_state_v1",
        &global_owner_id,
        &content_sha256,
        &global.to_string_lossy(),
        &sanitized,
        source_updated_at,
        now,
        merge_migration_app_data_json,
    ) {
        Ok(BoundSnapshotImportOutcome::Imported { .. }) => {
            eprintln!(
                "bound and imported the legacy global Windows timer state for its content-matched account"
            );
            true
        }
        Ok(BoundSnapshotImportOutcome::AlreadyBound(binding)) => {
            if binding.user_id == global_owner_id && binding.content_sha256 == content_sha256 {
                eprintln!(
                    "skipped the legacy global Windows timer state because it was already imported"
                );
            } else {
                eprintln!(
                    "skipped the legacy global Windows timer state because it is permanently bound to another account or content version"
                );
            }
            false
        }
        Err(error) if deferred_legacy_media_intent(&error) => {
            eprintln!(
                "deferred legacy global Windows timer state: ambiguous media removal remains in its original file"
            );
            false
        }
        Err(error) => return Err(error),
    };
    Ok(changed || global_changed)
}

#[cfg(not(target_os = "android"))]
fn resolve_legacy_global_snapshot_owner(
    store: &SqliteServerStore,
    global_app_data_json: &str,
) -> Result<Option<String>, StoreError> {
    let global_value = serde_json::from_str::<Value>(global_app_data_json)?;
    let global_ids = stable_history_entity_ids(&global_value);
    if global_ids.len() < 3 {
        return Ok(None);
    }
    let mut candidates = Vec::<(usize, String)>::new();
    for account in store.list_account_snapshots()? {
        let Ok(value) = serde_json::from_str::<Value>(&account.app_data_json) else {
            continue;
        };
        let account_ids = stable_history_entity_ids(&value);
        candidates.push((
            global_ids.intersection(&account_ids).count(),
            account.user_id,
        ));
    }
    candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let Some((best_overlap, best_user_id)) = candidates.first() else {
        return Ok(None);
    };
    let second_overlap = candidates.get(1).map(|value| value.0).unwrap_or(0);
    let has_high_coverage = *best_overlap >= 3
        && best_overlap.saturating_mul(100) >= global_ids.len().saturating_mul(60);
    let is_unique = second_overlap == 0
        || best_overlap.saturating_mul(100) >= second_overlap.saturating_mul(125);
    if has_high_coverage && is_unique {
        Ok(Some(best_user_id.clone()))
    } else {
        Ok(None)
    }
}

fn stable_history_entity_ids(value: &Value) -> HashSet<String> {
    let mut ids = HashSet::new();
    let Some(root) = value.as_object() else {
        return ids;
    };
    for field_name in ["sessions", "archivedTasks", "notes"] {
        let Some(values) = root.get(field_name).and_then(Value::as_array) else {
            continue;
        };
        for value in values {
            let Some(object) = value.as_object() else {
                continue;
            };
            let id = object.get("id").and_then(|value| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| value.as_i64().map(|value| value.to_string()))
            });
            if let Some(id) = id {
                ids.insert(format!("{field_name}\n{id}"));
            }
        }
    }
    ids
}

#[cfg(not(target_os = "android"))]
fn merge_migration_app_data_json(
    account: &str,
    account_updated_at: i64,
    incoming: &str,
    incoming_updated_at: i64,
    now: i64,
) -> Result<String, StoreError> {
    ensure_migration_schema_supported(account, "legacy account snapshot")?;
    ensure_migration_schema_supported(incoming, "legacy incoming snapshot")?;
    let account = prepare_legacy_scalar_revisions(account, account_updated_at)?;
    let incoming = prepare_legacy_scalar_revisions(incoming, incoming_updated_at)?;
    match (account.trim().is_empty(), incoming.trim().is_empty()) {
        (true, true) => Ok(String::new()),
        (true, false) => sanitize_sync_app_data(&incoming, now).ok_or_else(|| {
            StoreError::Integrity("legacy incoming snapshot is invalid".to_string())
        }),
        (false, true) => sanitize_sync_app_data(&account, now)
            .ok_or_else(|| StoreError::Integrity("legacy account snapshot is invalid".to_string())),
        (false, false) => merge_sync_app_data_json_with_media_intents(
            &account,
            &incoming,
            now,
            &crate::note_media_intent::NoteAttachmentDetachments::new(),
            &std::collections::BTreeMap::new(),
        )
        .map(|plan| plan.app_data_json)
        .map_err(|reason| {
            StoreError::Integrity(format!("legacy account snapshot merge failed: {reason}"))
        }),
    }
}

#[cfg(not(target_os = "android"))]
fn deferred_legacy_media_intent(error: &StoreError) -> bool {
    matches!(error, StoreError::Integrity(reason)
        if reason == "legacy account snapshot merge failed: attachment deletion intent is ambiguous; original references require recovery")
}

#[cfg(not(target_os = "android"))]
fn ensure_migration_schema_supported(raw: &str, label: &str) -> Result<(), StoreError> {
    match inspect_app_data_schema(raw) {
        AppDataSchemaStatus::Supported => Ok(()),
        AppDataSchemaStatus::Future(version) => Err(StoreError::Integrity(format!(
            "{label} uses app data schemaVersion {version}, newer than supported version {}; upgrade required",
            app_data::APP_DATA_SCHEMA_VERSION
        ))),
        AppDataSchemaStatus::Invalid => Err(StoreError::Integrity(format!(
            "{label} has an invalid app data schema envelope"
        ))),
    }
}

#[cfg(not(target_os = "android"))]
fn prepare_legacy_scalar_revisions(
    raw: &str,
    source_updated_at: i64,
) -> Result<String, StoreError> {
    if raw.trim().is_empty() {
        return Ok(String::new());
    }
    let mut value = serde_json::from_str::<Value>(raw).map_err(StoreError::Json)?;
    let root = value.as_object_mut().ok_or_else(|| {
        StoreError::Integrity("legacy app snapshot root is not an object".to_string())
    })?;
    let revision = source_updated_at.max(1);
    for (field_name, revision_name) in [
        ("slotOrder", "slotOrderUpdatedAtEpochMillis"),
        ("notePreferences", "notePreferencesUpdatedAtEpochMillis"),
        ("financeProfile", "financeProfileUpdatedAtEpochMillis"),
        ("themeMode", "themeModeUpdatedAtEpochMillis"),
    ] {
        if root.contains_key(field_name) && revision_field(root, revision_name) == 0 {
            root.insert(revision_name.to_string(), json!(revision));
        }
    }
    Ok(value.to_string())
}

#[cfg(not(target_os = "android"))]
fn windows_scoped_state_path(root: &Path, user_id: &str) -> PathBuf {
    let safe_prefix = user_id
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .take(32)
        .collect::<String>();
    let safe_prefix = if safe_prefix.is_empty() {
        "user"
    } else {
        &safe_prefix
    };
    root.join("accounts")
        .join(format!("{safe_prefix}-{}", token_identifier(user_id)))
        .join("timer_state.json")
}

fn tolerant_json_string(raw: &str, key: &str) -> Option<String> {
    if let Ok(value) = serde_json::from_str::<Value>(raw) {
        if let Some(value) = value.get(key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }
    let marker = format!("\"{key}\"");
    let start = raw.find(&marker)? + marker.len();
    let remainder = raw[start..].trim_start();
    let remainder = remainder.strip_prefix(':')?.trim_start();
    let bytes = remainder.as_bytes();
    if bytes.first().copied() != Some(b'"') {
        return None;
    }
    let mut escaped = false;
    for index in 1..bytes.len() {
        match (bytes[index], escaped) {
            (b'"', false) => {
                return serde_json::from_str::<String>(&remainder[..=index]).ok();
            }
            (b'\\', false) => escaped = true,
            (_, true) => escaped = false,
            _ => {}
        }
    }
    None
}

fn system_time_to_millis(value: SystemTime) -> Option<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
}

struct ActiveConnectionPermit(Arc<AtomicUsize>);

impl Drop for ActiveConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Default)]
struct LoginRateLimiter {
    attempts: HashMap<IpAddr, VecDeque<i64>>,
}

impl LoginRateLimiter {
    fn allow(&mut self, address: IpAddr, now: i64) -> bool {
        let cutoff = now.saturating_sub(LOGIN_RATE_WINDOW_MILLIS);
        self.attempts
            .retain(|_, attempts| attempts.back().is_some_and(|value| *value >= cutoff));
        let attempts = self.attempts.entry(address).or_default();
        while attempts.front().is_some_and(|value| *value < cutoff) {
            attempts.pop_front();
        }
        if attempts.len() >= LOGIN_RATE_MAX_ATTEMPTS {
            return false;
        }
        attempts.push_back(now);
        true
    }
}

pub fn default_server_store_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("GridTimerSync").join("server_store.json")
}

fn public_access_candidate_port(bind_addr: &str) -> Option<u16> {
    if let Ok(socket_addr) = bind_addr.parse::<SocketAddr>() {
        if socket_addr.ip().is_loopback() || socket_addr.port() == 0 {
            return None;
        }
        return Some(socket_addr.port());
    }
    bind_addr
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok())
        .filter(|port| *port != 0)
}

fn configured_public_runtime_info() -> ServerRuntimeInfo {
    let public_server_url = std::env::var("GRID_TIMER_PUBLIC_SERVER_URL")
        .ok()
        .and_then(|value| normalized_public_server_url(&value))
        .or_else(|| {
            configured_public_server_url_file().and_then(|path| {
                fs::read_to_string(path)
                    .ok()
                    .and_then(|value| normalized_public_server_url(&value))
            })
        })
        .unwrap_or_default();
    if public_server_url.is_empty() {
        ServerRuntimeInfo::default()
    } else {
        ServerRuntimeInfo {
            public_server_url: public_server_url.clone(),
            public_access_message: format!("Public tunnel URL: {public_server_url}"),
            ..ServerRuntimeInfo::default()
        }
    }
}

fn refresh_configured_public_runtime_info(runtime_info: &Arc<Mutex<ServerRuntimeInfo>>) {
    let refreshed = configured_public_runtime_info();
    if let Ok(mut locked) = runtime_info.lock() {
        if locked.public_server_url != refreshed.public_server_url {
            if refreshed.public_server_url.is_empty() {
                println!("sync public access: HTTPS tunnel is unavailable");
            } else {
                println!("sync public access: {}", refreshed.public_access_message);
            }
            locked.public_server_url = refreshed.public_server_url;
            locked.public_access_message = refreshed.public_access_message;
        }
    }
}

fn start_sync_beacon(port: u16, runtime_info: Arc<Mutex<ServerRuntimeInfo>>) {
    thread::spawn(move || {
        let Ok(socket) = UdpSocket::bind(("0.0.0.0", SYNC_BEACON_PORT)) else {
            return;
        };
        let _ = socket.set_broadcast(true);
        let _ = socket.set_read_timeout(Some(Duration::from_millis(160)));
        loop {
            refresh_configured_public_runtime_info(&runtime_info);
            let info = runtime_info
                .lock()
                .map(|locked| locked.clone())
                .unwrap_or_default();
            if let Some(payload) = sync_beacon_payload(port, &info, None) {
                let bytes = payload.as_bytes();
                let _ = socket.send_to(bytes, ("255.255.255.255", SYNC_BEACON_PORT));
                let _ = socket.send_to(bytes, ("239.255.89.17", SYNC_BEACON_PORT));
            }
            let until = Instant::now() + Duration::from_secs(1);
            let mut buffer = [0u8; 2048];
            while Instant::now() < until {
                match socket.recv_from(&mut buffer) {
                    Ok((length, peer)) => {
                        let text = String::from_utf8_lossy(&buffer[..length]);
                        if text.contains("grid_timer_sync_discovery") {
                            if let Some(payload) = sync_beacon_payload(port, &info, Some(peer)) {
                                let _ = socket.send_to(payload.as_bytes(), peer);
                            }
                        }
                    }
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            || error.kind() == io::ErrorKind::TimedOut => {}
                    Err(_) => break,
                }
            }
        }
    });
}

fn sync_beacon_payload(
    port: u16,
    info: &ServerRuntimeInfo,
    peer: Option<SocketAddr>,
) -> Option<String> {
    let public_server_url = info.public_server_url.trim();
    let local_server_url = peer
        .and_then(|peer| local_ipv4_for_peer(peer).ok())
        .map(|ip| format!("http://{ip}:{port}"))
        .unwrap_or_default();
    let server_url = if !public_server_url.is_empty() {
        public_server_url.to_string()
    } else {
        local_server_url.clone()
    };
    if server_url.is_empty() {
        return None;
    }
    Some(
        json!({
            "type": "grid_timer_sync_beacon",
            "version": 1,
            "serverUrl": server_url,
            "publicServerUrl": public_server_url,
            "localServerUrl": local_server_url,
            "port": port
        })
        .to_string(),
    )
}

fn local_ipv4_for_peer(peer: SocketAddr) -> io::Result<Ipv4Addr> {
    match peer {
        SocketAddr::V4(peer_v4) => local_ipv4_for_remote(&peer_v4.ip().to_string(), peer_v4.port()),
        SocketAddr::V6(_) => Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            "peer did not use IPv4",
        )),
    }
}

fn configured_public_server_url_file() -> Option<PathBuf> {
    std::env::var_os("GRID_TIMER_PUBLIC_SERVER_URL_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|dir| dir.join("tmp").join("sync_public_server_url.txt"))
        })
}

fn normalized_public_server_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    let parsed = parse_base_url(&trimmed).ok()?;
    if parsed.scheme != SyncUrlScheme::Https || !parsed.base_path.is_empty() {
        return None;
    }
    let host = parsed.host.to_ascii_lowercase();
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.parse::<IpAddr>().map_or_else(
            |_| !is_public_dns_name(&host),
            |address| !is_public_sync_ip(address),
        )
    {
        return None;
    }
    Some(trimmed)
}

fn is_public_dns_name(host: &str) -> bool {
    if host.len() > 253 || !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn is_public_sync_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_sync_ipv4(address),
        IpAddr::V6(address) => is_public_sync_ipv6(address),
    }
}

fn is_public_sync_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_sync_ipv4(mapped);
    }
    let octets = address.octets();
    !address.is_unspecified()
        && !address.is_loopback()
        && !address.is_multicast()
        && octets[0] & 0xfe != 0xfc
        && !(octets[0] == 0xfe && octets[1] & 0xc0 == 0x80)
        && !(octets[0] == 0xfe && octets[1] & 0xc0 == 0xc0)
        && octets[..4] != [0x20, 0x01, 0x0d, 0xb8]
}

#[cfg(test)]
fn configure_public_sync_access(port: u16) -> ServerRuntimeInfo {
    let gateway = match discover_upnp_gateway() {
        Ok(value) => value,
        Err(error) => {
            return ServerRuntimeInfo {
                public_access_message: format!(
                    "Router did not provide automatic public access: {error}"
                ),
                ..ServerRuntimeInfo::default()
            };
        }
    };
    let local_ip = match local_ipv4_for_remote(&gateway.control_host, gateway.control_port) {
        Ok(value) => value,
        Err(error) => {
            return ServerRuntimeInfo {
                public_access_message: format!(
                    "Could not find the computer LAN address for router mapping: {error}"
                ),
                ..ServerRuntimeInfo::default()
            };
        }
    };
    let external_ip = match upnp_external_ipv4(&gateway) {
        Ok(value) => value,
        Err(error) => {
            return ServerRuntimeInfo {
                public_access_message: format!("Could not read router public address: {error}"),
                ..ServerRuntimeInfo::default()
            };
        }
    };
    if !is_public_sync_ipv4(external_ip) {
        return ServerRuntimeInfo {
            public_access_message: format!(
                "Router external address {external_ip} is not a public IPv4 address."
            ),
            ..ServerRuntimeInfo::default()
        };
    }

    let mapping_ready = match upnp_add_port_mapping(&gateway, port, local_ip) {
        Ok(()) => Ok("Router port mapping is active.".to_string()),
        Err(add_error) => {
            if upnp_existing_mapping_matches(&gateway, port, local_ip).unwrap_or(false) {
                Ok("Existing router port mapping is active.".to_string())
            } else {
                Err(add_error)
            }
        }
    };
    let mapping_message = match mapping_ready {
        Ok(value) => value,
        Err(error) => {
            return ServerRuntimeInfo {
                public_access_message: format!("Could not create router port mapping: {error}"),
                ..ServerRuntimeInfo::default()
            };
        }
    };

    let public_server_url = format!("http://{external_ip}:{port}");
    ServerRuntimeInfo {
        public_server_url: public_server_url.clone(),
        public_access_message: format!("{mapping_message} Public sync URL: {public_server_url}"),
        ..ServerRuntimeInfo::default()
    }
}

#[derive(Clone, Debug)]
#[cfg(test)]
struct UpnpGateway {
    control_url: String,
    service_type: String,
    control_host: String,
    control_port: u16,
}

#[cfg(test)]
fn discover_upnp_gateway() -> Result<UpnpGateway, String> {
    let locations = discover_upnp_locations()?;
    for location in locations {
        let Ok(description) = upnp_http_get(&location) else {
            continue;
        };
        let Some((service_type, control_url)) =
            find_wan_connection_service(&description, &location)
        else {
            continue;
        };
        let parsed = parse_base_url(&control_url)?;
        return Ok(UpnpGateway {
            control_url,
            service_type,
            control_host: parsed.host,
            control_port: parsed.port,
        });
    }
    Err("no WAN connection service responded on the local router".to_string())
}

#[cfg(test)]
fn discover_upnp_locations() -> Result<Vec<String>, String> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|error| format!("could not open local discovery socket: {error}"))?;
    let _ = socket.set_multicast_ttl_v4(2);
    socket
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|error| format!("could not configure discovery timeout: {error}"))?;
    let search_targets = [
        "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
        "urn:schemas-upnp-org:service:WANIPConnection:1",
        "urn:schemas-upnp-org:service:WANPPPConnection:1",
        "ssdp:all",
    ];
    for target in search_targets {
        let request = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {target}\r\n\r\n"
        );
        let _ = socket.send_to(request.as_bytes(), "239.255.255.250:1900");
    }

    let deadline = Instant::now() + UPNP_DISCOVERY_TIMEOUT;
    let mut locations = Vec::<String>::new();
    let mut buffer = [0u8; 4096];
    while Instant::now() < deadline {
        match socket.recv_from(&mut buffer) {
            Ok((length, _)) => {
                let response = String::from_utf8_lossy(&buffer[..length]);
                if let Some(location) = ssdp_header_value(&response, "location") {
                    if !locations
                        .iter()
                        .any(|known| known.eq_ignore_ascii_case(&location))
                    {
                        locations.push(location);
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    if locations.is_empty() {
        Err("no UPnP gateway answered local discovery".to_string())
    } else {
        Ok(locations)
    }
}

#[cfg(test)]
fn ssdp_header_value(response: &str, header_name: &str) -> Option<String> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case(header_name) {
            Some(value.trim().to_string()).filter(|value| !value.is_empty())
        } else {
            None
        }
    })
}

#[cfg(test)]
fn upnp_http_get(url: &str) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(UPNP_HTTP_TIMEOUT)
        .timeout_read(UPNP_HTTP_TIMEOUT)
        .build();
    match agent
        .get(url)
        .set("Accept", "text/xml, application/xml, */*")
        .call()
    {
        Ok(response) => response
            .into_string()
            .map_err(|error| format!("could not read router description: {error}")),
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(format!("router description returned HTTP {status}: {body}"))
        }
        Err(error) => Err(format!("could not fetch router description: {error}")),
    }
}

#[cfg(test)]
fn find_wan_connection_service(description: &str, location: &str) -> Option<(String, String)> {
    for block in xml_blocks(description, "service") {
        let service_type = xml_tag_text(block, "serviceType")?;
        if !service_type.contains("WANIPConnection") && !service_type.contains("WANPPPConnection") {
            continue;
        }
        let control_path = xml_tag_text(block, "controlURL")?;
        let control_url = absolute_control_url(location, &control_path)?;
        return Some((service_type, control_url));
    }
    None
}

#[cfg(test)]
fn xml_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let lower = xml.to_ascii_lowercase();
    let open = format!("<{}", tag.to_ascii_lowercase());
    let close = format!("</{}>", tag.to_ascii_lowercase());
    let mut output = Vec::new();
    let mut search_start = 0usize;
    while let Some(open_start) = lower[search_start..].find(&open) {
        let open_start = search_start + open_start;
        let Some(open_end) = lower[open_start..].find('>') else {
            break;
        };
        let content_start = open_start + open_end + 1;
        let Some(close_start) = lower[content_start..].find(&close) else {
            break;
        };
        let close_start = content_start + close_start;
        output.push(&xml[content_start..close_start]);
        search_start = close_start + close.len();
    }
    output
}

#[cfg(test)]
fn xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let tag_lower = tag.to_ascii_lowercase();
    let open = format!("<{tag_lower}");
    let close = format!("</{tag_lower}>");
    let open_start = lower.find(&open)?;
    let open_end = lower[open_start..].find('>')? + open_start;
    let content_start = open_end + 1;
    let close_start = lower[content_start..].find(&close)? + content_start;
    Some(xml_unescape(xml[content_start..close_start].trim())).filter(|value| !value.is_empty())
}

#[cfg(test)]
fn xml_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

#[cfg(test)]
fn absolute_control_url(location: &str, control_path: &str) -> Option<String> {
    let control_path = control_path.trim();
    if control_path.starts_with("http://") || control_path.starts_with("https://") {
        return Some(control_path.to_string());
    }
    let parsed = parse_base_url(location).ok()?;
    let root = format!("{}://{}", parsed.scheme.as_str(), host_header(&parsed));
    if control_path.starts_with('/') {
        return Some(format!("{root}{control_path}"));
    }
    let base_dir = parsed
        .base_path
        .rsplit_once('/')
        .map(|(dir, _)| dir)
        .unwrap_or("");
    if base_dir.is_empty() {
        Some(format!("{root}/{control_path}"))
    } else {
        Some(format!("{root}{base_dir}/{control_path}"))
    }
}

fn local_ipv4_for_remote(host: &str, port: u16) -> io::Result<Ipv4Addr> {
    let address = format!("{host}:{port}");
    let mut last_error = None;
    for remote in address.to_socket_addrs()? {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        if let Err(error) = socket.connect(remote) {
            last_error = Some(error);
            continue;
        }
        if let SocketAddr::V4(local_addr) = socket.local_addr()? {
            return Ok(*local_addr.ip());
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no IPv4 route to {address}"),
        )
    }))
}

#[cfg(test)]
fn upnp_external_ipv4(gateway: &UpnpGateway) -> Result<Ipv4Addr, String> {
    let response = upnp_soap_call(gateway, "GetExternalIPAddress", "")?;
    let raw_ip = xml_tag_text(&response, "NewExternalIPAddress")
        .ok_or_else(|| "router did not return NewExternalIPAddress".to_string())?;
    raw_ip
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("router returned invalid external address {raw_ip}"))
}

#[cfg(test)]
fn upnp_add_port_mapping(
    gateway: &UpnpGateway,
    external_port: u16,
    local_ip: Ipv4Addr,
) -> Result<(), String> {
    let body = format!(
        concat!(
            "<NewRemoteHost></NewRemoteHost>",
            "<NewExternalPort>{external_port}</NewExternalPort>",
            "<NewProtocol>TCP</NewProtocol>",
            "<NewInternalPort>{external_port}</NewInternalPort>",
            "<NewInternalClient>{local_ip}</NewInternalClient>",
            "<NewEnabled>1</NewEnabled>",
            "<NewPortMappingDescription>Grid Timer Sync</NewPortMappingDescription>",
            "<NewLeaseDuration>0</NewLeaseDuration>"
        ),
        external_port = external_port,
        local_ip = local_ip
    );
    upnp_soap_call(gateway, "AddPortMapping", &body).map(|_| ())
}

#[cfg(test)]
fn upnp_existing_mapping_matches(
    gateway: &UpnpGateway,
    external_port: u16,
    local_ip: Ipv4Addr,
) -> Result<bool, String> {
    let body = format!(
        concat!(
            "<NewRemoteHost></NewRemoteHost>",
            "<NewExternalPort>{external_port}</NewExternalPort>",
            "<NewProtocol>TCP</NewProtocol>"
        ),
        external_port = external_port
    );
    let response = upnp_soap_call(gateway, "GetSpecificPortMappingEntry", &body)?;
    let mapped_ip = xml_tag_text(&response, "NewInternalClient")
        .and_then(|value| value.parse::<Ipv4Addr>().ok());
    let mapped_port =
        xml_tag_text(&response, "NewInternalPort").and_then(|value| value.parse::<u16>().ok());
    Ok(mapped_ip == Some(local_ip) && mapped_port == Some(external_port))
}

#[cfg(test)]
fn upnp_soap_call(gateway: &UpnpGateway, action: &str, body: &str) -> Result<String, String> {
    let envelope = format!(
        concat!(
            r#"<?xml version="1.0"?>"#,
            r#"<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" "#,
            r#"s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">"#,
            r#"<s:Body><u:{action} xmlns:u="{service_type}">"#,
            "{body}",
            r#"</u:{action}></s:Body></s:Envelope>"#
        ),
        action = action,
        service_type = gateway.service_type,
        body = body
    );
    let soap_action = format!("\"{}#{}\"", gateway.service_type, action);
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(UPNP_HTTP_TIMEOUT)
        .timeout_read(UPNP_HTTP_TIMEOUT)
        .build();
    match agent
        .post(&gateway.control_url)
        .set("Content-Type", "text/xml; charset=\"utf-8\"")
        .set("SOAPAction", &soap_action)
        .send_string(&envelope)
    {
        Ok(response) => response
            .into_string()
            .map_err(|error| format!("could not read router response: {error}")),
        Err(ureq::Error::Status(status, response)) => {
            let body = response.into_string().unwrap_or_default();
            Err(format!("router returned HTTP {status}: {body}"))
        }
        Err(error) => Err(format!("router request failed: {error}")),
    }
}

fn is_public_sync_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, d] = ip.octets();
    if a == 0 || a == 10 || a == 127 || a >= 224 || ip.is_private() || ip.is_link_local() {
        return false;
    }
    if a == 100 && (64..=127).contains(&b) {
        return false;
    }
    if a == 192 && b == 0 {
        return false;
    }
    if a == 198 && (b == 18 || b == 19) {
        return false;
    }
    if (a == 192 && b == 0 && c == 2)
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
    {
        return false;
    }
    !(a == 255 && b == 255 && c == 255 && d == 255)
}

fn register_account_result(
    server_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> SyncClientResult {
    let request = RegisterRequest {
        request_id: random_token(18),
        email: normalized_email(email),
        password: password.to_string(),
        device_name: normalized_device_name(device_name),
    };
    post_json(
        server_url,
        "/v1/register",
        None,
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    )
}

fn login_account_result(
    server_url: &str,
    email: &str,
    password: &str,
    device_name: &str,
) -> SyncClientResult {
    let request = LoginRequest {
        request_id: random_token(18),
        email: normalized_email(email),
        password: password.to_string(),
        device_name: normalized_device_name(device_name),
    };
    post_json(
        server_url,
        "/v1/login",
        None,
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    )
}

fn sync_app_data_result(
    server_url: &str,
    token: &str,
    server_instance_id: &str,
    account_namespace: &str,
    workspace_id: &str,
    workspace_proof: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    device_name: &str,
    acknowledged_generation: i64,
    restore_receipt: &str,
    force_upload: bool,
    force_download: bool,
) -> SyncClientResult {
    if token.trim().is_empty() {
        return error_result("Not logged in.");
    }
    let now = now_millis();
    let sanitized = sanitize_sync_app_data(app_data_json, now)
        .unwrap_or_else(|| app_data_json.trim().to_string());
    let request = SyncSnapshotRequest {
        request_id: random_token(18),
        app_data_json: sanitized.clone(),
        client_updated_at_epoch_millis,
        acknowledged_generation: acknowledged_generation.max(0),
        restore_receipt: restore_receipt.trim().to_string(),
        server_instance_id: server_instance_id.to_string(),
        account_namespace: account_namespace.to_string(),
        workspace_id: workspace_id.to_string(),
        workspace_proof: workspace_proof.to_string(),
        allow_workspace_identity_rebind: false,
        previous_server_instance_id: String::new(),
        previous_account_namespace: String::new(),
        device_name: normalized_device_name(device_name),
        force_upload,
        force_download,
    };
    let mut result = post_json(
        server_url,
        if force_upload {
            "/v1/upload-local"
        } else {
            "/v1/sync"
        },
        Some(token),
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    );
    if !force_upload && !force_download {
        if let Some(rebound) = request_workspace_identity_rebind_baseline(
            server_url,
            token,
            server_instance_id,
            account_namespace,
            workspace_id,
            workspace_proof,
            &sanitized,
            client_updated_at_epoch_millis,
            acknowledged_generation,
            restore_receipt,
            device_name,
            &result,
        ) {
            result = rebound;
        }
    }
    if result.ok && !result.restore_required && !force_upload && !force_download {
        if let Some(received_json) = result.app_data_json.clone() {
            if let Some(merged_json) = merge_sync_app_data_json(&received_json, &sanitized, now) {
                if merged_json != received_json {
                    result.app_data_json = Some(merged_json);
                    result.server_updated_at_epoch_millis = app_data_revision_millis(
                        result.app_data_json.as_deref().unwrap_or_default(),
                        result
                            .server_updated_at_epoch_millis
                            .max(client_updated_at_epoch_millis),
                    );
                    result.mode = "merged".to_string();
                    result.message = "本机和账号数据已合并。".to_string();
                }
            }
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn request_workspace_identity_rebind_baseline(
    server_url: &str,
    token: &str,
    previous_server_instance_id: &str,
    previous_account_namespace: &str,
    workspace_id: &str,
    previous_workspace_proof: &str,
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    acknowledged_generation: i64,
    restore_receipt: &str,
    device_name: &str,
    mismatch: &SyncClientResult,
) -> Option<SyncClientResult> {
    let user_id = mismatch.user_id.trim();
    let current_server_instance_id = mismatch.server_instance_id.trim();
    let current_account_namespace = mismatch.account_namespace.trim();
    let previous_server_instance_id = previous_server_instance_id.trim();
    let previous_account_namespace = previous_account_namespace.trim();
    let workspace_id = workspace_id.trim();
    if mismatch.ok
        || mismatch.mode != "workspace_binding_mismatch"
        || mismatch.retryable
        || mismatch.current_committed
        || mismatch.restore_required
        || mismatch.baseline_merge_required
        || mismatch.workspace_identity_rebound
        || mismatch.app_data_json.is_some()
        || !mismatch.restore_receipt.is_empty()
        || !mismatch.workspace_id.is_empty()
        || !mismatch.workspace_proof.is_empty()
        || mismatch.current_generation != 0
        || acknowledged_generation != 0
        || !restore_receipt.trim().is_empty()
        || user_id.is_empty()
        || mismatch.token_id != token_identifier(token)
        || !valid_workspace_capability_component(current_server_instance_id)
        || !valid_workspace_capability_component(current_account_namespace)
        || current_account_namespace
            != account_namespace_identifier(current_server_instance_id, user_id)
        || !valid_workspace_capability_component(previous_server_instance_id)
        || !valid_workspace_capability_component(previous_account_namespace)
        || previous_account_namespace
            != account_namespace_identifier(previous_server_instance_id, user_id)
        || previous_server_instance_id == current_server_instance_id
        || previous_account_namespace == current_account_namespace
        || !valid_workspace_capability_component(workspace_id)
        || !valid_workspace_capability_component(previous_workspace_proof.trim())
    {
        return None;
    }

    let request = SyncSnapshotRequest {
        request_id: random_token(18),
        app_data_json: app_data_json.to_string(),
        client_updated_at_epoch_millis,
        acknowledged_generation: 0,
        restore_receipt: String::new(),
        server_instance_id: current_server_instance_id.to_string(),
        account_namespace: current_account_namespace.to_string(),
        workspace_id: workspace_id.to_string(),
        workspace_proof: String::new(),
        allow_workspace_identity_rebind: true,
        previous_server_instance_id: previous_server_instance_id.to_string(),
        previous_account_namespace: previous_account_namespace.to_string(),
        device_name: normalized_device_name(device_name),
        force_upload: false,
        force_download: false,
    };
    let rebound = post_json(
        server_url,
        "/v1/sync",
        Some(token),
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    );
    let valid_rebound = rebound.ok
        && rebound.mode == "baseline_required"
        && !rebound.retryable
        && rebound.current_committed
        && rebound.restore_required
        && rebound.baseline_merge_required
        && rebound.workspace_identity_rebound
        && rebound.app_data_json.is_some()
        && rebound.current_generation == 0
        && !rebound.restore_receipt.trim().is_empty()
        && rebound.workspace_id == workspace_id
        && rebound.workspace_proof.is_empty()
        && rebound.user_id == user_id
        && rebound.token_id == token_identifier(token)
        && rebound.server_instance_id == current_server_instance_id
        && rebound.account_namespace == current_account_namespace
        && (rebound.token.is_empty() || token_identifier(&rebound.token) == rebound.token_id);
    valid_rebound.then_some(rebound)
}

#[cfg(not(target_os = "android"))]
fn validate_desktop_media_auth(
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
) -> Option<SyncClientResult> {
    if expected_user_id.trim().is_empty() {
        return Some(desktop_media_client_error("Media userId is required."));
    }
    if token.trim().is_empty() {
        return Some(desktop_media_client_error("Not logged in."));
    }
    if !valid_workspace_capability_component(expected_server_instance_id.trim()) {
        return Some(desktop_media_client_error(
            "Media serverInstanceId is invalid.",
        ));
    }
    if !valid_workspace_capability_component(expected_account_namespace.trim()) {
        return Some(desktop_media_client_error(
            "Media accountNamespace is invalid.",
        ));
    }
    None
}

#[cfg(not(target_os = "android"))]
fn desktop_media_client_error(message: &str) -> SyncClientResult {
    let mut result = error_result(message);
    result.mode = "media_client_validation".to_string();
    result
}

#[cfg(not(target_os = "android"))]
fn validate_desktop_media_response_identity(
    result: SyncClientResult,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
) -> Result<SyncClientResult, SyncClientResult> {
    // Preserve transport, authentication, quota, and restore-barrier errors
    // verbatim when the server did not claim an authenticated identity.
    if !result.ok
        && result.user_id.trim().is_empty()
        && result.token_id.trim().is_empty()
        && result.server_instance_id.trim().is_empty()
        && result.account_namespace.trim().is_empty()
    {
        return Ok(result);
    }
    if result.user_id.trim() != expected_user_id.trim() {
        return Err(desktop_media_client_error(
            "Media response user does not match the authenticated account.",
        ));
    }
    if result.token_id.trim() != token_identifier(token) {
        return Err(desktop_media_client_error(
            "Media response token does not match the authenticated session.",
        ));
    }
    if result.server_instance_id.trim() != expected_server_instance_id.trim() {
        return Err(desktop_media_client_error(
            "Media response server instance does not match the authenticated server.",
        ));
    }
    if result.account_namespace.trim() != expected_account_namespace.trim() {
        return Err(desktop_media_client_error(
            "Media response account namespace does not match the authenticated account.",
        ));
    }
    Ok(result)
}

#[cfg(not(target_os = "android"))]
fn valid_media_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(not(target_os = "android"))]
fn validate_desktop_media_item(item: &MediaManifestItem) -> Result<(), &'static str> {
    if item.attachment_id.trim().is_empty() {
        return Err("Media response attachmentId is missing.");
    }
    if item.updated_at_epoch_millis < 0 || item.deleted_at_epoch_millis < 0 {
        return Err("Media response contains an invalid revision.");
    }
    if item.size_bytes < 0 || item.size_bytes as u128 > MAX_MEDIA_BYTES as u128 {
        return Err("Media response contains an invalid size.");
    }
    if item.deleted_at_epoch_millis > 0 {
        if !item.sha256.trim().is_empty() && !valid_media_sha256(item.sha256.trim()) {
            return Err("Media tombstone contains an invalid SHA-256.");
        }
        return Ok(());
    }
    if item.updated_at_epoch_millis <= 0 {
        return Err("Media response contains an invalid update revision.");
    }
    if item.size_bytes < 1 {
        return Err("Media response contains an invalid size.");
    }
    if !valid_media_sha256(item.sha256.trim()) {
        return Err("Media response contains an invalid SHA-256.");
    }
    if item.mime_type.trim().is_empty() {
        return Err("Media response MIME type is missing.");
    }
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn validate_desktop_media_manifest_response(
    result: SyncClientResult,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
) -> SyncClientResult {
    let result = match validate_desktop_media_response_identity(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        Ok(result) => result,
        Err(error) => return error,
    };
    let mut attachment_ids = HashSet::with_capacity(result.media_items.len());
    for item in &result.media_items {
        if let Err(message) = validate_desktop_media_item(item) {
            return desktop_media_client_error(message);
        }
        if !attachment_ids.insert(item.attachment_id.trim()) {
            return desktop_media_client_error("Media manifest contains a duplicate attachmentId.");
        }
    }
    let mut legacy_ids = HashSet::new();
    for item in &result.legacy_media_references {
        if !crate::server_store::valid_media_attachment_id(&item.attachment_id)
            || !crate::server_store::valid_media_mime_type(&item.mime_type)
            || !(1..=MAX_MEDIA_BYTES as i64).contains(&item.size_bytes)
            || !legacy_ids.insert(item.attachment_id.as_str())
        {
            return desktop_media_client_error("Legacy attachment manifest metadata is invalid.");
        }
    }
    result
}

#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
fn validate_desktop_media_upload_response(
    result: SyncClientResult,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
    attachment_id: &str,
    sha256: &str,
    mime_type: &str,
    size_bytes: i64,
) -> SyncClientResult {
    let result = match validate_desktop_media_response_identity(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        Ok(result) => result,
        Err(error) => return error,
    };
    if !result.ok {
        if let Some(item) = result.media_item.as_ref() {
            if let Err(message) = validate_desktop_media_item(item) {
                return desktop_media_client_error(message);
            }
            if item.attachment_id.trim() != attachment_id {
                return desktop_media_client_error("Media upload response attachmentId mismatch.");
            }
        }
        return result;
    }
    let Some(item) = result.media_item.as_ref() else {
        return desktop_media_client_error("Media upload response metadata is missing.");
    };
    if let Err(message) = validate_desktop_media_item(item) {
        return desktop_media_client_error(message);
    }
    if item.attachment_id.trim() != attachment_id {
        return desktop_media_client_error("Media upload response attachmentId mismatch.");
    }
    if !item.sha256.trim().eq_ignore_ascii_case(sha256) {
        return desktop_media_client_error("Media upload response SHA-256 mismatch.");
    }
    if item.mime_type.trim() != mime_type {
        return desktop_media_client_error("Media upload response MIME type mismatch.");
    }
    if item.size_bytes != size_bytes {
        return desktop_media_client_error("Media upload response size mismatch.");
    }
    result
}

#[cfg(not(target_os = "android"))]
fn validate_desktop_media_download_response(
    result: SyncClientResult,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
    attachment_id: &str,
) -> SyncClientResult {
    let result = match validate_desktop_media_response_identity(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        Ok(result) => result,
        Err(error) => return error,
    };
    if !result.ok {
        if let Some(item) = result.media_item.as_ref() {
            if let Err(message) = validate_desktop_media_item(item) {
                return desktop_media_client_error(message);
            }
            if item.attachment_id.trim() != attachment_id {
                return desktop_media_client_error(
                    "Media download response attachmentId mismatch.",
                );
            }
        }
        return result;
    }
    let Some(item) = result.media_item.as_ref() else {
        return desktop_media_client_error("Media download response metadata is missing.");
    };
    if let Err(message) = validate_desktop_media_item(item) {
        return desktop_media_client_error(message);
    }
    if item.deleted_at_epoch_millis > 0 {
        return desktop_media_client_error("Media download returned a tombstone.");
    }
    if item.attachment_id.trim() != attachment_id {
        return desktop_media_client_error("Media download response attachmentId mismatch.");
    }
    let content = match BASE64_STANDARD.decode(result.media_content_base64.trim()) {
        Ok(content) => content,
        Err(_) => {
            return desktop_media_client_error("Media download contentBase64 is invalid.");
        }
    };
    if content.is_empty()
        || content.len() > MAX_MEDIA_BYTES
        || content.len() as i64 != item.size_bytes
    {
        return desktop_media_client_error("Media download size mismatch.");
    }
    let computed_sha256 = hex_bytes(&Sha256::digest(&content));
    if !computed_sha256.eq_ignore_ascii_case(item.sha256.trim()) {
        return desktop_media_client_error("Media download SHA-256 mismatch.");
    }
    result
}

#[cfg(not(target_os = "android"))]
fn validate_desktop_media_delete_response(
    result: SyncClientResult,
    expected_user_id: &str,
    token: &str,
    expected_server_instance_id: &str,
    expected_account_namespace: &str,
    attachment_id: &str,
    deleted_at_epoch_millis: i64,
) -> SyncClientResult {
    let result = match validate_desktop_media_response_identity(
        result,
        expected_user_id,
        token,
        expected_server_instance_id,
        expected_account_namespace,
    ) {
        Ok(result) => result,
        Err(error) => return error,
    };
    if !result.ok {
        if let Some(item) = result.media_item.as_ref() {
            if let Err(message) = validate_desktop_media_item(item) {
                return desktop_media_client_error(message);
            }
            if item.attachment_id.trim() != attachment_id {
                return desktop_media_client_error("Media delete response attachmentId mismatch.");
            }
        }
        return result;
    }
    let Some(item) = result.media_item.as_ref() else {
        return desktop_media_client_error("Media delete response tombstone is missing.");
    };
    if let Err(message) = validate_desktop_media_item(item) {
        return desktop_media_client_error(message);
    }
    if item.attachment_id.trim() != attachment_id {
        return desktop_media_client_error("Media delete response attachmentId mismatch.");
    }
    if item.deleted_at_epoch_millis < deleted_at_epoch_millis {
        return desktop_media_client_error("Media delete response tombstone is stale.");
    }
    result
}

fn post_json(
    server_url: &str,
    endpoint: &str,
    bearer_token: Option<&str>,
    body: Value,
) -> SyncClientResult {
    #[cfg(not(target_os = "android"))]
    if let Err(error) = crate::runtime::cancellation::check_current() {
        return error_result(&error.to_string());
    }
    let base = match parse_base_url(server_url) {
        Ok(value) => value,
        Err(error) => return error_result(&error),
    };
    let request_body = match serde_json::to_string(&body) {
        Ok(value) => value,
        Err(_) => return error_result("Could not encode sync request."),
    };

    if base.scheme == SyncUrlScheme::Http && !is_loopback_sync_host(&base.host) {
        return error_result(
            "For data safety, account requests require HTTPS. HTTP is allowed only on this device.",
        );
    }

    match send_json_to_base(&base, endpoint, bearer_token, &request_body) {
        Ok(result) => result,
        Err(SyncClientIoError::Connect(error)) => {
            error_result(&format!("Could not connect to sync server: {error}"))
        }
        Err(SyncClientIoError::Send(error)) => {
            error_result(&format!("Could not send sync request: {error}"))
        }
        Err(SyncClientIoError::Read(error)) => {
            error_result(&format!("Could not read sync response: {error}"))
        }
    }
}

fn send_json_to_base(
    base: &ParsedBaseUrl,
    endpoint: &str,
    bearer_token: Option<&str>,
    request_body: &str,
) -> Result<SyncClientResult, SyncClientIoError> {
    if base.scheme == SyncUrlScheme::Https {
        #[cfg(target_os = "windows")]
        {
            let (status, body) = windows_http::request(
                "POST",
                &format_request_url(base, endpoint),
                bearer_token,
                request_body,
                SYNC_REQUEST_TIMEOUT,
                MAX_RESPONSE_BODY_BYTES,
            )
            .map_err(SyncClientIoError::Read)?;
            return Ok(parse_sync_body_response(status, &body));
        }
        #[cfg(not(target_os = "windows"))]
        return send_json_to_ureq_base(base, endpoint, bearer_token, request_body);
    }

    let response_bytes = send_http_json_to_base(base, endpoint, bearer_token, request_body)?;
    Ok(parse_client_response(&response_bytes))
}

fn send_http_json_to_base(
    base: &ParsedBaseUrl,
    endpoint: &str,
    bearer_token: Option<&str>,
    request_body: &str,
) -> Result<Vec<u8>, SyncClientIoError> {
    send_http_json_with_timeout(
        base,
        endpoint,
        bearer_token,
        request_body,
        SYNC_REQUEST_TIMEOUT,
    )
}

fn send_http_json_with_timeout(
    base: &ParsedBaseUrl,
    endpoint: &str,
    bearer_token: Option<&str>,
    request_body: &str,
    total_timeout: Duration,
) -> Result<Vec<u8>, SyncClientIoError> {
    let deadline = Instant::now() + total_timeout;
    if base.scheme != SyncUrlScheme::Http || !is_loopback_sync_host(&base.host) {
        return Err(SyncClientIoError::Connect(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "plain HTTP account requests must stay on this device",
        )));
    }
    let path = join_paths(&base.base_path, endpoint);
    let host_header = host_header(base);
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host_header}\r\nContent-Type: application/json\r\nAccept: application/json\r\nConnection: close\r\nContent-Length: {}\r\n",
        request_body.as_bytes().len()
    );
    if let Some(token) = bearer_token {
        request.push_str("Authorization: Bearer ");
        request.push_str(token.trim());
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    request.push_str(&request_body);

    let stream = http_deadline::connect_loopback(&base.host, base.port, deadline)
        .map_err(SyncClientIoError::Connect)?;
    #[cfg(not(target_os = "android"))]
    let _cancel_socket =
        http_deadline::interrupt_on_cancel(&stream).map_err(SyncClientIoError::Connect)?;
    let _verified_peer_image = peer_policy::verify(&stream).map_err(SyncClientIoError::Connect)?;
    let mut stream =
        http_deadline::DeadlineStream::new(stream, deadline).map_err(SyncClientIoError::Connect)?;
    if let Err(error) = stream.write_all(request.as_bytes()) {
        return Err(SyncClientIoError::Send(error));
    }
    // Once the request bytes have been sent, a read failure is ambiguous: the
    // server may already have committed the mutation. Never issue a second
    // POST here. Sync requests carry a request id as a second line of defence.
    let response = read_http_response(&mut stream).map_err(SyncClientIoError::Read)?;
    http_deadline::remaining(deadline, SYNC_RESPONSE_READ_TIMEOUT)
        .map_err(SyncClientIoError::Read)?;
    Ok(response)
}

/// Revoke through the same guarded loopback connection used by account sync.
/// An HTTP 401 means the token is already unusable; no request is replayed.
pub fn revoke_loopback_token_once(server_url: &str, token: &str) -> bool {
    if token.trim().is_empty() {
        return true;
    }
    let Ok(base) = parse_base_url(server_url) else {
        return false;
    };
    if base.scheme != SyncUrlScheme::Http || !is_loopback_sync_host(&base.host) {
        return false;
    }
    let Ok(bytes) =
        send_http_json_with_timeout(&base, "v1/logout", Some(token), "{}", SYNC_LOGOUT_TIMEOUT)
    else {
        return false;
    };
    let response = String::from_utf8_lossy(&bytes);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok());
    status == Some(401) || parse_client_response(&bytes).ok
}

/// Read-only health probe using cancellable Windows I/O and a bounded response.
#[cfg(target_os = "windows")]
pub fn windows_health_response(
    server_url: &str,
    timeout: Duration,
    limit: usize,
) -> io::Result<(u16, String)> {
    let base = parse_base_url(server_url)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    windows_http::request(
        "GET",
        &format_request_url(&base, "health"),
        None,
        "",
        timeout,
        limit.min(MAX_RESPONSE_BODY_BYTES),
    )
}

#[cfg(target_os = "windows")]
pub fn revoke_windows_token_once(server_url: &str, token: &str) -> bool {
    if token.trim().is_empty() {
        return true;
    }
    let Ok(base) = parse_base_url(server_url) else {
        return false;
    };
    if base.scheme == SyncUrlScheme::Http {
        return revoke_loopback_token_once(server_url, token);
    }
    let Ok((status, body)) = windows_http::request(
        "POST",
        &format_request_url(&base, "v1/logout"),
        Some(token),
        "{}",
        SYNC_LOGOUT_TIMEOUT,
        MAX_RESPONSE_BODY_BYTES,
    ) else {
        return false;
    };
    status == 401 || parse_sync_body_response(status, &body).ok
}

#[cfg(any(test, not(target_os = "windows")))]
fn send_json_to_ureq_base(
    base: &ParsedBaseUrl,
    endpoint: &str,
    bearer_token: Option<&str>,
    request_body: &str,
) -> Result<SyncClientResult, SyncClientIoError> {
    // Keep TLS configuration and the connection pool for this process. A read
    // timeout alone permits a peer to drip bytes forever; the total deadline
    // covers request transmission and body consumption as well.
    static AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(SYNC_CONNECT_TIMEOUT)
            .timeout_read(SYNC_RESPONSE_READ_TIMEOUT)
            .timeout_write(SYNC_WRITE_TIMEOUT)
            .timeout(SYNC_REQUEST_TIMEOUT)
            .redirects(0)
            .build()
    });
    let url = format_request_url(base, endpoint);
    let mut request = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");
    if let Some(token) = bearer_token
        .map(str::trim)
        .filter(|token| !token.is_empty())
    {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    match request.send_string(request_body) {
        Ok(response) => {
            let status = response.status();
            let raw = read_bounded_ureq_response(response).map_err(SyncClientIoError::Read)?;
            Ok(parse_sync_body_response(status, &raw))
        }
        Err(ureq::Error::Status(status, response)) => {
            let raw = read_bounded_ureq_response(response).map_err(SyncClientIoError::Read)?;
            Ok(parse_sync_body_response(status, &raw))
        }
        Err(error) => Err(SyncClientIoError::Connect(io::Error::new(
            io::ErrorKind::Other,
            error.to_string(),
        ))),
    }
}

#[cfg(any(test, not(target_os = "windows")))]
fn read_bounded_ureq_response(response: ureq::Response) -> io::Result<String> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_RESPONSE_BODY_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RESPONSE_BODY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sync response exceeds the configured body limit",
        ));
    }
    String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
fn discover_sync_server_base(base: &ParsedBaseUrl) -> Option<ParsedBaseUrl> {
    if base.scheme != SyncUrlScheme::Http {
        return None;
    }
    let host_ip = base.host.parse::<Ipv4Addr>().ok()?;
    if !is_discoverable_lan_ipv4(host_ip) {
        return None;
    }

    let candidates = sibling_ipv4_candidates(host_ip);
    if candidates.is_empty() {
        return None;
    }

    let queue = Arc::new(Mutex::new(VecDeque::from(candidates)));
    let found = Arc::new(AtomicBool::new(false));
    let worker_count = SYNC_DISCOVERY_WORKERS.min(queue.lock().ok()?.len()).max(1);
    let (tx, rx) = mpsc::channel::<Ipv4Addr>();
    let mut handles = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let found = Arc::clone(&found);
        let tx = tx.clone();
        let base_path = base.base_path.clone();
        let port = base.port;
        handles.push(thread::spawn(move || loop {
            if found.load(Ordering::Relaxed) {
                break;
            }
            let candidate = match queue.lock().ok().and_then(|mut locked| locked.pop_front()) {
                Some(value) => value,
                None => break,
            };
            if sync_health_probe(candidate, port, &base_path) {
                found.store(true, Ordering::Relaxed);
                let _ = tx.send(candidate);
                break;
            }
        }));
    }
    drop(tx);

    let discovered = rx.recv_timeout(SYNC_DISCOVERY_TOTAL_TIMEOUT).ok();
    found.store(true, Ordering::Relaxed);
    for handle in handles {
        let _ = handle.join();
    }

    discovered.map(|ip| ParsedBaseUrl {
        scheme: base.scheme,
        host: ip.to_string(),
        port: base.port,
        base_path: base.base_path.clone(),
    })
}

#[cfg(test)]
fn sync_health_probe(ip: Ipv4Addr, port: u16, base_path: &str) -> bool {
    let socket_addr = SocketAddr::from((ip, port));
    let mut stream = match TcpStream::connect_timeout(&socket_addr, SYNC_DISCOVERY_CONNECT_TIMEOUT)
    {
        Ok(value) => value,
        Err(_) => return false,
    };
    let _ = stream.set_read_timeout(Some(SYNC_DISCOVERY_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SYNC_DISCOVERY_IO_TIMEOUT));
    let path = join_paths(base_path, "/health");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {ip}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    read_http_response(&mut stream)
        .map(|bytes| {
            let response = parse_client_response(&bytes);
            response.ok && response.mode == "health"
        })
        .unwrap_or(false)
}

#[cfg(test)]
fn is_discoverable_lan_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

#[cfg(test)]
fn sibling_ipv4_candidates(ip: Ipv4Addr) -> Vec<Ipv4Addr> {
    let [a, b, c, current] = ip.octets();
    (1u8..=254)
        .filter(|last| *last != current)
        .map(|last| Ipv4Addr::new(a, b, c, last))
        .collect()
}

fn parse_client_response(bytes: &[u8]) -> SyncClientResult {
    let response = String::from_utf8_lossy(bytes);
    let Some(header_end) = response.find("\r\n\r\n") else {
        return error_result("Invalid sync response.");
    };
    let (head, body_with_separator) = response.split_at(header_end);
    let body = &body_with_separator[4..];
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    parse_sync_body_response(status, body)
}

fn parse_sync_body_response(status: u16, body: &str) -> SyncClientResult {
    let mut result = serde_json::from_str::<SyncClientResult>(body)
        .unwrap_or_else(|_| error_result("Sync server returned an unreadable response."));
    if !(200..300).contains(&status) {
        result.ok = false;
        if result.message.trim().is_empty() {
            result.message = format!("Sync server returned HTTP {status}.");
        }
    }
    result
}

fn read_http_response(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut buffer = Vec::<u8>::new();
    let mut chunk = [0u8; 4096];
    let mut expected_total_bytes: Option<usize> = None;

    loop {
        if let Some(total_bytes) = expected_total_bytes {
            if buffer.len() >= total_bytes {
                buffer.truncate(total_bytes);
                return Ok(buffer);
            }
        }

        let read = reader.read(&mut chunk)?;
        if read == 0 {
            if let Some(total_bytes) = expected_total_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "connection closed before reading full response body (received {} of {} bytes)",
                        buffer.len(),
                        total_bytes
                    ),
                ));
            }
            return Ok(buffer);
        }

        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_RESPONSE_BODY_BYTES.saturating_add(8192) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response body too large",
            ));
        }
        if expected_total_bytes.is_none() {
            if let Some(header_end) = find_header_end(&buffer) {
                let body_start = header_end + 4;
                let content_length = parse_content_length(&buffer[..header_end], true)?;
                expected_total_bytes = content_length.map(|value| body_start + value);
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn handle_connection(
    mut stream: TcpStream,
    store: Arc<SqliteServerStore>,
    runtime_info: Arc<Mutex<ServerRuntimeInfo>>,
    login_rate_limiter: Arc<Mutex<LoginRateLimiter>>,
) -> io::Result<()> {
    let peer_address = stream.peer_addr().ok();
    let peer_is_loopback = peer_address
        .map(|address| address.ip().is_loopback())
        .unwrap_or(false);
    let request = match read_http_request(&mut stream) {
        Ok(value) => value,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof
            ) =>
        {
            return write_json_response(&mut stream, 400, &error_result("Invalid HTTP request."));
        }
        Err(error) => return Err(error),
    };
    let route = request.path.split('?').next().unwrap_or(&request.path);
    let direct_http_allowed = matches!(
        (request.method.as_str(), route),
        ("GET", "/health") | ("POST", "/v1/discovery-proof")
    );
    if !peer_is_loopback && !direct_http_allowed {
        return write_json_response(
            &mut stream,
            403,
            &error_result("Account requests must arrive through the local HTTPS tunnel."),
        );
    }
    if request.method == "POST" && matches!(route, "/v1/login" | "/v1/register") {
        if let Some(address) = peer_address.map(|value| value.ip()) {
            let allowed = login_rate_limiter
                .lock()
                .map(|mut limiter| limiter.allow(address, now_millis()))
                .unwrap_or(false);
            if !allowed {
                return write_json_response(
                    &mut stream,
                    429,
                    &error_result("Too many account attempts. Try again shortly."),
                );
            }
        }
    }
    let response = match (request.method.as_str(), route) {
        ("GET", "/health") => {
            refresh_configured_public_runtime_info(&runtime_info);
            let info = runtime_info
                .lock()
                .map(|locked| locked.clone())
                .unwrap_or_default();
            let privacy_status = backup_worker::visible_status(&info);
            (
                200,
                SyncClientResult {
                    ok: true,
                    message: "Sync server is running.".to_string(),
                    product_id: product_identity::PRODUCT.internal_id.to_string(),
                    service_role: "sync_server".to_string(),
                    sync_protocol_version: SYNC_PROTOCOL_VERSION,
                    mode: "health".to_string(),
                    server_process_name: current_server_process_name(),
                    server_build_id: SYNC_SERVER_BUILD_ID.to_string(),
                    server_git_commit: product_identity::BUILD_GIT_COMMIT.to_string(),
                    server_source_snapshot_sha256: product_identity::BUILD_SOURCE_SNAPSHOT_SHA256
                        .to_string(),
                    server_process_id: std::process::id(),
                    public_server_url: info.public_server_url,
                    public_access_message: info.public_access_message,
                    backup_status: info.backup_status,
                    last_backup_success_at_epoch_millis: info.last_backup_success_at_epoch_millis,
                    last_backup_failure_at_epoch_millis: info.last_backup_failure_at_epoch_millis,
                    backup_message: info.backup_message,
                    backup_privacy: privacy_status,
                    ..SyncClientResult::default()
                },
            )
        }
        ("POST", "/v1/discovery-proof") => {
            handle_discovery_proof(&request.body, &store, &runtime_info)
        }
        ("POST", "/v1/register") => handle_register(&request.body, &store),
        ("POST", "/v1/login") => handle_login(&request.body, &store),
        ("POST", "/v1/logout") => handle_logout(&request, &store),
        ("POST", "/v1/sync") => handle_sync(&request, &store),
        ("POST", "/v1/upload-local") => handle_sync_mode(&request, &store, true),
        ("POST", "/v1/media/manifest") => handle_media_manifest(&request, &store),
        ("POST", "/v1/media/upload") => handle_media_upload(&request, &store),
        ("POST", "/v1/media/download") => handle_media_download(&request, &store),
        ("POST", "/v1/media/delete") => handle_media_delete(&request, &store),
        ("POST", "/v1/media/private-references") => private_media::handle(&request, &store),
        ("POST", "/v1/legal-reports/manifest")
        | ("POST", "/v1/legal-reports/upload")
        | ("POST", "/v1/legal-reports/download")
        | ("POST", "/v1/legal-reports/delete") => legal_reports::handle(route, &request, &store),
        _ => (404, error_result("Endpoint not found.")),
    };
    let response = bind_response_account_identity(&request, route, &store, response);
    write_json_response(&mut stream, response.0, &response.1)
}

#[cfg(not(target_os = "android"))]
fn current_server_process_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
fn bind_response_account_identity(
    request: &HttpRequest,
    route: &str,
    store: &SqliteServerStore,
    response: (u16, SyncClientResult),
) -> (u16, SyncClientResult) {
    let (status, mut result) = response;
    let token_scoped = matches!(
        (request.method.as_str(), route),
        ("POST", "/v1/logout")
            | ("POST", "/v1/sync")
            | ("POST", "/v1/upload-local")
            | ("POST", "/v1/media/manifest")
            | ("POST", "/v1/media/upload")
            | ("POST", "/v1/media/download")
            | ("POST", "/v1/media/delete")
            | ("POST", "/v1/media/private-references")
            | ("POST", "/v1/legal-reports/manifest")
            | ("POST", "/v1/legal-reports/upload")
            | ("POST", "/v1/legal-reports/download")
            | ("POST", "/v1/legal-reports/delete")
    );
    if token_scoped {
        if let Some(raw_token) = bearer_token(request) {
            if let Ok(
                TokenAuthentication::Active(token) | TokenAuthentication::PendingActivation(token),
            ) = store.authenticate_token(raw_token, now_millis())
            {
                // Never echo a request-body or stale response user identifier.
                // An authenticated response is always scoped to the token owner.
                result.user_id = token.user_id;
                result.token_id = token.token_identifier;
            }
        }
    }
    // Storage failures also need authenticated response identity so clients
    // can display the actual failure without mistaking it for account drift.
    if result.user_id.trim().is_empty() {
        return (status, result);
    }
    match store.server_account_identity(&result.user_id) {
        Ok(identity) => {
            result.server_instance_id = identity.server_instance_id;
            result.account_namespace = identity.account_namespace;
            (status, result)
        }
        Err(error) => store_failure("Could not read account namespace", error),
    }
}

#[cfg(not(target_os = "android"))]
fn handle_register(body: &str, store: &SqliteServerStore) -> (u16, SyncClientResult) {
    let request = match serde_json::from_str::<RegisterRequest>(body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid registration request.")),
    };
    let email = normalized_email(&request.email);
    if !valid_email(&email) {
        return (400, error_result("Email is invalid."));
    }
    if request.password.chars().count() < 6 {
        return (400, error_result("Password must be at least 6 characters."));
    }
    if request.password.chars().count() > 1024 {
        return (400, error_result("Password is too long."));
    }
    if let Err(error) = store.cleanup_expired_pending_tokens(now_millis()) {
        return store_failure("Could not clean up expired login leases", error);
    }
    match store.find_user_by_email(&email) {
        Ok(Some(_)) => return (409, error_result("Account already exists.")),
        Ok(None) => {}
        Err(error) => return store_failure("Could not inspect account", error),
    }
    let password_hash = match hash_password_argon2(&request.password) {
        Ok(value) => value,
        Err(message) => return (500, error_result(&message)),
    };
    let now = now_millis();
    let user_id = format!("user-{}", random_token(12));
    let token = random_token(32);
    let metadata = match store.create_user_with_initial_pending_token(
        NewStoredUser {
            id: user_id.clone(),
            email,
            password_salt: String::new(),
            password_hash,
            password_scheme: "argon2id_phc".to_string(),
            created_at_epoch_millis: now,
            updated_at_epoch_millis: now,
            app_data_json: String::new(),
            account_revision: 0,
        },
        &token,
        &normalized_device_name(&request.device_name),
        now,
        now.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS),
    ) {
        Ok(value) => value,
        Err(error) if error.to_string().contains("UNIQUE constraint failed") => {
            return (409, error_result("Account already exists."));
        }
        Err(error) => return store_failure("Could not save account", error),
    };
    (
        200,
        SyncClientResult {
            ok: true,
            message: "Account created.".to_string(),
            user_id,
            token,
            token_id: metadata.token_id,
            mode: "registered".to_string(),
            ..SyncClientResult::default()
        },
    )
}

#[cfg(not(target_os = "android"))]
fn handle_login(body: &str, store: &SqliteServerStore) -> (u16, SyncClientResult) {
    let request = match serde_json::from_str::<LoginRequest>(body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid login request.")),
    };
    let email = normalized_email(&request.email);
    if !valid_email(&email) || request.password.chars().count() > 1_024 {
        return (401, error_result("Account or password is wrong."));
    }
    let now = now_millis();
    let user = match store.find_user_by_email(&email) {
        Ok(Some(value)) => value,
        Ok(None) => return (401, error_result("Account or password is wrong.")),
        Err(error) => return store_failure("Could not read account", error),
    };
    let password_matches = match verify_stored_password(&user, &request.password) {
        Ok(value) => value,
        Err(message) => return (500, error_result(&message)),
    };
    if !password_matches {
        return (401, error_result("Account or password is wrong."));
    }
    if user.password_scheme == "legacy_sha256" {
        let upgraded = match hash_password_argon2(&request.password) {
            Ok(value) => value,
            Err(message) => return (500, error_result(&message)),
        };
        if let Err(error) = store.update_password_hash(&user.id, "", &upgraded, "argon2id_phc", now)
        {
            return store_failure("Could not upgrade password protection", error);
        }
    }
    if let Err(error) = store.cleanup_expired_pending_tokens(now) {
        return store_failure("Could not clean up expired login leases", error);
    }
    let token = random_token(32);
    let metadata = match store.issue_pending_token(
        &user.id,
        &token,
        &normalized_device_name(&request.device_name),
        now,
        now.saturating_add(TOKEN_ACTIVATION_LEASE_MILLIS),
    ) {
        Ok(value) => value,
        Err(error) => return store_failure("Could not save login", error),
    };
    (
        200,
        SyncClientResult {
            ok: true,
            message: "Logged in.".to_string(),
            user_id: user.id,
            token,
            token_id: metadata.token_id,
            mode: "logged_in".to_string(),
            ..SyncClientResult::default()
        },
    )
}

#[cfg(not(target_os = "android"))]
fn handle_logout(request: &HttpRequest, store: &SqliteServerStore) -> (u16, SyncClientResult) {
    let raw_token = match bearer_token(request) {
        Some(value) => value,
        None => return (401, error_result("Missing sync token.")),
    };
    match store.revoke_token_for_logout(raw_token, now_millis()) {
        Ok(Some(user_id)) => (
            200,
            SyncClientResult {
                ok: true,
                message: "Logged out.".to_string(),
                user_id,
                mode: "logged_out".to_string(),
                ..SyncClientResult::default()
            },
        ),
        Ok(None) => (401, error_result("Login is invalid.")),
        Err(error) => store_failure("Could not revoke login token", error),
    }
}

#[cfg(not(target_os = "android"))]
fn handle_sync(request: &HttpRequest, store: &SqliteServerStore) -> (u16, SyncClientResult) {
    handle_sync_mode(request, store, false)
}

#[cfg(not(target_os = "android"))]
fn handle_sync_mode(
    request: &HttpRequest,
    store: &SqliteServerStore,
    force_upload_endpoint: bool,
) -> (u16, SyncClientResult) {
    let raw_token = match bearer_token(request) {
        Some(value) => value,
        None => return (401, error_result("Missing sync token.")),
    };
    let snapshot = match serde_json::from_str::<SyncSnapshotRequest>(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid sync request.")),
    };
    if !valid_request_id(&snapshot.request_id) {
        return (400, error_result("Sync requestId is required."));
    }
    let now = now_millis();
    let force_upload = force_upload_endpoint || snapshot.force_upload;
    let force_download = !force_upload && snapshot.force_download;
    let cleanup_recovery_allowed =
        !force_upload && !force_download && !snapshot.allow_workspace_identity_rebind;
    let token_authentication = match store.authenticate_token(raw_token, now) {
        Ok(value) => value,
        Err(error) => return store_failure("Could not verify login token", error),
    };
    let (authenticated, pending_activation, cleanup_recovery_original_expiry) =
        match token_authentication {
            TokenAuthentication::Active(token) => {
                let token = match renew_active_token_if_due(store, token, now) {
                    Ok(token) => token,
                    Err(response) => return response,
                };
                (token, false, None)
            }
            TokenAuthentication::PendingActivation(token) => (token, true, None),
            TokenAuthentication::Unknown => return (401, error_result("Login is invalid.")),
            TokenAuthentication::Expired => {
                match recover_recent_active_token_if_eligible(store, raw_token, now) {
                    Ok(Some(token)) => (token, false, None),
                    Ok(None) => {
                        if !cleanup_recovery_allowed {
                            return (401, error_result("Login expired."));
                        }
                        match store.pending_token_recovery_candidate_for_normal_sync(
                            raw_token,
                            now,
                            TOKEN_ACTIVATION_LEASE_MILLIS,
                            PENDING_TOKEN_RECOVERY_WINDOW_MILLIS,
                        ) {
                            Ok(Some(candidate)) => (
                                candidate.token,
                                true,
                                Some(candidate.original_expires_at_epoch_millis),
                            ),
                            Ok(None) => return (401, error_result("Login expired.")),
                            Err(error) => {
                                return store_failure(
                                    "Could not verify pending login recovery",
                                    error,
                                )
                            }
                        }
                    }
                    Err(response) => return response,
                }
            }
            TokenAuthentication::Revoked => {
                if !cleanup_recovery_allowed {
                    return (401, error_result("Login was revoked."));
                }
                match store.pending_token_recovery_candidate_for_normal_sync(
                    raw_token,
                    now,
                    TOKEN_ACTIVATION_LEASE_MILLIS,
                    PENDING_TOKEN_RECOVERY_WINDOW_MILLIS,
                ) {
                    Ok(Some(candidate)) => (
                        candidate.token,
                        true,
                        Some(candidate.original_expires_at_epoch_millis),
                    ),
                    Ok(None) => return (401, error_result("Login was revoked.")),
                    Err(error) => {
                        return store_failure("Could not verify pending login recovery", error)
                    }
                }
            }
        };
    if pending_activation && (force_upload || force_download) {
        let mut result = error_result("Complete a normal sync before using this login token.");
        result.mode = "token_activation_required".to_string();
        return (409, result);
    }
    let user_id = authenticated.user_id.clone();
    let barrier = match store.restore_barrier_state(&user_id, authenticated.token_id) {
        Ok(value) => value,
        Err(error) => return store_failure("Could not read account restore state", error),
    };
    let account_identity = match store.server_account_identity(&user_id) {
        Ok(value) => value,
        Err(error) => return store_failure("Could not read account workspace identity", error),
    };
    if snapshot.allow_workspace_identity_rebind {
        let previous_server_instance_id = snapshot.previous_server_instance_id.trim();
        let previous_account_namespace = snapshot.previous_account_namespace.trim();
        let workspace_identity_rebind_allowed = !pending_activation
            && !force_upload_endpoint
            && !force_upload
            && !force_download
            && snapshot.server_instance_id == account_identity.server_instance_id
            && snapshot.account_namespace == account_identity.account_namespace
            && valid_workspace_capability_component(previous_server_instance_id)
            && valid_workspace_capability_component(previous_account_namespace)
            && previous_server_instance_id != account_identity.server_instance_id
            && previous_account_namespace != account_identity.account_namespace
            && previous_account_namespace
                == account_namespace_identifier(previous_server_instance_id, &user_id)
            && valid_workspace_capability_component(&snapshot.workspace_id)
            && snapshot.workspace_proof.is_empty()
            && snapshot.acknowledged_generation == 0
            && barrier.current_generation == 0
            && !barrier.token_restore_acknowledged
            && snapshot.restore_receipt.trim().is_empty();
        if !workspace_identity_rebind_allowed {
            return workspace_capability_error_response(
                &user_id,
                &authenticated.token_identifier,
                &account_identity.server_instance_id,
                &account_identity.account_namespace,
                barrier.current_generation,
                "workspace_identity_rebind_invalid",
                "Sync workspace identity rebind request is not eligible.",
            );
        }
        let (status, mut result) = restore_required_response_for_workspace(
            store,
            &user_id,
            authenticated.token_id,
            barrier.current_generation,
            snapshot.client_updated_at_epoch_millis.max(0),
            &snapshot.workspace_id,
            true,
        );
        if status == 200
            && result.ok
            && result.mode == "baseline_required"
            && result.restore_required
            && result.baseline_merge_required
            && result.current_generation == 0
            && result.workspace_proof.is_empty()
        {
            result.workspace_identity_rebound = true;
        }
        return (status, result);
    }
    let legacy_identity = snapshot.server_instance_id.is_empty()
        && snapshot.account_namespace.is_empty()
        && snapshot.workspace_id.is_empty()
        && snapshot.workspace_proof.is_empty();
    let identity_bound = snapshot.server_instance_id == account_identity.server_instance_id
        && snapshot.account_namespace == account_identity.account_namespace;
    if !legacy_identity && !identity_bound {
        return workspace_capability_error_response(
            &user_id,
            &authenticated.token_identifier,
            &account_identity.server_instance_id,
            &account_identity.account_namespace,
            barrier.current_generation,
            "workspace_binding_mismatch",
            "Sync workspace identity does not match the authenticated account.",
        );
    }
    if !snapshot.workspace_id.is_empty()
        && !valid_workspace_capability_component(&snapshot.workspace_id)
    {
        return workspace_capability_error_response(
            &user_id,
            &authenticated.token_identifier,
            &account_identity.server_instance_id,
            &account_identity.account_namespace,
            barrier.current_generation,
            "workspace_id_invalid",
            "Workspace identifier is invalid.",
        );
    }
    if !snapshot.workspace_proof.is_empty()
        && (!valid_workspace_capability_component(&snapshot.workspace_proof)
            || snapshot.workspace_id.is_empty())
    {
        return workspace_capability_error_response(
            &user_id,
            &authenticated.token_identifier,
            &account_identity.server_instance_id,
            &account_identity.account_namespace,
            barrier.current_generation,
            "workspace_proof_invalid",
            "Workspace capability proof is invalid.",
        );
    }
    if snapshot.acknowledged_generation > barrier.current_generation {
        return server_generation_rollback_response(&user_id, barrier.current_generation);
    }
    if snapshot.acknowledged_generation < barrier.current_generation {
        return restore_required_response_for_workspace(
            store,
            &user_id,
            authenticated.token_id,
            barrier.current_generation,
            snapshot.client_updated_at_epoch_millis.max(0),
            &snapshot.workspace_id,
            false,
        );
    }
    let workspace_capability_valid = if snapshot.workspace_proof.is_empty() {
        false
    } else {
        match store.verify_workspace_capability(
            &user_id,
            &snapshot.workspace_id,
            snapshot.acknowledged_generation,
            &snapshot.workspace_proof,
        ) {
            Ok(true) => true,
            Ok(false) => {
                return workspace_capability_error_response(
                    &user_id,
                    &authenticated.token_identifier,
                    &account_identity.server_instance_id,
                    &account_identity.account_namespace,
                    barrier.current_generation,
                    "workspace_proof_invalid",
                    "Workspace capability proof is invalid.",
                )
            }
            Err(StoreError::RestoreGenerationConflict {
                actual_generation, ..
            }) => {
                return restore_required_response_for_workspace(
                    store,
                    &user_id,
                    authenticated.token_id,
                    actual_generation,
                    snapshot.client_updated_at_epoch_millis.max(0),
                    &snapshot.workspace_id,
                    false,
                )
            }
            Err(StoreError::ServerGenerationRollback {
                server_generation, ..
            }) => return server_generation_rollback_response(&user_id, server_generation),
            Err(error) => return store_failure("Could not verify workspace capability", error),
        }
    };
    let allow_receiptless_workspace_baseline = !barrier.token_restore_acknowledged
        && snapshot.restore_receipt.trim().is_empty()
        && workspace_capability_valid
        && !force_upload
        && !force_download;
    if !barrier.token_restore_acknowledged
        && snapshot.restore_receipt.trim().is_empty()
        && !allow_receiptless_workspace_baseline
    {
        let baseline_merge_required = barrier.current_generation == 0
            && identity_bound
            && !snapshot.workspace_id.is_empty()
            && snapshot.workspace_proof.is_empty()
            && !force_upload
            && !force_download;
        return restore_required_response_for_workspace(
            store,
            &user_id,
            authenticated.token_id,
            barrier.current_generation,
            snapshot.client_updated_at_epoch_millis.max(0),
            &snapshot.workspace_id,
            baseline_merge_required,
        );
    }
    match inspect_app_data_schema(&snapshot.app_data_json) {
        AppDataSchemaStatus::Supported => {}
        AppDataSchemaStatus::Future(version) => {
            return (426, upgrade_required_result(&version, false));
        }
        AppDataSchemaStatus::Invalid => return (400, error_result("App data is invalid.")),
    }
    let sanitized_client_json = match sanitize_sync_app_data(&snapshot.app_data_json, now) {
        Some(value) => value,
        None => return (400, error_result("App data is invalid.")),
    };
    let client_revision = app_data_revision_millis(
        &sanitized_client_json,
        snapshot.client_updated_at_epoch_millis,
    );
    let device_name = normalized_device_name(&snapshot.device_name);
    let operation_payload_json = json!({
        "operation": "sync_snapshot_v1",
        "appDataJson": &sanitized_client_json,
        "clientUpdatedAtEpochMillis": snapshot.client_updated_at_epoch_millis,
        "acknowledgedGeneration": snapshot.acknowledged_generation,
        "serverInstanceId": &snapshot.server_instance_id,
        "accountNamespace": &snapshot.account_namespace,
        "workspaceId": &snapshot.workspace_id,
        "workspaceProof": &snapshot.workspace_proof,
        "allowWorkspaceIdentityRebind": snapshot.allow_workspace_identity_rebind,
        "previousServerInstanceId": &snapshot.previous_server_instance_id,
        "previousAccountNamespace": &snapshot.previous_account_namespace,
        "tokenId": &authenticated.token_identifier,
        "deviceName": device_name,
        "forceUpload": force_upload,
        "forceDownload": force_download
    })
    .to_string();
    let pending_token_activation = pending_activation.then_some(PendingTokenActivation {
        token_id: authenticated.token_id,
        activated_at_epoch_millis: now,
        active_expires_at_epoch_millis: now.saturating_add(TOKEN_TTL_MILLIS),
        cleanup_recovery_original_expiry_epoch_millis: cleanup_recovery_original_expiry,
        cleanup_recovery_original_lease_millis: if cleanup_recovery_original_expiry.is_some() {
            TOKEN_ACTIVATION_LEASE_MILLIS
        } else {
            0
        },
        cleanup_recovery_window_millis: if cleanup_recovery_original_expiry.is_some() {
            PENDING_TOKEN_RECOVERY_WINDOW_MILLIS
        } else {
            0
        },
    });
    if force_download {
        if let Err(error) = store.acknowledge_restore_generation(
            &user_id,
            authenticated.token_id,
            barrier.current_generation,
            snapshot.restore_receipt.trim(),
        ) {
            return match error {
                StoreError::RestoreGenerationConflict {
                    actual_generation, ..
                } => restore_required_response_for_workspace(
                    store,
                    &user_id,
                    authenticated.token_id,
                    actual_generation,
                    client_revision,
                    &snapshot.workspace_id,
                    false,
                ),
                StoreError::RestoreReceiptRequired { actual_generation } => {
                    restore_required_response_for_workspace(
                        store,
                        &user_id,
                        authenticated.token_id,
                        actual_generation,
                        client_revision,
                        &snapshot.workspace_id,
                        false,
                    )
                }
                StoreError::ServerGenerationRollback {
                    server_generation, ..
                } => server_generation_rollback_response(&user_id, server_generation),
                error => store_failure("Could not acknowledge account restore", error),
            };
        }
        return match store.read_account(&user_id) {
            Ok(account) => match inspect_app_data_schema(&account.app_data_json) {
                AppDataSchemaStatus::Future(version) => {
                    (426, upgrade_required_result(&version, true))
                }
                AppDataSchemaStatus::Invalid => {
                    (500, error_result("Stored account data is invalid."))
                }
                AppDataSchemaStatus::Supported => {
                    let quota = match store.snapshot_history_quota_status(&user_id) {
                        Ok(value) => value,
                        Err(error) => {
                            return store_failure("Could not read snapshot history quota", error)
                        }
                    };
                    let mut result = SyncClientResult {
                        ok: true,
                        message: "Pulled account data.".to_string(),
                        user_id: user_id.clone(),
                        app_data_json: Some(account.app_data_json.clone()),
                        server_updated_at_epoch_millis: app_data_revision_millis(
                            &account.app_data_json,
                            account.updated_at_epoch_millis,
                        ),
                        client_updated_at_epoch_millis: client_revision,
                        current_generation: barrier.current_generation,
                        mode: "downloaded_account".to_string(),
                        current_committed: true,
                        app_data_current_bytes: account.app_data_json.len() as i64,
                        app_data_projected_bytes: account.app_data_json.len() as i64,
                        app_data_limit_bytes: ACCOUNT_APP_DATA_HARD_LIMIT_BYTES,
                        ..SyncClientResult::default()
                    };
                    bind_snapshot_history_quota(&mut result, quota);
                    if let Err(error) = attach_workspace_capability(
                        store,
                        &user_id,
                        &snapshot.workspace_id,
                        barrier.current_generation,
                        &mut result,
                    ) {
                        return workspace_capability_store_error_response(
                            store,
                            &user_id,
                            authenticated.token_id,
                            &snapshot.workspace_id,
                            client_revision,
                            error,
                        );
                    }
                    match prepare_sync_payload(&mut result) {
                        Ok(_) => (200, result),
                        Err(error) => (507, error),
                    }
                }
            },
            Err(error) => store_failure("Could not read account data", error),
        };
    }

    for _ in 0..6 {
        let account = match store.read_account(&user_id) {
            Ok(value) => value,
            Err(error) => return store_failure("Could not read account data", error),
        };
        match inspect_app_data_schema(&account.app_data_json) {
            AppDataSchemaStatus::Supported => {}
            AppDataSchemaStatus::Future(version) => {
                return (426, upgrade_required_result(&version, true));
            }
            AppDataSchemaStatus::Invalid => {
                return (500, error_result("Stored account data is invalid."));
            }
        }
        let intent_policy = match store.note_media_intent_policy(&user_id) {
            Ok(policy) => policy,
            Err(error) => return store_failure("Could not read attachment intent policy", error),
        };
        let merged_json = match merge_sync_app_data_json_with_media_intents(
            &account.app_data_json,
            &snapshot.app_data_json,
            now,
            intent_policy.note_attachment_detachments(),
            intent_policy.media_deletions(),
        ) {
            Ok(plan) => plan.app_data_json,
            Err(_) => return (409, SyncClientResult {
                ok: false,
                message: "附件删除与现有引用存在冲突，本次未改动账户数据；原引用和附件仍保留，需先恢复或确认所属笔记。".into(),
                mode: "media_deletion_intent_conflict".into(),
                current_committed: false,
                retryable: false,
                ..SyncClientResult::default()
            }),
        };
        let mode = if force_upload {
            "merged_force_upload"
        } else if merged_json == account.app_data_json {
            "unchanged"
        } else {
            "merged"
        };
        let quota = match if merged_json == account.app_data_json {
            store.snapshot_history_quota_status(&user_id)
        } else {
            store.snapshot_history_quota_after_archiving(&account)
        } {
            Ok(value) => value,
            Err(error) => return store_failure("Could not project snapshot history quota", error),
        };
        let unresolved_media =
            match crate::server_store::unresolved_media_metadata_count(&merged_json) {
                Ok(count) => count,
                Err(error) => {
                    return store_failure("Could not validate attachment metadata", error)
                }
            };
        let mut response = SyncClientResult {
            ok: true,
            message: if unresolved_media > 0 {
                format!("数据已同步；{unresolved_media} 个历史附件待恢复")
            } else if force_upload {
                "Local and account data were merged.".to_string()
            } else if mode == "unchanged" {
                "Account data is already up to date.".to_string()
            } else {
                "Local and account data were merged.".to_string()
            },
            user_id: user_id.clone(),
            app_data_json: Some(merged_json.clone()),
            server_updated_at_epoch_millis: app_data_revision_millis(&merged_json, now),
            client_updated_at_epoch_millis: client_revision,
            current_generation: barrier.current_generation,
            mode: mode.to_string(),
            current_committed: true,
            app_data_current_bytes: account.app_data_json.len() as i64,
            app_data_projected_bytes: merged_json.len() as i64,
            app_data_limit_bytes: ACCOUNT_APP_DATA_HARD_LIMIT_BYTES,
            ..SyncClientResult::default()
        };
        bind_snapshot_history_quota(&mut response, quota);
        if let Err(error) = attach_workspace_capability(
            store,
            &user_id,
            &snapshot.workspace_id,
            barrier.current_generation,
            &mut response,
        ) {
            return workspace_capability_store_error_response(
                store,
                &user_id,
                authenticated.token_id,
                &snapshot.workspace_id,
                client_revision,
                error,
            );
        }
        let response_json = match prepare_sync_payload(&mut response) {
            Ok(value) => value,
            Err(error) => return (507, error),
        };
        let apply_result = if allow_receiptless_workspace_baseline {
            store.apply_workspace_bound_sync_request_for_generation_with_pending_activation_and_media_intents(
                &user_id,
                authenticated.token_id,
                barrier.current_generation,
                snapshot.request_id.trim(),
                &operation_payload_json,
                account.revision,
                &merged_json,
                &response_json,
                now,
                pending_token_activation,
                &snapshot.app_data_json,
            )
        } else {
            store.apply_sync_request_for_generation_with_pending_activation_and_media_intents(
                &user_id,
                authenticated.token_id,
                barrier.current_generation,
                snapshot.restore_receipt.trim(),
                snapshot.request_id.trim(),
                &operation_payload_json,
                account.revision,
                &merged_json,
                &response_json,
                now,
                pending_token_activation,
                &snapshot.app_data_json,
            )
        };
        match apply_result {
            Ok(SyncRequestOutcome::Applied(receipt) | SyncRequestOutcome::Replayed(receipt)) => {
                let replayed =
                    match serde_json::from_str::<SyncClientResult>(&receipt.response_json) {
                        Ok(value) => value,
                        Err(_) => return (500, error_result("Stored sync response is invalid.")),
                    };
                return (200, replayed);
            }
            Err(StoreError::RevisionConflict { .. }) => continue,
            Err(StoreError::RestoreGenerationConflict {
                actual_generation, ..
            }) => {
                return restore_required_response_for_workspace(
                    store,
                    &user_id,
                    authenticated.token_id,
                    actual_generation,
                    client_revision,
                    &snapshot.workspace_id,
                    false,
                );
            }
            Err(StoreError::RestoreReceiptRequired { actual_generation }) => {
                return restore_required_response_for_workspace(
                    store,
                    &user_id,
                    authenticated.token_id,
                    actual_generation,
                    client_revision,
                    &snapshot.workspace_id,
                    false,
                );
            }
            Err(StoreError::ServerGenerationRollback {
                server_generation, ..
            }) => {
                return server_generation_rollback_response(&user_id, server_generation);
            }
            Err(StoreError::SnapshotHistoryQuotaExceeded {
                usage_bytes,
                projected_bytes,
                limit_bytes,
            }) => {
                return (
                    507,
                    snapshot_history_quota_exceeded_result(
                        usage_bytes,
                        projected_bytes,
                        limit_bytes,
                    ),
                );
            }
            Err(StoreError::AppDataQuotaExceeded {
                current_bytes,
                projected_bytes,
                limit_bytes,
            }) => {
                return (
                    507,
                    app_data_quota_exceeded_result(current_bytes, projected_bytes, limit_bytes),
                );
            }
            Err(StoreError::Integrity(message))
                if message.contains("request id is already owned")
                    || message.contains("request id was reused") =>
            {
                return (409, error_result("requestId is already in use."));
            }
            Err(error) => return store_failure("Could not save sync data", error),
        }
    }
    (
        409,
        error_result("Account changed concurrently. Retry with a new requestId."),
    )
}

#[cfg(not(target_os = "android"))]
fn handle_discovery_proof(
    body: &str,
    store: &SqliteServerStore,
    runtime_info: &Arc<Mutex<ServerRuntimeInfo>>,
) -> (u16, SyncClientResult) {
    refresh_configured_public_runtime_info(runtime_info);
    let public_server_url = runtime_info
        .lock()
        .map(|locked| locked.public_server_url.clone())
        .unwrap_or_default();
    handle_discovery_proof_at(body, store, &public_server_url, now_millis())
}

#[cfg(not(target_os = "android"))]
fn handle_discovery_proof_at(
    body: &str,
    store: &SqliteServerStore,
    public_server_url: &str,
    now_epoch_millis: i64,
) -> (u16, SyncClientResult) {
    let request = match serde_json::from_str::<DiscoveryProofRequest>(body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid discovery proof request.")),
    };
    let user_id = request.user_id.trim();
    let token_id = request.token_id.trim();
    let nonce = request.nonce;
    if user_id.is_empty() || token_id.len() != 32 {
        return (400, error_result("Discovery identity is invalid."));
    }
    if !(DISCOVERY_NONCE_MIN_BYTES..=DISCOVERY_NONCE_MAX_BYTES).contains(&nonce.len())
        || nonce.chars().any(char::is_control)
    {
        return (400, error_result("Discovery nonce is invalid."));
    }
    if normalized_public_server_url(public_server_url).as_deref() != Some(public_server_url) {
        return (
            503,
            error_result("Authenticated HTTPS public access is not ready."),
        );
    }
    let (key, recovered_pending_lease) = match store.discovery_token_key_by_id_with_recovery(
        user_id,
        token_id,
        &nonce,
        public_server_url,
        request.client_proof.trim(),
        now_epoch_millis,
        TOKEN_ACTIVATION_LEASE_MILLIS,
        PENDING_TOKEN_RECOVERY_WINDOW_MILLIS,
        TOKEN_TTL_MILLIS,
        ACTIVE_TOKEN_RECOVERY_GRACE_MILLIS,
        ACTIVE_TOKEN_RECENT_ACTIVITY_MILLIS,
    ) {
        Ok(DiscoveryTokenKeyLookup::Available {
            key,
            recovered_pending_lease,
        }) => (key, recovered_pending_lease),
        Ok(DiscoveryTokenKeyLookup::RecoveredActiveLease { key }) => (key, false),
        Ok(DiscoveryTokenKeyLookup::PendingReauthenticationRequired) => {
            let mut result = error_result("The pending login recovery window has expired.");
            result.mode = "pending_reauthentication_required".to_string();
            return (401, result);
        }
        Ok(DiscoveryTokenKeyLookup::Invalid) => {
            return (401, error_result("Discovery identity is not valid."));
        }
        Err(error) => return store_failure("Could not verify discovery identity", error),
    };
    let signed_message = format!("{nonce}\n{public_server_url}");
    (
        200,
        SyncClientResult {
            ok: true,
            message: if recovered_pending_lease {
                "Discovery proof created after safely renewing the pending login lease.".to_string()
            } else {
                "Discovery proof created.".to_string()
            },
            user_id: user_id.to_string(),
            mode: "discovery_proof".to_string(),
            public_server_url: public_server_url.to_string(),
            nonce,
            proof: hmac_sha256_hex(&key, signed_message.as_bytes()),
            ..SyncClientResult::default()
        },
    )
}

#[cfg(not(target_os = "android"))]
fn handle_media_manifest(
    request: &HttpRequest,
    store: &SqliteServerStore,
) -> (u16, SyncClientResult) {
    let raw_token = match bearer_token(request) {
        Some(value) => value,
        None => return (401, error_result("Missing sync token.")),
    };
    let authenticated = match authenticate_token_request(store, raw_token, now_millis()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = authenticated.user_id.clone();
    let manifest = if request.body.trim().is_empty() {
        DesktopMediaManifestRequest::default()
    } else {
        match serde_json::from_str::<DesktopMediaManifestRequest>(&request.body) {
            Ok(value) => value,
            Err(_) => return (400, error_result("Invalid media manifest request.")),
        }
    };
    match store.media_manifest_for_generation(
        &user_id,
        authenticated.token_id,
        manifest.acknowledged_generation,
        manifest.restore_receipt.trim(),
        true,
    ) {
        Ok((items, legacy_media_references)) => (
            200,
            SyncClientResult {
                ok: true,
                message: "Media manifest loaded.".to_string(),
                user_id,
                current_generation: manifest.acknowledged_generation,
                mode: "media_manifest".to_string(),
                private_media_protocol_version: 1,
                media_items: items.into_iter().map(media_manifest_item).collect(),
                legacy_media_references,
                ..SyncClientResult::default()
            },
        ),
        Err(StoreError::Integrity(message)) => (400, error_result(&message)),
        Err(StoreError::RestoreGenerationConflict {
            actual_generation, ..
        }) => media_restore_required_response(
            store,
            &user_id,
            authenticated.token_id,
            actual_generation,
        ),
        Err(StoreError::RestoreReceiptRequired { actual_generation }) => {
            media_restore_required_response(
                store,
                &user_id,
                authenticated.token_id,
                actual_generation,
            )
        }
        Err(StoreError::ServerGenerationRollback {
            server_generation, ..
        }) => server_generation_rollback_response(&user_id, server_generation),
        Err(error) => store_failure("Could not load media manifest", error),
    }
}

#[cfg(not(target_os = "android"))]
fn handle_media_upload(
    request: &HttpRequest,
    store: &SqliteServerStore,
) -> (u16, SyncClientResult) {
    let raw_token = match bearer_token(request) {
        Some(value) => value,
        None => return (401, error_result("Missing sync token.")),
    };
    let authenticated = match authenticate_token_request(store, raw_token, now_millis()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = authenticated.user_id.clone();
    let upload = match serde_json::from_str::<MediaUploadRequest>(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid media upload request.")),
    };
    if !valid_request_id(&upload.request_id) || !valid_media_attachment_id(&upload.attachment_id) {
        return (
            400,
            error_result("Media requestId or attachmentId is invalid."),
        );
    }
    if !valid_media_mime_type(&upload.mime_type) {
        return (400, error_result("Media MIME type is invalid."));
    }
    if upload.size_bytes < 1 || upload.size_bytes as u128 > MAX_MEDIA_BYTES as u128 {
        return (413, error_result("Media attachment is too large."));
    }
    if upload.updated_at_epoch_millis <= 0 {
        return (
            400,
            error_result("Media updatedAtEpochMillis must be a positive revision."),
        );
    }
    let content = match BASE64_STANDARD.decode(upload.content_base64.trim()) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Media contentBase64 is invalid.")),
    };
    if content.is_empty() || content.len() > MAX_MEDIA_BYTES {
        return (413, error_result("Media attachment is too large."));
    }
    match store.upsert_media_with_restore_for_generation(
        &user_id,
        authenticated.token_id,
        upload.acknowledged_generation,
        upload.restore_receipt.trim(),
        &upload.attachment_id,
        upload.sha256.trim(),
        &upload.mime_type,
        upload.size_bytes,
        &content,
        upload.updated_at_epoch_millis,
        upload.restore_deleted,
    ) {
        Ok(MediaUpsertOutcome::Stored(metadata)) => (
            200,
            SyncClientResult {
                ok: true,
                message: "Media uploaded.".to_string(),
                user_id,
                current_generation: upload.acknowledged_generation,
                mode: "media_upload".to_string(),
                media_item: Some(media_manifest_item(metadata)),
                ..SyncClientResult::default()
            },
        ),
        Ok(MediaUpsertOutcome::RejectedByTombstone(metadata)) => (
            409,
            SyncClientResult {
                ok: false,
                message: "Media was deleted by a newer revision; apply the returned tombstone."
                    .to_string(),
                user_id,
                current_generation: upload.acknowledged_generation,
                mode: "media_deleted".to_string(),
                media_item: Some(media_manifest_item(metadata)),
                ..SyncClientResult::default()
            },
        ),
        Err(StoreError::SnapshotHistoryQuotaExceeded {
            usage_bytes,
            projected_bytes,
            limit_bytes,
        }) => (
            507,
            snapshot_history_quota_exceeded_result(usage_bytes, projected_bytes, limit_bytes),
        ),
        Err(StoreError::Integrity(message))
            if message == "attachment id already belongs to different content" =>
        {
            (409, error_result(&message))
        }
        Err(error) if is_media_server_capacity_error(&error) => {
            (507, media_server_capacity_exceeded_result())
        }
        Err(StoreError::MediaAccountIdentityQuotaExceeded { .. }) => {
            (507, media_account_quota_exceeded_result())
        }
        Err(StoreError::Integrity(message))
            if message.contains("quota")
                || message.contains("item limit")
                || message.contains("identity limit") =>
        {
            (507, media_account_quota_exceeded_result())
        }
        Err(StoreError::Integrity(message)) if message.contains("attachment exceeds") => {
            (413, error_result("Media attachment is too large."))
        }
        Err(StoreError::Integrity(message)) => (400, error_result(&message)),
        Err(StoreError::RestoreGenerationConflict {
            actual_generation, ..
        }) => media_restore_required_response(
            store,
            &user_id,
            authenticated.token_id,
            actual_generation,
        ),
        Err(StoreError::RestoreReceiptRequired { actual_generation }) => {
            media_restore_required_response(
                store,
                &user_id,
                authenticated.token_id,
                actual_generation,
            )
        }
        Err(StoreError::ServerGenerationRollback {
            server_generation, ..
        }) => server_generation_rollback_response(&user_id, server_generation),
        Err(error) => store_failure("Could not save media attachment", error),
    }
}

#[cfg(not(target_os = "android"))]
fn handle_media_download(
    request: &HttpRequest,
    store: &SqliteServerStore,
) -> (u16, SyncClientResult) {
    let raw_token = match bearer_token(request) {
        Some(value) => value,
        None => return (401, error_result("Missing sync token.")),
    };
    let authenticated = match authenticate_token_request(store, raw_token, now_millis()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = authenticated.user_id.clone();
    let download = match serde_json::from_str::<MediaDownloadRequest>(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid media download request.")),
    };
    if !valid_media_attachment_id(&download.attachment_id) {
        return (400, error_result("attachmentId is invalid."));
    }
    match store.read_media_for_generation(
        &user_id,
        authenticated.token_id,
        download.acknowledged_generation,
        download.restore_receipt.trim(),
        &download.attachment_id,
    ) {
        Ok(Some(media)) => (
            200,
            SyncClientResult {
                ok: true,
                message: "Media downloaded.".to_string(),
                user_id,
                current_generation: download.acknowledged_generation,
                mode: "media_download".to_string(),
                media_item: Some(media_manifest_item(media.metadata)),
                media_content_base64: BASE64_STANDARD.encode(media.content),
                ..SyncClientResult::default()
            },
        ),
        Ok(None) => (404, error_result("Media attachment was not found.")),
        Err(StoreError::Integrity(message)) => (400, error_result(&message)),
        Err(StoreError::RestoreGenerationConflict {
            actual_generation, ..
        }) => media_restore_required_response(
            store,
            &user_id,
            authenticated.token_id,
            actual_generation,
        ),
        Err(StoreError::RestoreReceiptRequired { actual_generation }) => {
            media_restore_required_response(
                store,
                &user_id,
                authenticated.token_id,
                actual_generation,
            )
        }
        Err(StoreError::ServerGenerationRollback {
            server_generation, ..
        }) => server_generation_rollback_response(&user_id, server_generation),
        Err(error) => store_failure("Could not read media attachment", error),
    }
}

#[cfg(not(target_os = "android"))]
fn handle_media_delete(
    request: &HttpRequest,
    store: &SqliteServerStore,
) -> (u16, SyncClientResult) {
    let raw_token = match bearer_token(request) {
        Some(value) => value,
        None => return (401, error_result("Missing sync token.")),
    };
    let authenticated = match authenticate_token_request(store, raw_token, now_millis()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = authenticated.user_id.clone();
    let deletion = match serde_json::from_str::<MediaDeleteRequest>(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid media delete request.")),
    };
    if !valid_request_id(&deletion.request_id)
        || !valid_media_attachment_id(&deletion.attachment_id)
    {
        return (
            400,
            error_result("Media requestId or attachmentId is invalid."),
        );
    }
    if deletion.deleted_at_epoch_millis <= 0 {
        return (
            400,
            error_result("Media deletedAtEpochMillis must be a positive revision."),
        );
    }
    match store.delete_media_for_generation(
        &user_id,
        authenticated.token_id,
        deletion.acknowledged_generation,
        deletion.restore_receipt.trim(),
        &deletion.attachment_id,
        deletion.deleted_at_epoch_millis,
        now_millis(),
    ) {
        Ok(metadata) if metadata.deleted_at_epoch_millis.is_some() => (
            200,
            SyncClientResult {
                ok: true,
                message: "Media deletion recorded.".to_string(),
                user_id,
                current_generation: deletion.acknowledged_generation,
                mode: "media_delete".to_string(),
                media_item: Some(media_manifest_item(metadata)),
                ..SyncClientResult::default()
            },
        ),
        Ok(metadata) => (
            409,
            SyncClientResult {
                ok: false,
                message: "A newer media upload already exists; apply the returned revision."
                    .to_string(),
                user_id,
                current_generation: deletion.acknowledged_generation,
                mode: "media_upload_newer".to_string(),
                media_item: Some(media_manifest_item(metadata)),
                ..SyncClientResult::default()
            },
        ),
        Err(StoreError::MediaReferenceConflict { opaque }) => (
            409,
            SyncClientResult {
                ok: false,
                message: if opaque {
                    "加密笔记的附件引用尚未确认，已保留文件。"
                } else {
                    "附件仍被笔记或历史记录使用，已保留文件。"
                }
                .to_string(),
                user_id,
                current_generation: deletion.acknowledged_generation,
                mode: if opaque {
                    "media_references_unresolved"
                } else {
                    "media_referenced"
                }
                .to_string(),
                ..SyncClientResult::default()
            },
        ),
        Err(StoreError::MediaServerIdentityQuotaExceeded { .. }) => {
            (507, media_server_capacity_exceeded_result())
        }
        Err(StoreError::MediaAccountIdentityQuotaExceeded { .. }) => {
            (507, media_account_quota_exceeded_result())
        }
        Err(StoreError::Integrity(message)) if message.contains("media identity limit") => {
            (507, media_account_quota_exceeded_result())
        }
        Err(StoreError::Integrity(message)) => (400, error_result(&message)),
        Err(StoreError::RestoreGenerationConflict {
            actual_generation, ..
        }) => media_restore_required_response(
            store,
            &user_id,
            authenticated.token_id,
            actual_generation,
        ),
        Err(StoreError::RestoreReceiptRequired { actual_generation }) => {
            media_restore_required_response(
                store,
                &user_id,
                authenticated.token_id,
                actual_generation,
            )
        }
        Err(StoreError::ServerGenerationRollback {
            server_generation, ..
        }) => server_generation_rollback_response(&user_id, server_generation),
        Err(error) => store_failure("Could not delete media attachment", error),
    }
}

#[cfg(not(target_os = "android"))]
fn authenticate_token_request(
    store: &SqliteServerStore,
    raw_token: &str,
    now: i64,
) -> Result<AuthenticatedToken, (u16, SyncClientResult)> {
    match store.authenticate_token(raw_token, now) {
        Ok(TokenAuthentication::Active(token)) => renew_active_token_if_due(store, token, now),
        Ok(TokenAuthentication::PendingActivation(_)) => {
            Err((401, error_result("Login activation is pending.")))
        }
        Ok(TokenAuthentication::Unknown) => Err((401, error_result("Login is invalid."))),
        Ok(TokenAuthentication::Expired) => {
            match recover_recent_active_token_if_eligible(store, raw_token, now)? {
                Some(token) => Ok(token),
                None => Err((401, error_result("Login expired."))),
            }
        }
        Ok(TokenAuthentication::Revoked) => Err((401, error_result("Login was revoked."))),
        Err(error) => Err(store_failure("Could not verify login token", error)),
    }
}

#[cfg(not(target_os = "android"))]
fn recover_recent_active_token_if_eligible(
    store: &SqliteServerStore,
    raw_token: &str,
    now: i64,
) -> Result<Option<AuthenticatedToken>, (u16, SyncClientResult)> {
    match store.recover_recent_active_token_lease(
        raw_token,
        now,
        DESKTOP_ACTIVE_TOKEN_RECOVERY_GRACE_MILLIS,
        ACTIVE_TOKEN_RECENT_ACTIVITY_MILLIS,
        TOKEN_TTL_MILLIS,
    ) {
        Ok(Some(token)) => Ok(Some(token)),
        Ok(None) => match store.authenticate_token(raw_token, now) {
            Ok(TokenAuthentication::Active(token)) => {
                renew_active_token_if_due(store, token, now).map(Some)
            }
            Ok(_) => Ok(None),
            Err(error) => Err(store_failure("Could not verify login token", error)),
        },
        Err(error) => Err(store_failure("Could not recover recent login", error)),
    }
}

#[cfg(not(target_os = "android"))]
fn renew_active_token_if_due(
    store: &SqliteServerStore,
    mut token: AuthenticatedToken,
    now: i64,
) -> Result<AuthenticatedToken, (u16, SyncClientResult)> {
    if token.expires_at_epoch_millis > now.saturating_add(TOKEN_TTL_MILLIS / 2) {
        return Ok(token);
    }
    let renewed_expiry = now.saturating_add(TOKEN_TTL_MILLIS);
    match store.renew_active_token_lease(token.token_id, now, renewed_expiry) {
        Ok(true) => {
            token.expires_at_epoch_millis = renewed_expiry;
            Ok(token)
        }
        Ok(false) => Err((401, error_result("Login expired or was revoked."))),
        Err(error) => Err(store_failure("Could not renew login token", error)),
    }
}

#[cfg(not(target_os = "android"))]
fn attach_workspace_capability(
    store: &SqliteServerStore,
    user_id: &str,
    workspace_id: &str,
    current_generation: i64,
    result: &mut SyncClientResult,
) -> Result<(), StoreError> {
    if workspace_id.is_empty() {
        return Ok(());
    }
    result.workspace_id = workspace_id.to_string();
    result.workspace_proof =
        store.workspace_capability_proof(user_id, workspace_id, current_generation)?;
    Ok(())
}

#[cfg(not(target_os = "android"))]
fn workspace_capability_store_error_response(
    store: &SqliteServerStore,
    user_id: &str,
    token_id: i64,
    workspace_id: &str,
    client_revision: i64,
    error: StoreError,
) -> (u16, SyncClientResult) {
    match error {
        StoreError::RestoreGenerationConflict {
            actual_generation, ..
        } => restore_required_response_for_workspace(
            store,
            user_id,
            token_id,
            actual_generation,
            client_revision,
            workspace_id,
            false,
        ),
        StoreError::ServerGenerationRollback {
            server_generation, ..
        } => server_generation_rollback_response(user_id, server_generation),
        error => store_failure("Could not issue workspace capability", error),
    }
}

#[cfg(not(target_os = "android"))]
fn workspace_capability_error_response(
    user_id: &str,
    token_identifier: &str,
    server_instance_id: &str,
    account_namespace: &str,
    current_generation: i64,
    mode: &str,
    message: &str,
) -> (u16, SyncClientResult) {
    (
        409,
        SyncClientResult {
            ok: false,
            message: message.to_string(),
            user_id: user_id.to_string(),
            token_id: token_identifier.to_string(),
            server_instance_id: server_instance_id.to_string(),
            account_namespace: account_namespace.to_string(),
            current_generation,
            mode: mode.to_string(),
            retryable: false,
            current_committed: false,
            ..SyncClientResult::default()
        },
    )
}

#[cfg(not(target_os = "android"))]
fn restore_required_response_for_workspace(
    store: &SqliteServerStore,
    user_id: &str,
    token_id: i64,
    current_generation: i64,
    client_revision: i64,
    workspace_id: &str,
    baseline_merge_required: bool,
) -> (u16, SyncClientResult) {
    let (account, receipt) = match store.restored_account_with_receipt(user_id, token_id) {
        Ok(value) => value,
        Err(error) => return store_failure("Could not issue account restore receipt", error),
    };
    if receipt.current_generation < current_generation {
        return server_generation_rollback_response(user_id, receipt.current_generation);
    }
    let current_generation = receipt.current_generation;
    let baseline_merge_required = baseline_merge_required && current_generation == 0;
    match inspect_app_data_schema(&account.app_data_json) {
        AppDataSchemaStatus::Future(version) => (426, upgrade_required_result(&version, true)),
        AppDataSchemaStatus::Invalid => (500, error_result("Stored account data is invalid.")),
        AppDataSchemaStatus::Supported => {
            let quota = match store.snapshot_history_quota_status(user_id) {
                Ok(value) => value,
                Err(error) => return store_failure("Could not read snapshot history quota", error),
            };
            let mut result = SyncClientResult {
                ok: true,
                message: if baseline_merge_required {
                    "Merge this initial account baseline with the local workspace, persist it, then acknowledge the receipt."
                        .to_string()
                } else {
                    "Account data was restored. Replace the local snapshot before uploading changes."
                        .to_string()
                },
                user_id: user_id.to_string(),
                workspace_id: workspace_id.to_string(),
                app_data_json: Some(account.app_data_json.clone()),
                server_updated_at_epoch_millis: app_data_revision_millis(
                    &account.app_data_json,
                    account.updated_at_epoch_millis,
                ),
                client_updated_at_epoch_millis: client_revision,
                current_generation,
                restore_required: true,
                baseline_merge_required,
                restore_receipt: receipt.receipt,
                mode: if baseline_merge_required {
                    "baseline_required".to_string()
                } else {
                    "restore_required".to_string()
                },
                current_committed: true,
                app_data_current_bytes: account.app_data_json.len() as i64,
                app_data_projected_bytes: account.app_data_json.len() as i64,
                app_data_limit_bytes: ACCOUNT_APP_DATA_HARD_LIMIT_BYTES,
                ..SyncClientResult::default()
            };
            bind_snapshot_history_quota(&mut result, quota);
            match prepare_sync_payload(&mut result) {
                Ok(_) => (200, result),
                Err(error) => (507, error),
            }
        }
    }
}

#[cfg(not(target_os = "android"))]
fn media_restore_required_response(
    store: &SqliteServerStore,
    user_id: &str,
    token_id: i64,
    current_generation: i64,
) -> (u16, SyncClientResult) {
    let receipt = match store.ensure_restore_receipt(user_id, token_id) {
        Ok(receipt) => receipt,
        Err(error) => return store_failure("Could not issue media restore receipt", error),
    };
    if receipt.current_generation < current_generation {
        return server_generation_rollback_response(user_id, receipt.current_generation);
    }
    let current_generation = receipt.current_generation;
    (
        409,
        SyncClientResult {
            ok: false,
            message:
                "Account data was restored. Apply the restored snapshot before changing media."
                    .to_string(),
            user_id: user_id.to_string(),
            current_generation,
            restore_required: true,
            restore_receipt: receipt.receipt,
            mode: "restore_required".to_string(),
            ..SyncClientResult::default()
        },
    )
}

#[cfg(not(target_os = "android"))]
fn server_generation_rollback_response(
    user_id: &str,
    server_generation: i64,
) -> (u16, SyncClientResult) {
    (
        409,
        SyncClientResult {
            ok: false,
            message: "The server generation is older than this device. Uploads are blocked until the server database is recovered."
                .to_string(),
            user_id: user_id.to_string(),
            current_generation: server_generation,
            restore_required: false,
            mode: "server_generation_rollback".to_string(),
            ..SyncClientResult::default()
        },
    )
}

#[cfg(not(target_os = "android"))]
fn media_manifest_item(metadata: MediaMetadata) -> MediaManifestItem {
    MediaManifestItem {
        attachment_id: metadata.attachment_id,
        sha256: metadata.sha256,
        mime_type: metadata.mime_type,
        size_bytes: metadata.size_bytes,
        updated_at_epoch_millis: metadata.updated_at_epoch_millis,
        deleted_at_epoch_millis: metadata.deleted_at_epoch_millis.unwrap_or(0),
    }
}

#[cfg(not(target_os = "android"))]
fn is_media_server_capacity_error(error: &StoreError) -> bool {
    matches!(
        error,
        StoreError::MediaServerRetainedQuotaExceeded { .. }
            | StoreError::MediaServerIdentityQuotaExceeded { .. }
            | StoreError::DiskReserveExceeded { .. }
    ) || is_underlying_disk_capacity_error(error)
}

#[cfg(not(target_os = "android"))]
fn is_underlying_disk_capacity_error(error: &StoreError) -> bool {
    match error {
        StoreError::Sqlite(rusqlite::Error::SqliteFailure(error, _)) => {
            error.code == rusqlite::ErrorCode::DiskFull
        }
        StoreError::Io(error) => is_disk_full_io_error(error),
        _ => false,
    }
}

#[cfg(all(not(target_os = "android"), windows))]
fn is_disk_full_io_error(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(39 | 112))
}

#[cfg(all(not(target_os = "android"), unix))]
fn is_disk_full_io_error(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(28 | 122))
}

#[cfg(all(not(target_os = "android"), not(any(windows, unix))))]
fn is_disk_full_io_error(_error: &io::Error) -> bool {
    false
}

#[cfg(not(target_os = "android"))]
fn store_failure(label: &str, error: StoreError) -> (u16, SyncClientResult) {
    eprintln!("{label}: {error}");
    match error {
        StoreError::Integrity(message)
            if message == "new or changed attachment metadata is incomplete" =>
        {
            (
                422,
                SyncClientResult {
                    ok: false,
                    message: "附件校验信息不完整，本次同步未写入。请先恢复对应文件。".to_string(),
                    mode: "media_metadata_incomplete".to_string(),
                    ..SyncClientResult::default()
                },
            )
        }
        error if is_media_server_capacity_error(&error) => {
            (507, media_server_capacity_exceeded_result())
        }
        StoreError::MediaAccountIdentityQuotaExceeded { .. } => {
            (507, media_account_quota_exceeded_result())
        }
        StoreError::RegisteredAccountQuotaExceeded { .. } => {
            (507, registration_capacity_exceeded_result())
        }
        _ => (500, error_result("Sync store is unavailable.")),
    }
}

#[cfg(not(target_os = "android"))]
fn media_server_capacity_exceeded_result() -> SyncClientResult {
    SyncClientResult {
        ok: false,
        message: "服务器媒体空间已达到安全边界，本次操作未写入；本地数据仍完整保留。请先清理已删除附件或历史后重试。"
            .to_string(),
        mode: "media_server_capacity_exceeded".to_string(),
        retryable: false,
        current_committed: false,
        ..SyncClientResult::default()
    }
}

#[cfg(not(target_os = "android"))]
fn media_account_quota_exceeded_result() -> SyncClientResult {
    SyncClientResult {
        ok: false,
        message:
            "账户媒体空间已达到安全边界，本次操作未写入；本地数据仍完整保留。请先清理附件后重试。"
                .to_string(),
        mode: "media_account_quota_exceeded".to_string(),
        retryable: false,
        current_committed: false,
        ..SyncClientResult::default()
    }
}

#[cfg(not(target_os = "android"))]
fn registration_capacity_exceeded_result() -> SyncClientResult {
    SyncClientResult {
        ok: false,
        message: "同步服务的账户容量已满，未创建新账户。".to_string(),
        mode: "registration_capacity_exceeded".to_string(),
        retryable: false,
        current_committed: false,
        ..SyncClientResult::default()
    }
}

#[cfg(not(target_os = "android"))]
fn bind_snapshot_history_quota(result: &mut SyncClientResult, status: SnapshotHistoryQuotaStatus) {
    result.snapshot_history_usage_bytes = status.usage_bytes;
    result.snapshot_history_projected_bytes = status.projected_bytes;
    result.snapshot_history_limit_bytes = status.limit_bytes;
    result.snapshot_history_warning = status.warning.as_str().to_string();
}

#[cfg(not(target_os = "android"))]
fn snapshot_history_quota_exceeded_result(
    usage_bytes: i64,
    projected_bytes: i64,
    limit_bytes: i64,
) -> SyncClientResult {
    SyncClientResult {
        ok: false,
        message:
            "服务器历史空间已满，本次同步未写入；本地数据仍完整保留。请先导出或明确删除历史后重试。"
                .to_string(),
        mode: "snapshot_history_quota_exceeded".to_string(),
        snapshot_history_usage_bytes: usage_bytes,
        snapshot_history_projected_bytes: projected_bytes,
        snapshot_history_limit_bytes: limit_bytes,
        snapshot_history_warning: "over_limit".to_string(),
        retryable: false,
        current_committed: false,
        ..SyncClientResult::default()
    }
}

fn sync_payload_warning(usage_bytes: usize) -> &'static str {
    if usage_bytes > MAX_RESPONSE_BODY_BYTES {
        "over_limit"
    } else if usage_bytes >= SYNC_PAYLOAD_CRITICAL_BYTES {
        "critical"
    } else if usage_bytes >= SYNC_PAYLOAD_WARNING_BYTES {
        "approaching_limit"
    } else {
        ""
    }
}

fn sync_payload_quota_exceeded_result(usage_bytes: usize) -> SyncClientResult {
    SyncClientResult {
        ok: false,
        message: "同步数据已超出安全传输上限，本次未写入服务器；本地数据仍完整保留。请先导出数据并明确精简历史后重试。"
            .to_string(),
        mode: "sync_payload_quota_exceeded".to_string(),
        sync_payload_usage_bytes: usage_bytes.min(i64::MAX as usize) as i64,
        sync_payload_limit_bytes: MAX_RESPONSE_BODY_BYTES as i64,
        sync_payload_warning: "over_limit".to_string(),
        retryable: false,
        current_committed: false,
        ..SyncClientResult::default()
    }
}

#[cfg(not(target_os = "android"))]
pub(crate) fn refresh_committed_sync_response(raw: &str, current: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(raw).map_err(|_| "sync response is not valid JSON")?;
    // Store-level legacy callers also persist arbitrary JSON receipts. Only a
    // real successful snapshot-sync response uses this wire result contract.
    if value.get("ok").and_then(Value::as_bool) != Some(true)
        || value
            .get("userId")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || !matches!(
            value.get("mode").and_then(Value::as_str),
            Some("merged" | "merged_force_upload" | "unchanged")
        )
        || value.get("currentCommitted").and_then(Value::as_bool) != Some(true)
        || !value.get("appDataJson").is_some_and(Value::is_string)
    {
        return Ok(raw.to_owned());
    }
    let mut response: SyncClientResult = serde_json::from_value(value)
        .map_err(|_| "committed sync response does not match its wire contract")?;
    response.app_data_json = Some(current.to_owned());
    response.app_data_projected_bytes = current.len().min(i64::MAX as usize) as i64;
    response.server_updated_at_epoch_millis =
        app_data_revision_millis(current, response.server_updated_at_epoch_millis);
    prepare_sync_payload(&mut response)
        .map_err(|_| "projected committed response exceeds its wire quota".into())
}

fn prepare_sync_payload(result: &mut SyncClientResult) -> Result<String, SyncClientResult> {
    let mut encoded = encode_result(result);
    for _ in 0..3 {
        result.sync_payload_usage_bytes = encoded.len().min(i64::MAX as usize) as i64;
        result.sync_payload_limit_bytes = MAX_RESPONSE_BODY_BYTES as i64;
        result.sync_payload_warning = sync_payload_warning(encoded.len()).to_string();
        let updated = encode_result(result);
        if updated.len() == encoded.len() {
            encoded = updated;
            break;
        }
        encoded = updated;
    }
    if encoded.len() > MAX_RESPONSE_BODY_BYTES {
        Err(sync_payload_quota_exceeded_result(encoded.len()))
    } else {
        Ok(encoded)
    }
}

fn app_data_quota_exceeded_result(
    current_bytes: i64,
    projected_bytes: i64,
    limit_bytes: i64,
) -> SyncClientResult {
    let projected_size = usize::try_from(projected_bytes.max(0)).unwrap_or(usize::MAX);
    let mut result = sync_payload_quota_exceeded_result(projected_size);
    result.message = "账户数据已超出安全上限，本次同步未写入；本地数据仍完整保留。请先导出数据并明确精简历史后重试。".to_string();
    result.mode = "app_data_quota_exceeded".to_string();
    result.app_data_current_bytes = current_bytes;
    result.app_data_projected_bytes = projected_bytes;
    result.app_data_limit_bytes = limit_bytes;
    result
}

fn valid_request_id(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed.len() <= 256 && !trimmed.chars().any(char::is_control)
}

fn valid_workspace_capability_component(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, Default)]
struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<HttpRequest> {
    let header_deadline = Instant::now() + Duration::from_secs(10);
    let mut buffer = Vec::<u8>::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let remaining = header_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request headers timed out",
            ));
        }
        stream.set_read_timeout(Some(remaining))?;
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before headers",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&buffer) {
            if index > MAX_HEADER_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "request headers too large",
                ));
            }
            break index;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    let content_length = parse_content_length(&buffer[..header_end], false)?.unwrap_or(0);
    if content_length > MAX_REQUEST_BODY_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "request body too large",
        ));
    }
    let body_start = header_end + 4;
    let body_deadline = Instant::now() + Duration::from_secs(60);
    while buffer.len().saturating_sub(body_start) < content_length {
        let remaining = body_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "request body timed out",
            ));
        }
        stream.set_read_timeout(Some(remaining.min(Duration::from_secs(30))))?;
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before the declared request body was received",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len().saturating_sub(body_start) > MAX_REQUEST_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request body too large",
            ));
        }
    }
    let body_bytes = &buffer[body_start..body_start + content_length];
    let body = String::from_utf8_lossy(body_bytes).to_string();
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn write_json_response(
    stream: &mut TcpStream,
    status: u16,
    result: &SyncClientResult,
) -> io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    let body = encode_result(result);
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        507 => "Insufficient Storage",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json; charset=utf-8\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        body.as_bytes().len(),
        body
    )?;
    stream.flush()?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

fn parse_base_url(server_url: &str) -> Result<ParsedBaseUrl, String> {
    let trimmed = server_url.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.chars().any(|value| {
            value.is_control() || value.is_whitespace() || matches!(value, '@' | '?' | '#' | '\\')
        })
    {
        return Err("Sync server URL contains unsupported characters.".to_string());
    }
    let lower = trimmed.to_ascii_lowercase();
    let (scheme, without_scheme) = if lower.starts_with("http://") {
        (SyncUrlScheme::Http, &trimmed[7..])
    } else if lower.starts_with("https://") {
        (SyncUrlScheme::Https, &trimmed[8..])
    } else {
        return Err("Sync server URL must start with http:// or https://".to_string());
    };
    let (host_port, path) = without_scheme
        .split_once('/')
        .map(|(host, path)| (host, format!("/{path}")))
        .unwrap_or((without_scheme, String::new()));
    if host_port.trim().is_empty() {
        return Err("Sync server host is empty.".to_string());
    }
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let closing = bracketed
            .find(']')
            .ok_or_else(|| "Sync server IPv6 host is invalid.".to_string())?;
        let host = &bracketed[..closing];
        host.parse::<Ipv6Addr>()
            .map_err(|_| "Sync server IPv6 host is invalid.".to_string())?;
        let remainder = &bracketed[closing + 1..];
        let port = if remainder.is_empty() {
            scheme.default_port()
        } else {
            remainder
                .strip_prefix(':')
                .ok_or_else(|| "Sync server authority is invalid.".to_string())?
                .parse::<u16>()
                .map_err(|_| "Sync server port is invalid.".to_string())?
        };
        (host.to_ascii_lowercase(), port)
    } else if host_port.matches(':').count() > 1 {
        return Err("IPv6 sync server hosts must use brackets.".to_string());
    } else if let Some((host, port)) = host_port.rsplit_once(':') {
        let port = port
            .parse::<u16>()
            .map_err(|_| "Sync server port is invalid.".to_string())?;
        (host.to_ascii_lowercase(), port)
    } else {
        (host_port.to_ascii_lowercase(), scheme.default_port())
    };
    if host.is_empty() || port == 0 {
        return Err("Sync server host is empty.".to_string());
    }
    Ok(ParsedBaseUrl {
        scheme,
        host,
        port,
        base_path: path,
    })
}

fn is_loopback_sync_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    normalized == "localhost"
        || normalized
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false)
}

fn join_paths(base_path: &str, endpoint: &str) -> String {
    let base = base_path.trim_end_matches('/');
    let endpoint = endpoint.trim_start_matches('/');
    if base.is_empty() {
        format!("/{endpoint}")
    } else {
        format!("{base}/{endpoint}")
    }
}

#[cfg(test)]
fn format_base_url(base: &ParsedBaseUrl) -> String {
    let base_path = base.base_path.trim_end_matches('/');
    let host = host_header(base);
    if base_path.is_empty() {
        format!("{}://{}", base.scheme.as_str(), host)
    } else {
        format!("{}://{}{}", base.scheme.as_str(), host, base_path)
    }
}

fn format_request_url(base: &ParsedBaseUrl, endpoint: &str) -> String {
    let path = join_paths(&base.base_path, endpoint);
    format!("{}://{}{}", base.scheme.as_str(), host_header(base), path)
}

fn host_header(base: &ParsedBaseUrl) -> String {
    let host = if base.host.contains(':') {
        format!("[{}]", base.host)
    } else {
        base.host.clone()
    };
    if base.port == base.scheme.default_port() {
        host
    } else {
        format!("{host}:{}", base.port)
    }
}

fn bearer_token(request: &HttpRequest) -> Option<&str> {
    header_value(&request.headers, "authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn parse_content_length(head_bytes: &[u8], is_response: bool) -> io::Result<Option<usize>> {
    let head = String::from_utf8_lossy(head_bytes);
    let mut content_length_value: Option<&str> = None;
    for line in head.lines().skip(1) {
        if line.trim().is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "malformed HTTP header"))?;
        let name = name.trim();
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Transfer-Encoding is not supported",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length_value.replace(value).is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "duplicate Content-Length headers are not allowed",
                ));
            }
        }
    }
    let Some(value) = content_length_value else {
        return Ok(None);
    };
    let content_length = value.parse::<usize>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            if is_response {
                "response content-length is invalid"
            } else {
                "request content-length is invalid"
            },
        )
    })?;
    let max_size = if is_response {
        MAX_RESPONSE_BODY_BYTES
    } else {
        MAX_REQUEST_BODY_BYTES
    };
    if content_length > max_size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            if is_response {
                "response body too large"
            } else {
                "request body too large"
            },
        ));
    }
    Ok(Some(content_length))
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn normalized_email(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn valid_email(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 320
        || value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return false;
    }
    let mut parts = value.split('@');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(local), Some(domain), None) if !local.is_empty() && !domain.is_empty()
    )
}

fn normalized_device_name(value: &str) -> String {
    let normalized = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(48)
        .collect::<String>();
    if normalized.is_empty() {
        "device".to_string()
    } else {
        normalized
    }
}

fn password_hash(salt: &str, password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    hex_bytes(&hasher.finalize())
}

#[cfg(not(target_os = "android"))]
fn hash_password_argon2(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| "Could not protect the account password.".to_string())
}

#[cfg(not(target_os = "android"))]
fn verify_stored_password(user: &StoredUser, password: &str) -> Result<bool, String> {
    match user.password_scheme.as_str() {
        "argon2id_phc" => {
            let parsed = PasswordHash::new(&user.password_hash)
                .map_err(|_| "Stored password record is invalid.".to_string())?;
            Ok(Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok())
        }
        "legacy_sha256" => Ok(constant_time_eq(
            password_hash(&user.password_salt, password).as_bytes(),
            user.password_hash.as_bytes(),
        )),
        _ => Err("Stored password scheme is not supported.".to_string()),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let compared_length = left.len().max(right.len());
    for index in 0..compared_length {
        let left_byte = left.get(index).copied().unwrap_or(0);
        let right_byte = right.get(index).copied().unwrap_or(0);
        difference |= (left_byte ^ right_byte) as usize;
    }
    difference == 0
}

fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    const BLOCK_BYTES: usize = 64;
    let mut key_block = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        let digest = Sha256::digest(key);
        key_block[..digest.len()].copy_from_slice(&digest);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner_pad = [0x36_u8; BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; BLOCK_BYTES];
    for index in 0..BLOCK_BYTES {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    hex_bytes(&outer.finalize())
}

pub fn token_identifier(token: &str) -> String {
    let digest = Sha256::digest(token.trim().as_bytes());
    hex_bytes(&digest[..16])
}

fn random_token(byte_count: usize) -> String {
    let mut bytes = vec![0u8; byte_count];
    OsRng.fill_bytes(&mut bytes);
    hex_bytes(&bytes)
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn max_revision_in_value(value: &Value) -> Option<i64> {
    match value {
        Value::Object(map) => map.iter().fold(None, |best, (key, value)| {
            let own = if is_revision_key(key) {
                value.as_i64().filter(|candidate| *candidate >= 0)
            } else {
                None
            };
            let nested = if is_revision_map_key(key) {
                max_nonnegative_number(value)
            } else {
                max_revision_in_value(value)
            };
            max_option(best, max_option(own, nested))
        }),
        Value::Array(values) => values.iter().fold(None, |best, value| {
            max_option(best, max_revision_in_value(value))
        }),
        _ => None,
    }
}

fn is_revision_key(key: &str) -> bool {
    key == "updatedAt"
        || key == "updatedAtEpochMillis"
        || key == "deletedAtEpochMillis"
        || key.ends_with("UpdatedAtEpochMillis")
        || key.ends_with("RevisionEpochMillis")
}

fn is_revision_map_key(key: &str) -> bool {
    matches!(
        key,
        "financeDayLedgerRevisions" | "financeMonthSnapshotRevisions"
    )
}

fn max_nonnegative_number(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().filter(|number| *number >= 0),
        Value::Object(values) => values.values().fold(None, |best, value| {
            max_option(best, max_nonnegative_number(value))
        }),
        Value::Array(values) => values.iter().fold(None, |best, value| {
            max_option(best, max_nonnegative_number(value))
        }),
        _ => None,
    }
}

fn is_blank_timer_slot_object(
    _timestamp_value: &Value,
    map: &serde_json::Map<String, Value>,
) -> bool {
    map.contains_key("accumulatedMillis")
        && map.contains_key("runningSinceEpochMillis")
        && map
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty()
        && map
            .get("note")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .is_empty()
        && map.get("categoryId").map(Value::is_null).unwrap_or(true)
        && map
            .get("accumulatedMillis")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            == 0
        && map
            .get("runningSinceEpochMillis")
            .map(Value::is_null)
            .unwrap_or(true)
}

fn has_meaningful_app_data(value: &Value) -> bool {
    let Some(root) = value.as_object() else {
        return false;
    };
    root.get("slots")
        .and_then(Value::as_array)
        .map(|slots| {
            slots.iter().any(|slot| {
                slot.as_object()
                    .map(|map| !is_blank_timer_slot_object(&Value::Null, map))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
        || root
            .get("sessions")
            .and_then(Value::as_array)
            .map(|values| !values.is_empty())
            .unwrap_or(false)
        || root
            .get("archivedTasks")
            .and_then(Value::as_array)
            .map(|values| !values.is_empty())
            .unwrap_or(false)
        || root
            .get("notes")
            .and_then(Value::as_array)
            .map(|notes| {
                notes.iter().any(|note| {
                    note.get("deletedAtEpochMillis")
                        .map(Value::is_null)
                        .unwrap_or(true)
                        && (note_has_encryption(note)
                            || !note
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .trim()
                                .is_empty()
                            || !note
                                .get("content")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .trim()
                                .is_empty())
                })
            })
            .unwrap_or(false)
        || root
            .get("financeProfile")
            .map(has_meaningful_finance_profile)
            .unwrap_or(false)
}

fn merge_app_data_values(account_value: &mut Value, local_value: &Value) {
    let (Some(account_root), Some(local_root)) =
        (account_value.as_object_mut(), local_value.as_object())
    else {
        if account_value.as_object().is_none() && local_value.as_object().is_some() {
            *account_value = local_value.clone();
        }
        return;
    };

    merge_tombstones(account_root, local_root);
    merge_array_by_identity(
        account_root,
        local_root,
        "syncConflictHistory",
        choose_newer_value,
    );
    merge_array_by_identity(
        account_root,
        local_root,
        "categories",
        choose_category_value,
    );
    merge_array_by_identity(account_root, local_root, "slots", choose_timer_slot_value);
    merge_slot_order(account_root, local_root);
    merge_array_by_identity_with_conflicts(
        account_root,
        local_root,
        "sessions",
        "session",
        choose_session_value,
        session_value_revision,
    );
    merge_array_by_identity_with_conflicts(
        account_root,
        local_root,
        "archivedTasks",
        "archivedTask",
        choose_archived_task_value,
        archived_task_value_revision,
    );
    merge_array_by_identity(
        account_root,
        local_root,
        "noteFolders",
        choose_note_folder_value,
    );
    merge_notes(account_root, local_root);

    merge_revisioned_field(
        account_root,
        local_root,
        "notePreferences",
        "notePreferencesUpdatedAtEpochMillis",
        true,
    );
    merge_finance_profile(account_root, local_root);
    merge_theme_preference(account_root, local_root);
}

fn merge_tombstones(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
) {
    let mut merged = account_root
        .get("tombstones")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut positions = HashMap::<String, usize>::new();
    for (index, value) in merged.iter().enumerate() {
        if let Some(key) = tombstone_identity_key(value) {
            positions.entry(key).or_insert(index);
        }
    }
    if let Some(local_values) = local_root.get("tombstones").and_then(Value::as_array) {
        for value in local_values {
            if let Some(key) = tombstone_identity_key(value) {
                if let Some(index) = positions.get(&key).copied() {
                    merged[index] = choose_newer_value(&merged[index], value);
                } else {
                    positions.insert(key, merged.len());
                    merged.push(value.clone());
                }
            }
        }
    }
    account_root.insert("tombstones".to_string(), Value::Array(merged));
}

fn tombstone_identity_key(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let entity_type = object.get("entityType")?.as_str()?.trim();
    let entity_id = object.get("entityId")?.as_str()?.trim();
    if entity_type.is_empty() || entity_id.is_empty() {
        None
    } else {
        Some(format!("{entity_type}\n{entity_id}"))
    }
}

fn merge_array_by_identity(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
    field_name: &str,
    choose_value: fn(&Value, &Value) -> Value,
) {
    let mut merged = account_root
        .get(field_name)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut positions = HashMap::new();
    for (index, value) in merged.iter().enumerate() {
        if let Some(key) = identity_key(value) {
            positions.entry(key).or_insert(index);
        }
    }

    if let Some(local_values) = local_root.get(field_name).and_then(Value::as_array) {
        for value in local_values {
            if let Some(key) = identity_key(value) {
                if let Some(index) = positions.get(&key).copied() {
                    merged[index] = choose_value(&merged[index], value);
                } else {
                    positions.insert(key, merged.len());
                    merged.push(value.clone());
                }
            } else if !merged.contains(value) {
                merged.push(value.clone());
            }
        }
    }

    account_root.insert(field_name.to_string(), Value::Array(merged));
}

fn merge_array_by_identity_with_conflicts(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
    field_name: &str,
    conflict_entity_type: &str,
    choose_value: fn(&Value, &Value) -> Value,
    revision_value: fn(&Value) -> i64,
) {
    let mut merged = account_root
        .get(field_name)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut positions = HashMap::new();
    for (index, value) in merged.iter().enumerate() {
        if let Some(key) = identity_key(value) {
            positions.entry(key).or_insert(index);
        }
    }
    let mut conflicts = Vec::new();
    if let Some(local_values) = local_root.get(field_name).and_then(Value::as_array) {
        for local in local_values {
            if let Some(key) = identity_key(local) {
                if let Some(index) = positions.get(&key).copied() {
                    let account = &merged[index];
                    let chosen = choose_value(account, local);
                    if account != local
                        && !history_entities_equivalent(conflict_entity_type, account, local)
                    {
                        let loser = if chosen == *local && chosen != *account {
                            account
                        } else {
                            local
                        };
                        if loser != &chosen {
                            let entity_id = entity_id_text(&chosen)
                                .or_else(|| entity_id_text(loser))
                                .unwrap_or_else(|| key.clone());
                            conflicts.push(sync_conflict_record_at(
                                conflict_entity_type,
                                &entity_id,
                                loser,
                                &chosen,
                                revision_value(loser),
                                revision_value(&chosen),
                            ));
                        }
                    }
                    merged[index] = chosen;
                } else {
                    positions.insert(key, merged.len());
                    merged.push(local.clone());
                }
            } else if !merged.contains(local) {
                merged.push(local.clone());
            }
        }
    }
    account_root.insert(field_name.to_string(), Value::Array(merged));
    for conflict in conflicts {
        append_sync_conflict(account_root, conflict);
    }
}

fn history_entities_equivalent(entity_type: &str, left: &Value, right: &Value) -> bool {
    fn canonical(entity_type: &str, value: &Value) -> Value {
        let Some(object) = value.as_object() else {
            return value.clone();
        };
        let mut object = object.clone();
        // Current-schema sanitation materializes an absent optional category as
        // null.  That is a representation change, not a recoverable conflict.
        if object.get("categoryId").is_some_and(Value::is_null) {
            object.remove("categoryId");
        }
        // Legacy archived tasks did not carry an explicit mutation revision.
        // Sanitation derives it from archivedAt; treat that exact derivation as
        // equivalent to absence so replaying an old client is idempotent.
        if entity_type == "archivedTask"
            && object.get("updatedAtEpochMillis").and_then(Value::as_i64)
                == object.get("archivedAtEpochMillis").and_then(Value::as_i64)
        {
            object.remove("updatedAtEpochMillis");
        }
        Value::Object(object)
    }

    canonical(entity_type, left) == canonical(entity_type, right)
}

fn entity_id_text(value: &Value) -> Option<String> {
    let id = value.get("id")?;
    id.as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| id.as_i64().map(|id| id.to_string()))
}

fn session_value_revision(value: &Value) -> i64 {
    if let Some(revision) = value.get("updatedAtEpochMillis").and_then(Value::as_i64) {
        return revision.max(0);
    }
    ["startedAtEpochMillis", "endedAtEpochMillis"]
        .into_iter()
        .filter_map(|field| value.get(field).and_then(Value::as_i64))
        .fold(0, i64::max)
        .max(0)
}

fn choose_session_value(account: &Value, local: &Value) -> Value {
    let account_revision = session_value_revision(account);
    let local_revision = session_value_revision(local);
    if local_revision > account_revision {
        local.clone()
    } else if account_revision > local_revision {
        account.clone()
    } else {
        choose_deterministic_value(account, local)
    }
}

fn archived_task_value_revision(value: &Value) -> i64 {
    if let Some(revision) = value.get("updatedAtEpochMillis").and_then(Value::as_i64) {
        return revision.max(0);
    }
    value
        .get("archivedAtEpochMillis")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0)
}

fn choose_archived_task_value(account: &Value, local: &Value) -> Value {
    let account_revision = archived_task_value_revision(account);
    let local_revision = archived_task_value_revision(local);
    if local_revision > account_revision {
        local.clone()
    } else if account_revision > local_revision {
        account.clone()
    } else {
        choose_deterministic_value(account, local)
    }
}

fn sync_conflict_record_at(
    entity_type: &str,
    entity_id: &str,
    loser: &Value,
    _winner: &Value,
    losing_revision: i64,
    winner_revision: i64,
) -> Value {
    let losing_revision = losing_revision.max(0);
    let captured_at = losing_revision.max(winner_revision).max(0);
    let identity_material = json!({
        "entityType": entity_type,
        "entityId": entity_id,
        "losingRevisionEpochMillis": losing_revision,
        "payload": loser,
    });
    let encoded = serde_json::to_vec(&identity_material).unwrap_or_default();
    let id = format!("sync-conflict-{}", hex_bytes(&Sha256::digest(&encoded)));
    json!({
        "id": id,
        "entityType": entity_type,
        "entityId": entity_id,
        "losingRevisionEpochMillis": losing_revision,
        "capturedAtEpochMillis": captured_at,
        "payload": loser,
    })
}

fn append_sync_conflict(root: &mut serde_json::Map<String, Value>, conflict: Value) {
    let history = root
        .entry("syncConflictHistory".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(history) = history.as_array_mut() else {
        *history = Value::Array(vec![conflict]);
        return;
    };
    let conflict_id = conflict.get("id").and_then(Value::as_str);
    if let Some(index) = conflict_id.and_then(|id| {
        history
            .iter()
            .position(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
    }) {
        history[index] = choose_newer_value(&history[index], &conflict);
    } else {
        history.push(conflict);
    }
}

fn merge_notes(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
) {
    let account_notes = account_root
        .get("notes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let local_notes = local_root
        .get("notes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut merged = Vec::<Value>::new();
    let mut positions = HashMap::<String, usize>::new();

    for note in account_notes.iter().chain(local_notes.iter()) {
        if let Some(key) = identity_key(note) {
            if let Some(index) = positions.get(&key).copied() {
                merged[index] = merge_note_value(&merged[index], note);
            } else {
                positions.insert(key, merged.len());
                merged.push(note.clone());
            }
        } else if !merged.contains(note) {
            merged.push(note.clone());
        }
    }

    account_root.insert("notes".to_string(), Value::Array(merged));
}

fn merge_note_value(account_note: &Value, local_note: &Value) -> Value {
    if note_protection_state_revision(account_note) != note_protection_state_revision(local_note)
        || note_has_encryption(account_note)
        || note_has_encryption(local_note)
    {
        return choose_note_protection_state_atomically(account_note, local_note);
    }
    let account_revision = note_current_revision(account_note);
    let local_revision = note_current_revision(local_note);
    let account_knowledge = note_has_knowledge_metadata(account_note);
    let local_knowledge = note_has_knowledge_metadata(local_note);
    let (mut merged, losing_current) = if account_knowledge != local_knowledge {
        if local_knowledge {
            (local_note.clone(), account_note)
        } else {
            (account_note.clone(), local_note)
        }
    } else if local_revision > account_revision {
        (local_note.clone(), account_note)
    } else if account_revision > local_revision {
        (account_note.clone(), local_note)
    } else {
        let chosen = choose_note_current_deterministically(account_note, local_note);
        if note_without_child_collections(&chosen) == note_without_child_collections(local_note) {
            (chosen, account_note)
        } else {
            (chosen, local_note)
        }
    };

    if account_knowledge != local_knowledge
        && note_current_revision(losing_current) > note_current_revision(&merged)
    {
        if let Some(deleted) = losing_current
            .get("deletedAtEpochMillis")
            .filter(|v| !v.is_null())
        {
            merged["deletedAtEpochMillis"] = deleted.clone();
            merged["updatedAtEpochMillis"] = losing_current["updatedAtEpochMillis"].clone();
        }
    }
    let attachments = merge_note_child_array(
        account_note,
        local_note,
        "attachments",
        choose_note_attachment_value,
    );
    let mut revisions = merge_note_child_array(
        account_note,
        local_note,
        "revisions",
        choose_note_revision_value,
    );
    let versions = normalize_note_version_values(merge_note_child_array(
        account_note,
        local_note,
        "versions",
        choose_note_revision_value,
    ));
    if note_recoverable_content(&merged) != note_recoverable_content(losing_current) {
        if let Some(mut snapshot) = note_current_as_revision(losing_current) {
            if note_has_knowledge_metadata(&merged) && !note_has_knowledge_metadata(losing_current)
            {
                snapshot["label"] = json!("旧版编辑器的修改");
            }
            if let Some(existing_index) =
                snapshot.get("id").and_then(Value::as_str).and_then(|id| {
                    revisions
                        .iter()
                        .position(|revision| revision.get("id").and_then(Value::as_str) == Some(id))
                })
            {
                revisions[existing_index] =
                    choose_note_revision_value(&revisions[existing_index], &snapshot);
            } else {
                revisions.push(snapshot);
            }
        }
    }
    let latest_version = versions
        .iter()
        .filter(|version| {
            version
                .get("deletedAtEpochMillis")
                .is_none_or(Value::is_null)
        })
        .max_by_key(|version| {
            (
                version.get("sequence").and_then(Value::as_i64).unwrap_or(0),
                version
                    .get("createdAtEpochMillis")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            )
        })
        .cloned();
    let should_apply_latest = latest_version.as_ref().is_some_and(|latest| {
        !note_current_matches_product_version(&merged, latest)
            && !(note_has_knowledge_metadata(&merged) && !note_has_knowledge_metadata(latest))
    });
    if let Some(object) = merged.as_object_mut() {
        object.insert("attachments".to_string(), Value::Array(attachments));
        object.insert("revisions".to_string(), Value::Array(revisions));
        if !versions.is_empty() {
            object.insert("versions".to_string(), Value::Array(versions));
            if let Some(latest) = latest_version {
                if let Some(version_id) = latest.get("id").and_then(Value::as_str) {
                    object.insert(
                        "latestVersionId".to_string(),
                        Value::String(version_id.to_string()),
                    );
                }
                if should_apply_latest {
                    for field in [
                        "title",
                        "content",
                        "kind",
                        "document",
                        "accentSeed",
                        "attachments",
                    ] {
                        if let Some(value) = latest.get(field) {
                            object.insert(field.to_string(), value.clone());
                        }
                    }
                }
            }
        }
    }
    merged
}

fn note_has_knowledge_metadata(note: &Value) -> bool {
    note["document"]["knowledge"].is_object()
        || note["document"]["blocks"]
            .as_array()
            .is_some_and(|blocks| blocks.iter().any(|b| b["knowledge"].is_object()))
}

fn note_current_matches_product_version(note: &Value, version: &Value) -> bool {
    let note_latest_id = note
        .get("latestVersionId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version_id = version
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if note_latest_id != version_id {
        return false;
    }
    for field in ["title", "content", "kind", "accentSeed", "attachments"] {
        if note.get(field) != version.get(field) {
            return false;
        }
    }
    let note_document = note.get("document");
    let note_has_structured_document = note_document
        .and_then(|document| document.get("blocks"))
        .and_then(Value::as_array)
        .is_some_and(|blocks| !blocks.is_empty())
        || note_document
            .and_then(|document| document.get("richTextEnabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
    !note_has_structured_document || note_document == version.get("document")
}

fn normalize_note_version_values(mut versions: Vec<Value>) -> Vec<Value> {
    versions.sort_by(|left, right| {
        let key = |value: &Value| {
            (
                value
                    .get("sequence")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0),
                value
                    .get("createdAtEpochMillis")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
                    .max(0),
                value
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        };
        key(left).cmp(&key(right))
    });
    let mut used_sequences = HashSet::<i64>::new();
    let mut next_sequence = versions
        .iter()
        .filter_map(|version| version.get("sequence").and_then(Value::as_i64))
        .max()
        .unwrap_or(0)
        .max(0)
        .saturating_add(1)
        .max(1);
    let latest_index = versions
        .iter()
        .enumerate()
        .filter(|(_, version)| {
            version
                .get("deletedAtEpochMillis")
                .is_none_or(Value::is_null)
        })
        .map(|(index, _)| index)
        .next_back();
    for (index, version) in versions.iter_mut().enumerate() {
        let Some(object) = version.as_object_mut() else {
            continue;
        };
        let mut sequence = object.get("sequence").and_then(Value::as_i64).unwrap_or(0);
        if sequence <= 0 || !used_sequences.insert(sequence) {
            sequence = next_sequence;
            next_sequence = next_sequence.saturating_add(1);
            used_sequences.insert(sequence);
            object.insert("sequence".to_string(), json!(sequence));
        }
        object.insert("isLatest".to_string(), json!(Some(index) == latest_index));
    }
    versions
}

fn note_has_encryption(note: &Value) -> bool {
    note.get("encryption").is_some_and(|value| !value.is_null())
}

fn note_protection_revision(note: &Value) -> i64 {
    note.get("encryption")
        .and_then(Value::as_object)
        .and_then(|envelope| envelope.get("protectionRevision"))
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0)
}

fn note_protection_state_revision(note: &Value) -> i64 {
    // This generation orders authenticated protection-state transitions.
    let explicit_revision = note
        .get("protectionStateRevision")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    if explicit_revision > 0 {
        return explicit_revision;
    }

    // Encrypted notes written before protectionStateRevision existed still
    // carry the authenticated envelope revision. Plaintext from that schema
    // has no authenticated transition generation and therefore remains at 0.
    note_protection_revision(note)
}

fn choose_note_protection_state_atomically(account_note: &Value, local_note: &Value) -> Value {
    let account_state_revision = note_protection_state_revision(account_note);
    let local_state_revision = note_protection_state_revision(local_note);
    if local_state_revision > account_state_revision {
        return local_note.clone();
    }
    if account_state_revision > local_state_revision {
        return account_note.clone();
    }

    let account_encrypted = note_has_encryption(account_note);
    let local_encrypted = note_has_encryption(local_note);
    if account_encrypted != local_encrypted {
        // A real enable/disable transition must advance the independent state
        // generation. At the same generation, fail closed instead of allowing
        // a timestamp-only plaintext candidate to expose protected content.
        return if local_encrypted {
            local_note.clone()
        } else {
            account_note.clone()
        };
    }

    let account_revision = note_current_revision(account_note);
    let local_revision = note_current_revision(local_note);
    if local_revision > account_revision {
        return local_note.clone();
    }
    if account_revision > local_revision {
        return account_note.clone();
    }

    // Ciphertext is opaque. Resolve a true tie using canonical JSON bytes only;
    // never use plaintext length/shape as a preference signal.
    let account_encoded = canonical_json_text(account_note);
    let local_encoded = canonical_json_text(local_note);
    if local_encoded > account_encoded {
        local_note.clone()
    } else {
        account_note.clone()
    }
}

fn note_recoverable_content(note: &Value) -> Value {
    let Some(object) = note.as_object() else {
        return Value::Null;
    };
    json!({
        "title": object.get("title").cloned().unwrap_or(Value::String(String::new())),
        "content": object.get("content").cloned().unwrap_or(Value::String(String::new())),
        "kind": object.get("kind").cloned().unwrap_or_else(|| json!("STICKY")),
        "document": object.get("document").cloned().unwrap_or_else(|| json!({})),
        "accentSeed": object.get("accentSeed").cloned().unwrap_or(Value::String(String::new())),
        "pinned": object.get("pinned").cloned().unwrap_or(Value::Bool(false)),
        "folderId": object.get("folderId").cloned().unwrap_or(Value::Null),
        "encryption": object.get("encryption").cloned().unwrap_or(Value::Null),
    })
}

fn note_current_as_revision(note: &Value) -> Option<Value> {
    if note_has_encryption(note) {
        return None;
    }
    let object = note.as_object()?;
    let recoverable = note_recoverable_content(note);
    let captured_at = direct_revision(object, &["updatedAtEpochMillis", "createdAtEpochMillis"]);
    let attachments = object
        .get("attachments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let attachment_ids = attachments
        .iter()
        .filter_map(|attachment| attachment.get("id").and_then(Value::as_str))
        .map(|id| Value::String(id.to_string()))
        .collect::<Vec<_>>();
    let identity_material = json!({
        "noteId": object.get("id").cloned().unwrap_or(Value::Null),
        "capturedAtEpochMillis": captured_at,
        "recoverable": recoverable,
        "attachmentIds": attachment_ids,
        "attachments": attachments,
    });
    let encoded = serde_json::to_vec(&identity_material).ok()?;
    let id = format!("merge-revision-{}", hex_bytes(&Sha256::digest(&encoded)));
    let recoverable = identity_material.get("recoverable")?.as_object()?;
    Some(json!({
        "id": id,
        "title": recoverable.get("title").cloned().unwrap_or(Value::String(String::new())),
        "content": recoverable.get("content").cloned().unwrap_or(Value::String(String::new())),
        "kind": recoverable.get("kind").cloned().unwrap_or_else(|| json!("STICKY")),
        "document": recoverable.get("document").cloned().unwrap_or_else(|| json!({})),
        "accentSeed": recoverable.get("accentSeed").cloned().unwrap_or(Value::String(String::new())),
        "pinned": recoverable.get("pinned").cloned().unwrap_or(Value::Bool(false)),
        "folderId": recoverable.get("folderId").cloned().unwrap_or(Value::Null),
        "attachmentIds": identity_material.get("attachmentIds").cloned().unwrap_or_else(|| json!([])),
        "attachments": identity_material.get("attachments").cloned().unwrap_or_else(|| json!([])),
        "capturedAtEpochMillis": captured_at,
        "updatedAtEpochMillis": captured_at,
    }))
}

fn note_current_revision(note: &Value) -> i64 {
    let Some(object) = note.as_object() else {
        return 0;
    };
    let updated_revision = object
        .get("updatedAtEpochMillis")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    let deletion_revision = object
        .get("deletedAtEpochMillis")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    let explicit_revision = updated_revision.max(deletion_revision);
    if explicit_revision > 0 {
        return explicit_revision;
    }
    direct_revision(object, &["createdAtEpochMillis"])
}

fn choose_note_current_deterministically(account_note: &Value, local_note: &Value) -> Value {
    let account_current = note_without_child_collections(account_note);
    let local_current = note_without_child_collections(local_note);
    let chosen = choose_deterministic_value(&account_current, &local_current);
    if chosen == local_current && chosen != account_current {
        local_note.clone()
    } else {
        account_note.clone()
    }
}

fn note_without_child_collections(note: &Value) -> Value {
    let mut current = note.clone();
    if let Some(object) = current.as_object_mut() {
        object.remove("attachments");
        object.remove("revisions");
        object.remove("versions");
        object.remove("latestVersionId");
    }
    current
}

fn merge_note_child_array(
    account_note: &Value,
    local_note: &Value,
    field_name: &str,
    choose_value: fn(&Value, &Value) -> Value,
) -> Vec<Value> {
    let account_values = account_note
        .get(field_name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let local_values = local_note
        .get(field_name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    let mut merged = Vec::<Value>::new();
    let mut positions = HashMap::<String, usize>::new();

    for value in account_values.chain(local_values) {
        if let Some(key) = identity_key(value) {
            if let Some(index) = positions.get(&key).copied() {
                merged[index] = choose_value(&merged[index], value);
            } else {
                positions.insert(key, merged.len());
                merged.push(value.clone());
            }
        } else if !merged.contains(value) {
            merged.push(value.clone());
        }
    }
    merged
}

fn choose_note_attachment_value(account_value: &Value, local_value: &Value) -> Value {
    let revision = |value: &Value| {
        let Some(object) = value.as_object() else {
            return 0;
        };
        object
            .get("updatedAtEpochMillis")
            .and_then(Value::as_i64)
            .filter(|revision| *revision > 0)
            .unwrap_or_else(|| direct_revision(object, &["createdAtEpochMillis"]))
            .max(0)
    };
    let account_revision = revision(account_value);
    let local_revision = revision(local_value);
    if local_revision > account_revision {
        local_value.clone()
    } else if account_revision > local_revision {
        account_value.clone()
    } else {
        choose_deterministic_value(account_value, local_value)
    }
}

fn choose_note_revision_value(account_value: &Value, local_value: &Value) -> Value {
    let revision = |value: &Value| {
        value
            .get("updatedAtEpochMillis")
            .and_then(Value::as_i64)
            .or_else(|| value.get("capturedAtEpochMillis").and_then(Value::as_i64))
            .unwrap_or(0)
            .max(0)
    };
    let account_revision = revision(account_value);
    let local_revision = revision(local_value);
    if local_revision > account_revision {
        local_value.clone()
    } else if account_revision > local_revision {
        account_value.clone()
    } else {
        choose_deterministic_value(account_value, local_value)
    }
}

#[cfg(test)]
fn choose_by_direct_revision(
    account_value: &Value,
    local_value: &Value,
    revision_fields: &[&str],
) -> Value {
    let account_revision = account_value
        .as_object()
        .map(|object| direct_revision(object, revision_fields))
        .unwrap_or(0);
    let local_revision = local_value
        .as_object()
        .map(|object| direct_revision(object, revision_fields))
        .unwrap_or(0);
    if local_revision > account_revision {
        local_value.clone()
    } else if account_revision > local_revision {
        account_value.clone()
    } else {
        choose_deterministic_value(account_value, local_value)
    }
}

fn direct_revision(object: &serde_json::Map<String, Value>, fields: &[&str]) -> i64 {
    fields
        .iter()
        .filter_map(|field| object.get(*field).and_then(Value::as_i64))
        .filter(|value| *value >= 0)
        .max()
        .unwrap_or(0)
}

fn merge_slot_order(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
) {
    let account_revision = revision_field(account_root, "slotOrderUpdatedAtEpochMillis");
    let local_revision = revision_field(local_root, "slotOrderUpdatedAtEpochMillis");
    if local_revision > account_revision {
        if let Some(local_order) = local_root.get("slotOrder") {
            account_root.insert("slotOrder".to_string(), local_order.clone());
        }
        account_root.insert(
            "slotOrderUpdatedAtEpochMillis".to_string(),
            json!(local_revision),
        );
        return;
    }
    if account_revision > local_revision {
        return;
    }

    let account_order = collect_slot_order_ids(account_root.get("slotOrder"));
    let local_order = collect_slot_order_ids(local_root.get("slotOrder"));
    let account_flat = slot_order_is_flat(account_root.get("slotOrder"));
    let local_flat = slot_order_is_flat(local_root.get("slotOrder"));
    let local_is_base = if local_flat != account_flat {
        local_flat
    } else {
        let account_value = Value::Array(account_order.iter().map(|id| json!(id)).collect());
        let local_value = Value::Array(local_order.iter().map(|id| json!(id)).collect());
        choose_deterministic_value(&account_value, &local_value) == local_value
            && account_value != local_value
    };
    let (base, supplement) = if local_is_base {
        (local_order, account_order)
    } else {
        (account_order, local_order)
    };
    let mut merged = Vec::new();
    let mut seen = HashSet::new();
    for slot_id in base.into_iter().chain(supplement) {
        if seen.insert(slot_id) {
            merged.push(json!(slot_id));
        }
    }
    if !merged.is_empty() {
        account_root.insert("slotOrder".to_string(), Value::Array(merged));
    }
    account_root.insert(
        "slotOrderUpdatedAtEpochMillis".to_string(),
        json!(account_revision.max(local_revision)),
    );
}

fn slot_order_is_flat(value: Option<&Value>) -> bool {
    value.and_then(Value::as_array).is_some_and(|values| {
        values.iter().all(|value| {
            value.as_i64().is_some()
                || value
                    .as_str()
                    .is_some_and(|raw| raw.trim().parse::<i64>().is_ok())
        })
    })
}

fn merge_revisioned_field(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
    field_name: &str,
    revision_name: &str,
    merge_legacy_objects: bool,
) {
    let account_revision = revision_field(account_root, revision_name);
    let local_revision = revision_field(local_root, revision_name);
    if let Some(local_value) = local_root.get(field_name) {
        if local_revision > account_revision {
            account_root.insert(field_name.to_string(), local_value.clone());
        } else if local_revision == account_revision {
            if account_revision == 0 && merge_legacy_objects {
                merge_object_field(account_root, field_name, local_value);
            } else if let Some(account_value) = account_root.get(field_name) {
                account_root.insert(
                    field_name.to_string(),
                    choose_deterministic_value(account_value, local_value),
                );
            } else {
                account_root.insert(field_name.to_string(), local_value.clone());
            }
        }
    }
    account_root.insert(
        revision_name.to_string(),
        json!(account_revision.max(local_revision)),
    );
}

fn merge_theme_preference(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
) {
    const REVISION_NAME: &str = "themeModeUpdatedAtEpochMillis";
    let account_revision = revision_field(account_root, REVISION_NAME);
    let local_revision = revision_field(local_root, REVISION_NAME);
    let preference = |root: &serde_json::Map<String, Value>| {
        json!({
            "themeMode": root
                .get("themeMode")
                .cloned()
                .unwrap_or_else(|| json!("SYSTEM")),
            "oledThemeEnabled": root
                .get("oledThemeEnabled")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    };
    let account_preference = preference(account_root);
    let local_preference = preference(local_root);
    let chosen = if local_revision > account_revision {
        local_preference
    } else if account_revision > local_revision {
        account_preference
    } else {
        choose_deterministic_value(&account_preference, &local_preference)
    };
    let chosen = chosen
        .as_object()
        .expect("theme preference is always an object");
    for field_name in ["themeMode", "oledThemeEnabled"] {
        account_root.insert(
            field_name.to_string(),
            chosen
                .get(field_name)
                .expect("theme preference field is always present")
                .clone(),
        );
    }
    account_root.insert(
        REVISION_NAME.to_string(),
        json!(account_revision.max(local_revision)),
    );
}

fn merge_finance_profile(
    account_root: &mut serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
) {
    let account_revision = revision_field(account_root, "financeProfileUpdatedAtEpochMillis");
    let local_revision = revision_field(local_root, "financeProfileUpdatedAtEpochMillis");
    let account_profile = account_root
        .get("financeProfile")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let local_profile = local_root
        .get("financeProfile")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let mut merged_profile = if local_revision > account_revision {
        local_profile.clone()
    } else if account_revision > local_revision {
        account_profile.clone()
    } else {
        choose_finance_profile_scalars_deterministically(&account_profile, &local_profile)
    };
    if !merged_profile.is_object() {
        merged_profile = json!({});
    }
    let account_scalars = finance_profile_without_history(&account_profile);
    let local_scalars = finance_profile_without_history(&local_profile);
    let chosen_scalars = finance_profile_without_history(&merged_profile);
    let scalar_conflict = if account_scalars != local_scalars {
        let (loser, losing_revision, winner_revision) =
            if chosen_scalars == local_scalars && chosen_scalars != account_scalars {
                (&account_scalars, account_revision, local_revision)
            } else {
                (&local_scalars, local_revision, account_revision)
            };
        (loser != &chosen_scalars).then(|| {
            sync_conflict_record_at(
                "financeProfile",
                "profile",
                loser,
                &chosen_scalars,
                losing_revision,
                winner_revision,
            )
        })
    } else {
        None
    };

    let (daily_ledgers, daily_revisions, daily_conflicts) = merge_finance_history_map(
        account_root,
        local_root,
        &account_profile,
        &local_profile,
        "dailyLedgers",
        "financeDayLedgerRevisions",
        "financeDayLedger",
        account_revision,
        local_revision,
    );
    let (monthly_snapshots, monthly_revisions, monthly_conflicts) = merge_finance_history_map(
        account_root,
        local_root,
        &account_profile,
        &local_profile,
        "monthlySnapshots",
        "financeMonthSnapshotRevisions",
        "financeMonthSnapshot",
        account_revision,
        local_revision,
    );
    if let Some(profile) = merged_profile.as_object_mut() {
        profile.insert("dailyLedgers".to_string(), Value::Object(daily_ledgers));
        profile.insert(
            "monthlySnapshots".to_string(),
            Value::Object(monthly_snapshots),
        );
    }
    account_root.insert("financeProfile".to_string(), merged_profile);
    account_root.insert(
        "financeDayLedgerRevisions".to_string(),
        Value::Object(daily_revisions),
    );
    account_root.insert(
        "financeMonthSnapshotRevisions".to_string(),
        Value::Object(monthly_revisions),
    );
    account_root.insert(
        "financeProfileUpdatedAtEpochMillis".to_string(),
        json!(account_revision.max(local_revision)),
    );
    for conflict in daily_conflicts
        .into_iter()
        .chain(monthly_conflicts.into_iter())
    {
        append_sync_conflict(account_root, conflict);
    }
    if let Some(conflict) = scalar_conflict {
        append_sync_conflict(account_root, conflict);
    }
}

fn choose_finance_profile_scalars_deterministically(
    account_profile: &Value,
    local_profile: &Value,
) -> Value {
    let account_scalars = finance_profile_without_history(account_profile);
    let local_scalars = finance_profile_without_history(local_profile);
    let chosen = choose_deterministic_value(&account_scalars, &local_scalars);
    if chosen == local_scalars && chosen != account_scalars {
        local_profile.clone()
    } else {
        account_profile.clone()
    }
}

fn finance_profile_without_history(profile: &Value) -> Value {
    let mut profile = profile.clone();
    if let Some(object) = profile.as_object_mut() {
        object.remove("dailyLedgers");
        object.remove("monthlySnapshots");
    }
    profile
}

#[allow(clippy::too_many_arguments)]
fn merge_finance_history_map(
    merged_root: &serde_json::Map<String, Value>,
    local_root: &serde_json::Map<String, Value>,
    account_profile: &Value,
    local_profile: &Value,
    map_name: &str,
    revision_map_name: &str,
    tombstone_entity_type: &str,
    account_root_revision: i64,
    local_root_revision: i64,
) -> (
    serde_json::Map<String, Value>,
    serde_json::Map<String, Value>,
    Vec<Value>,
) {
    let account_values = account_profile
        .get(map_name)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let local_values = local_profile
        .get(map_name)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let account_revisions = merged_root
        .get(revision_map_name)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let local_revisions = local_root
        .get(revision_map_name)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut keys = account_values
        .keys()
        .chain(local_values.keys())
        .chain(account_revisions.keys())
        .chain(local_revisions.keys())
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    keys.sort();
    let mut merged_values = serde_json::Map::new();
    let mut merged_revisions = serde_json::Map::new();
    let mut conflicts = Vec::new();
    for key in keys {
        let account_value = account_values.get(&key);
        let local_value = local_values.get(&key);
        let account_revision = finance_entity_revision(
            &account_revisions,
            &key,
            account_value.is_some(),
            account_root_revision,
        );
        let local_revision = finance_entity_revision(
            &local_revisions,
            &key,
            local_value.is_some(),
            local_root_revision,
        );
        let chosen = match (account_value, local_value) {
            (Some(account_value), Some(local_value)) => {
                let (winner_side, loser, losing_revision, winner_revision) =
                    if local_revision > account_revision {
                        (local_value, account_value, account_revision, local_revision)
                    } else if account_revision > local_revision {
                        (account_value, local_value, local_revision, account_revision)
                    } else {
                        let deterministic = choose_deterministic_value(account_value, local_value);
                        if deterministic == *local_value && deterministic != *account_value {
                            (local_value, account_value, account_revision, local_revision)
                        } else {
                            (account_value, local_value, local_revision, account_revision)
                        }
                    };
                let winner = merge_finance_container_values(winner_side, loser, map_name);
                if finance_container_has_conflict(winner_side, loser, map_name) {
                    conflicts.push(sync_conflict_record_at(
                        tombstone_entity_type,
                        &key,
                        loser,
                        &winner,
                        losing_revision,
                        winner_revision,
                    ));
                }
                Some(winner)
            }
            (Some(account_value), None) => Some(account_value.clone()),
            (None, Some(local_value)) => Some(local_value.clone()),
            (None, None) => None,
        };
        let merged_revision = account_revision.max(local_revision);
        let tombstone_revision =
            latest_entity_tombstone_revision(merged_root, tombstone_entity_type, &key);
        if let Some(chosen) = chosen {
            if tombstone_revision == 0 || merged_revision > tombstone_revision {
                merged_values.insert(key.clone(), chosen);
            } else {
                let tombstone = json!({
                    "deletedAtEpochMillis": tombstone_revision,
                });
                conflicts.push(sync_conflict_record_at(
                    tombstone_entity_type,
                    &key,
                    &chosen,
                    &tombstone,
                    merged_revision,
                    tombstone_revision,
                ));
            }
        }
        if merged_revision > 0 {
            merged_revisions.insert(key, json!(merged_revision));
        }
    }
    (merged_values, merged_revisions, conflicts)
}

fn merge_finance_container_values(primary: &Value, secondary: &Value, map_name: &str) -> Value {
    let mut merged = primary.clone();
    let Some(merged_object) = merged.as_object_mut() else {
        return choose_deterministic_value(primary, secondary);
    };
    let (primary_object, secondary_object) = match (primary.as_object(), secondary.as_object()) {
        (Some(primary_object), Some(secondary_object)) => (primary_object, secondary_object),
        _ => return choose_deterministic_value(primary, secondary),
    };
    for &field_name in finance_container_entry_fields(map_name) {
        let primary_entries = primary_object
            .get(field_name)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let secondary_entries = secondary_object
            .get(field_name)
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        merged_object.insert(
            field_name.to_string(),
            Value::Array(merge_finance_entry_multisets(
                primary_entries,
                secondary_entries,
            )),
        );
    }
    merged
}

fn finance_container_entry_fields(map_name: &str) -> &'static [&'static str] {
    match map_name {
        "dailyLedgers" => &["incomes", "expenses"],
        "monthlySnapshots" => &["assets", "liabilities"],
        _ => &[],
    }
}

/// Merges finance rows without requiring new clients on both sides. Stable IDs
/// are authoritative when present. Legacy rows without IDs use exact-JSON
/// multiset union, retaining the maximum multiplicity seen on either branch.
fn merge_finance_entry_multisets(primary: &[Value], secondary: &[Value]) -> Vec<Value> {
    let mut merged = primary
        .iter()
        .map(normalize_finance_entry_revision_fields)
        .collect::<Vec<_>>();
    let mut id_positions = HashMap::<String, usize>::new();
    for (index, entry) in merged.iter().enumerate() {
        if let Some(id) = finance_entry_id(entry) {
            id_positions.entry(id).or_insert(index);
        }
    }

    let primary_legacy_counts = finance_legacy_entry_counts(primary);
    let secondary_legacy_counts = finance_legacy_entry_counts(secondary);
    let mut merged_legacy_counts = primary_legacy_counts.clone();

    for entry in secondary {
        if let Some(id) = finance_entry_id(entry) {
            if let Some(index) = id_positions.get(&id).copied() {
                merged[index] = merge_finance_entry_values(&merged[index], entry);
            } else {
                id_positions.insert(id, merged.len());
                merged.push(normalize_finance_entry_revision_fields(entry));
            }
            continue;
        }

        let encoded = canonical_json_text(entry);
        let required_count = primary_legacy_counts
            .get(&encoded)
            .copied()
            .unwrap_or(0)
            .max(secondary_legacy_counts.get(&encoded).copied().unwrap_or(0));
        let current_count = merged_legacy_counts.get(&encoded).copied().unwrap_or(0);
        if current_count < required_count {
            merged.push(normalize_finance_entry_revision_fields(entry));
            merged_legacy_counts.insert(encoded, current_count + 1);
        }
    }
    merged
}

/// Resolves two versions of the same finance row independently of their
/// enclosing day/month revision. This prevents an older device from reviving a
/// row after another device has already deleted it.
fn merge_finance_entry_values(primary: &Value, secondary: &Value) -> Value {
    let primary = normalize_finance_entry_revision_fields(primary);
    let secondary = normalize_finance_entry_revision_fields(secondary);
    let primary_updated = finance_entry_millis(&primary, "updatedAtEpochMillis");
    let secondary_updated = finance_entry_millis(&secondary, "updatedAtEpochMillis");
    let primary_deleted = finance_entry_millis(&primary, "deletedAtEpochMillis");
    let secondary_deleted = finance_entry_millis(&secondary, "deletedAtEpochMillis");
    let primary_revision = primary_updated.max(primary_deleted);
    let secondary_revision = secondary_updated.max(secondary_deleted);

    if primary_revision > secondary_revision {
        return primary;
    }
    if secondary_revision > primary_revision {
        return secondary;
    }

    // At the same logical time deletion is monotonic. An active row must have
    // a strictly later update before it can intentionally restore a tombstone.
    let primary_is_deleted = primary_deleted > 0;
    let secondary_is_deleted = secondary_deleted > 0;
    if primary_is_deleted != secondary_is_deleted {
        return if primary_is_deleted {
            primary
        } else {
            secondary
        };
    }

    // Rows written before row-level revisions existed retain the existing
    // enclosing-container winner semantics.
    if primary_revision == 0 {
        return primary;
    }
    choose_deterministic_value(&primary, &secondary)
}

fn normalize_finance_entry_revision_fields(entry: &Value) -> Value {
    let mut normalized = entry.clone();
    let Some(object) = normalized.as_object_mut() else {
        return normalized;
    };
    object.insert(
        "updatedAtEpochMillis".to_string(),
        json!(finance_entry_millis(entry, "updatedAtEpochMillis")),
    );
    object.insert(
        "deletedAtEpochMillis".to_string(),
        json!(finance_entry_millis(entry, "deletedAtEpochMillis")),
    );
    if object.get("id").is_some_and(Value::is_null) {
        object.insert("id".to_string(), Value::String(String::new()));
    } else if let Some(id) = finance_entry_id(entry) {
        object.insert("id".to_string(), Value::String(id));
    }
    normalized
}

fn finance_entry_millis(entry: &Value, field_name: &str) -> i64 {
    let Some(value) = entry.get(field_name) else {
        return 0;
    };
    value
        .as_i64()
        .or_else(|| {
            value
                .as_u64()
                .map(|value| value.min(i64::MAX as u64) as i64)
        })
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        .unwrap_or(0)
        .max(0)
}

fn finance_legacy_entry_counts(entries: &[Value]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for entry in entries {
        if finance_entry_id(entry).is_none() {
            let count = counts.entry(canonical_json_text(entry)).or_insert(0usize);
            *count = count.saturating_add(1);
        }
    }
    counts
}

fn finance_entry_id(entry: &Value) -> Option<String> {
    let id = entry.get("id")?;
    id.as_str()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| id.as_i64().map(|id| id.to_string()))
}

fn canonical_json_text(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn finance_container_has_conflict(primary: &Value, secondary: &Value, map_name: &str) -> bool {
    let (Some(primary_object), Some(secondary_object)) =
        (primary.as_object(), secondary.as_object())
    else {
        return primary != secondary;
    };
    let primary_note = primary_object
        .get("note")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let secondary_note = secondary_object
        .get("note")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if primary_note != secondary_note {
        return true;
    }

    finance_container_entry_fields(map_name)
        .iter()
        .any(|field_name| {
            let primary_entries = primary_object
                .get(*field_name)
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let secondary_entries = secondary_object
                .get(*field_name)
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let primary_by_id = primary_entries
                .iter()
                .filter_map(|entry| finance_entry_id(entry).map(|id| (id, entry)))
                .collect::<HashMap<_, _>>();
            secondary_entries.iter().any(|entry| {
                finance_entry_id(entry)
                    .and_then(|id| primary_by_id.get(&id).copied())
                    .is_some_and(|primary_entry| primary_entry != entry)
            })
        })
}

fn finance_entity_revision(
    revisions: &serde_json::Map<String, Value>,
    key: &str,
    value_exists: bool,
    root_revision: i64,
) -> i64 {
    let explicit = revisions
        .get(key)
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    if explicit > 0 || !value_exists {
        explicit
    } else {
        root_revision
    }
}

fn latest_entity_tombstone_revision(
    root: &serde_json::Map<String, Value>,
    entity_type: &str,
    entity_id: &str,
) -> i64 {
    root.get("tombstones")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .filter(|tombstone| {
            tombstone.get("entityType").and_then(Value::as_str) == Some(entity_type)
                && tombstone.get("entityId").and_then(Value::as_str) == Some(entity_id)
        })
        .filter_map(|tombstone| {
            tombstone
                .get("deletedAtEpochMillis")
                .and_then(Value::as_i64)
        })
        .max()
        .unwrap_or(0)
        .max(0)
}

fn revision_field(root: &serde_json::Map<String, Value>, name: &str) -> i64 {
    root.get(name).and_then(Value::as_i64).unwrap_or(0).max(0)
}

fn collect_slot_order_ids(value: Option<&Value>) -> Vec<i64> {
    let mut slot_ids = Vec::new();
    if let Some(value) = value {
        collect_slot_order_ids_from_value(value, &mut slot_ids);
    }
    slot_ids
}

fn collect_slot_order_ids_from_value(value: &Value, slot_ids: &mut Vec<i64>) {
    match value {
        Value::Number(number) => {
            if let Some(slot_id) = number.as_i64() {
                slot_ids.push(slot_id);
            }
        }
        Value::String(raw) => {
            if let Ok(slot_id) = raw.trim().parse::<i64>() {
                slot_ids.push(slot_id);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_slot_order_ids_from_value(child, slot_ids);
            }
        }
        _ => {}
    }
}

fn merge_object_field(
    account_root: &mut serde_json::Map<String, Value>,
    field_name: &str,
    local_value: &Value,
) {
    match account_root.get_mut(field_name) {
        Some(account_value) => merge_json_value(account_value, local_value),
        None => {
            account_root.insert(field_name.to_string(), local_value.clone());
        }
    }
}

fn merge_json_value(account_value: &mut Value, local_value: &Value) {
    match (account_value, local_value) {
        (Value::Object(account_map), Value::Object(local_map)) => {
            for (key, local_child) in local_map {
                match account_map.get_mut(key) {
                    Some(account_child) => merge_json_value(account_child, local_child),
                    None => {
                        account_map.insert(key.clone(), local_child.clone());
                    }
                }
            }
        }
        (Value::Array(account_values), Value::Array(local_values)) => {
            let mut positions = HashMap::new();
            for (index, value) in account_values.iter().enumerate() {
                if let Some(key) = identity_key(value) {
                    positions.entry(key).or_insert(index);
                }
            }
            for value in local_values {
                if let Some(key) = identity_key(value) {
                    if let Some(index) = positions.get(&key).copied() {
                        account_values[index] = choose_newer_value(&account_values[index], value);
                    } else {
                        positions.insert(key, account_values.len());
                        account_values.push(value.clone());
                    }
                } else if !account_values.contains(value) {
                    account_values.push(value.clone());
                }
            }
        }
        (account_leaf, local_leaf) => {
            if is_empty_leaf(account_leaf) && !is_empty_leaf(local_leaf) {
                *account_leaf = local_leaf.clone();
            }
        }
    }
}

fn choose_timer_slot_value(account_value: &Value, local_value: &Value) -> Value {
    let (Some(account), Some(local)) = (account_value.as_object(), local_value.as_object()) else {
        return choose_newer_value(account_value, local_value);
    };
    let account_legacy_revision = timer_slot_updated_at(account_value);
    let local_legacy_revision = timer_slot_updated_at(local_value);
    let mut merged = choose_deterministic_value(account_value, local_value);
    let Some(merged) = merged.as_object_mut() else {
        return choose_newer_value(account_value, local_value);
    };

    for (fields, revision_name) in [
        (&["title"][..], "titleUpdatedAtEpochMillis"),
        (&["categoryId"][..], "categoryUpdatedAtEpochMillis"),
        (&["note"][..], "noteUpdatedAtEpochMillis"),
        (
            &["accumulatedMillis"][..],
            "accumulatedUpdatedAtEpochMillis",
        ),
        (
            &["runningSinceEpochMillis", "activeRunId"][..],
            "runningUpdatedAtEpochMillis",
        ),
        (
            &[
                "microBreakPhase",
                "microBreakCycleIndex",
                "microBreakPhaseProgressMillis",
            ][..],
            "microBreakUpdatedAtEpochMillis",
        ),
    ] {
        let account_revision = slot_field_revision(account, revision_name, account_legacy_revision);
        let local_revision = slot_field_revision(local, revision_name, local_legacy_revision);
        let chosen = if local_revision > account_revision {
            local
        } else if account_revision > local_revision {
            account
        } else {
            let account_meaningful = slot_group_is_meaningful(account, fields);
            let local_meaningful = slot_group_is_meaningful(local, fields);
            match (account_meaningful, local_meaningful) {
                (true, false) => account,
                (false, true) => local,
                _ => {
                    let account_group = slot_group_value(account, fields);
                    let local_group = slot_group_value(local, fields);
                    if choose_deterministic_value(&account_group, &local_group) == local_group
                        && account_group != local_group
                    {
                        local
                    } else {
                        account
                    }
                }
            }
        };
        copy_slot_group(merged, chosen, fields);
        merged.insert(
            revision_name.to_string(),
            json!(account_revision.max(local_revision)),
        );
    }

    let merged_revision = [
        "titleUpdatedAtEpochMillis",
        "categoryUpdatedAtEpochMillis",
        "noteUpdatedAtEpochMillis",
        "accumulatedUpdatedAtEpochMillis",
        "runningUpdatedAtEpochMillis",
        "microBreakUpdatedAtEpochMillis",
    ]
    .into_iter()
    .filter_map(|field| merged.get(field).and_then(Value::as_i64))
    .fold(account_legacy_revision.max(local_legacy_revision), i64::max);
    merged.insert("updatedAt".to_string(), json!(merged_revision));
    Value::Object(merged.clone())
}

fn timer_slot_updated_at(value: &Value) -> i64 {
    [
        "updatedAt",
        "titleUpdatedAtEpochMillis",
        "categoryUpdatedAtEpochMillis",
        "noteUpdatedAtEpochMillis",
        "accumulatedUpdatedAtEpochMillis",
        "runningUpdatedAtEpochMillis",
        "microBreakUpdatedAtEpochMillis",
    ]
    .into_iter()
    .filter_map(|field| value.get(field).and_then(Value::as_i64))
    .fold(0, i64::max)
    .max(0)
}

fn slot_field_revision(
    slot: &serde_json::Map<String, Value>,
    revision_name: &str,
    legacy_revision: i64,
) -> i64 {
    slot.get(revision_name)
        .and_then(Value::as_i64)
        .filter(|revision| *revision > 0)
        .unwrap_or(legacy_revision)
        .max(0)
}

fn slot_group_value(slot: &serde_json::Map<String, Value>, fields: &[&str]) -> Value {
    Value::Object(
        fields
            .iter()
            .filter_map(|field| {
                slot.get(*field)
                    .cloned()
                    .map(|value| ((*field).to_string(), value))
            })
            .collect(),
    )
}

fn slot_group_is_meaningful(slot: &serde_json::Map<String, Value>, fields: &[&str]) -> bool {
    fields.iter().any(|field| {
        slot.get(*field).is_some_and(|value| match value {
            Value::Null => false,
            Value::Bool(value) => *value,
            Value::Number(value) => value.as_i64().unwrap_or(0) != 0,
            Value::String(value) if *field == "microBreakPhase" => {
                !value.trim().is_empty() && !value.eq_ignore_ascii_case("FOCUS")
            }
            Value::String(value) => !value.trim().is_empty(),
            Value::Array(values) => !values.is_empty(),
            Value::Object(values) => !values.is_empty(),
        })
    })
}

fn copy_slot_group(
    destination: &mut serde_json::Map<String, Value>,
    source: &serde_json::Map<String, Value>,
    fields: &[&str],
) {
    for field in fields {
        if let Some(value) = source.get(*field) {
            destination.insert((*field).to_string(), value.clone());
        } else {
            destination.remove(*field);
        }
    }
}

fn choose_category_value(account_value: &Value, local_value: &Value) -> Value {
    choose_by_primary_revision_with_legacy_fallback(
        account_value,
        local_value,
        "updatedAtEpochMillis",
        &["createdAtEpochMillis"],
    )
}

fn choose_note_folder_value(account_value: &Value, local_value: &Value) -> Value {
    choose_by_primary_revision_with_legacy_fallback(
        account_value,
        local_value,
        "updatedAtEpochMillis",
        &["createdAtEpochMillis"],
    )
}

fn choose_by_primary_revision_with_legacy_fallback(
    account_value: &Value,
    local_value: &Value,
    primary: &str,
    legacy_fallback: &[&str],
) -> Value {
    let revision = |value: &Value| {
        let Some(object) = value.as_object() else {
            return 0;
        };
        if let Some(revision) = object.get(primary).and_then(Value::as_i64) {
            revision.max(0)
        } else {
            direct_revision(object, legacy_fallback)
        }
    };
    let account_revision = revision(account_value);
    let local_revision = revision(local_value);
    if local_revision > account_revision {
        local_value.clone()
    } else if account_revision > local_revision {
        account_value.clone()
    } else {
        choose_deterministic_value(account_value, local_value)
    }
}

fn choose_newer_value(account_value: &Value, local_value: &Value) -> Value {
    let account_revision = max_revision_in_value(account_value).unwrap_or(0);
    let local_revision = max_revision_in_value(local_value).unwrap_or(0);
    if local_revision > account_revision {
        local_value.clone()
    } else if local_revision == account_revision {
        choose_deterministic_value(account_value, local_value)
    } else {
        account_value.clone()
    }
}

fn choose_deterministic_value(left: &Value, right: &Value) -> Value {
    let left_weight = value_text_weight(left);
    let right_weight = value_text_weight(right);
    if right_weight > left_weight {
        return right.clone();
    }
    if left_weight > right_weight {
        return left.clone();
    }
    let left_encoded = serde_json::to_string(left).unwrap_or_default();
    let right_encoded = serde_json::to_string(right).unwrap_or_default();
    if right_encoded > left_encoded {
        right.clone()
    } else {
        left.clone()
    }
}

fn value_text_weight(value: &Value) -> usize {
    match value {
        Value::String(text) => text.trim().len(),
        Value::Array(values) => values.iter().map(value_text_weight).sum(),
        Value::Object(map) => map.values().map(value_text_weight).sum(),
        _ => 0,
    }
}

fn identity_key(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    if let Some(id) = object.get("id") {
        if let Some(text) = id.as_str().map(str::trim).filter(|text| !text.is_empty()) {
            return Some(format!("id:{text}"));
        }
        if let Some(number) = id.as_i64() {
            return Some(format!("id:{number}"));
        }
    }
    None
}

fn is_empty_leaf(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(value) => !*value,
        Value::Number(value) => value.as_i64().map(|number| number == 0).unwrap_or(false),
        Value::String(value) => value.trim().is_empty(),
        Value::Array(values) => values.is_empty(),
        Value::Object(map) => map.is_empty(),
    }
}

fn has_meaningful_finance_profile(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    [
        "activeIncomeMonthly",
        "assetIncomeMonthly",
        "livingExpenseMonthly",
        "liabilityPaymentMonthly",
        "cashReserve",
        "productiveAssetValue",
        "liabilityBalance",
    ]
    .iter()
    .any(|key| map.get(*key).and_then(Value::as_i64).unwrap_or(0) != 0)
        || map
            .get("dailyLedgers")
            .and_then(Value::as_object)
            .map(|values| !values.is_empty())
            .unwrap_or(false)
        || map
            .get("monthlySnapshots")
            .and_then(Value::as_object)
            .map(|values| !values.is_empty())
            .unwrap_or(false)
}

fn max_option(left: Option<i64>, right: Option<i64>) -> Option<i64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn error_result(message: &str) -> SyncClientResult {
    SyncClientResult {
        ok: false,
        message: message.to_string(),
        ..SyncClientResult::default()
    }
}

#[cfg(not(target_os = "android"))]
fn busy_server_result() -> SyncClientResult {
    SyncClientResult {
        ok: false,
        message: "Sync server is busy. Try again shortly.".to_string(),
        product_id: product_identity::PRODUCT.internal_id.to_string(),
        service_role: "sync_server".to_string(),
        sync_protocol_version: SYNC_PROTOCOL_VERSION,
        mode: "busy".to_string(),
        server_process_name: current_server_process_name(),
        server_build_id: SYNC_SERVER_BUILD_ID.to_string(),
        server_git_commit: product_identity::BUILD_GIT_COMMIT.to_string(),
        server_source_snapshot_sha256: product_identity::BUILD_SOURCE_SNAPSHOT_SHA256.to_string(),
        server_process_id: std::process::id(),
        ..SyncClientResult::default()
    }
}

fn upgrade_required_result(version: &str, stored_account: bool) -> SyncClientResult {
    let source = if stored_account {
        "Stored account data"
    } else {
        "Submitted app data"
    };
    SyncClientResult {
        ok: false,
        message: format!(
            "{source} uses schemaVersion {version}, but this sync server supports through schemaVersion {}. Upgrade the sync server before syncing.",
            app_data::APP_DATA_SCHEMA_VERSION
        ),
        mode: "upgrade_required".to_string(),
        ..SyncClientResult::default()
    }
}

fn encode_result(result: &SyncClientResult) -> String {
    serde_json::to_string(result).unwrap_or_else(|_| {
        "{\"ok\":false,\"message\":\"Could not encode sync result.\"}".to_string()
    })
}

#[cfg(test)]
#[path = "sync_core_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "sync_http_deadline_tests.rs"]
mod http_deadline_tests;

#[cfg(all(test, not(target_os = "android")))]
#[path = "sync_cancel_tests.rs"]
mod cancellation_tests;
