// v0.0.1 - Acquire actual supervisor mutex ownership and release it on orderly shutdown.
// v2.22.58 - Recover public tunnel health checks when the local DNS route fails.
// v2.22.57 - Budget startup for managed archive maintenance as well as the live database.
// v2.22.56 - Retain pending tunnels and persist bounded provisioning retries.
// v2.22.55 - Scope startup to the signed-in user and recover the supervisor independently.
// v2.22.45 - Provision public tunnels through the configured proxy and honor provider cooldown.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../quick_tunnel_provision.rs"]
mod quick_tunnel_provision;
#[path = "../sync_tunnel_retry.rs"]
mod tunnel_retry;

use gridtimer_native::product_identity;
use gridtimer_native::sync_core::SYNC_SERVER_BUILD_ID;
use gridtimer_native::sync_rendezvous::{
    ensure_local_rendezvous_identity, publish_url, RendezvousPayload,
};
use sha2::{Digest, Sha256};
use std::env;
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SYNC_BIND_ADDR: &str = "0.0.0.0:8917";
const LOCAL_SYNC_ADDR: &str = "127.0.0.1:8917";
const LOCAL_SYNC_PORT: u16 = 8917;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;
const SUPERVISOR_CHECK_INTERVAL: Duration = Duration::from_secs(15);
const TUNNEL_RETRY_DELAY: Duration = Duration::from_secs(20);
const SERVER_RETRY_DELAY: Duration = Duration::from_secs(10);
const SERVER_STARTUP_BASE_TIMEOUT: Duration = Duration::from_secs(45);
const SERVER_STARTUP_MAX_TIMEOUT: Duration = Duration::from_secs(6 * 60 * 60);
const SERVER_STARTUP_SECONDS_PER_MIB: u64 = 10;
const LOCAL_SERVER_RESTART_FAILURE_THRESHOLD: u32 = 3;
const MAX_CONSECUTIVE_HEALTH_FAILURES: u32 = 3;
const VERIFIED_QUICK_TUNNEL_FAILURE_THRESHOLD: u32 = 4;
const VERIFIED_QUICK_TUNNEL_FAILURE_WINDOW: Duration = Duration::from_secs(60);
const PENDING_QUICK_TUNNEL_FAILURE_WINDOW: Duration = Duration::from_secs(5 * 60);
const LAUNCHER_LOG_MAX_BYTES: u64 = 1_048_576;
const LAUNCHER_LOG_BACKUPS: usize = 3;
const SERVER_LOG_MAX_BYTES: u64 = 4_194_304;
const SERVER_LOG_BACKUPS: usize = 2;
const TUNNEL_LOG_MAX_BYTES: u64 = 4_194_304;
const HEALTH_RESPONSE_MAX_BYTES: u64 = 16_384;
const QUICK_TUNNEL_METRICS_BIND_ADDR: &str = "127.0.0.1:0";
const DOH_ENDPOINT: &str = "https://1.1.1.1/dns-query";
const DNS_WIRE_MIN_BYTES: usize = 12;
const DNS_WIRE_MAX_BYTES: usize = 4_096;
const DOH_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(2);
const DNS_MAX_RECORDS: usize = 128;
const CLOUDFLARE_TUNNEL_EDGE_HOSTS: [&str; 2] =
    ["region1.v2.argotunnel.com", "region2.v2.argotunnel.com"];
const CLOUDFLARE_TUNNEL_EDGE_PORT: u16 = 7_844;
const MIN_STATIC_EDGE_IPS: usize = 2;
const MAX_STATIC_EDGE_IPS: usize = 16;
const DISABLE_AUTOSTART_ENV: &str = "GRID_TIMER_LAUNCHER_DISABLE_AUTOSTART";
const STABLE_DESKTOP_ENTRY_DIRECTORY: &str = "desktop_entry";
const STABLE_DESKTOP_ENTRY_FILE_NAME: &str = "TenRate_Desktop_Launcher.exe";
const STABLE_SYNC_ENTRY_ARGUMENT: &str = "--sync-supervisor";
const STABLE_PUBLIC_URL_FILE_NAME: &str = "sync_stable_public_url.txt";
const CLOUDFLARED_TOKEN_FILE_NAME: &str = "cloudflared_tunnel_token.txt";
const RENDEZVOUS_GENERATION_FILE_NAME: &str = "rendezvous_generation.txt";
const RENDEZVOUS_GENERATION_MAX_BYTES: u64 = 20;
const RENDEZVOUS_RENEW_INTERVAL: Duration = Duration::from_secs(4 * 60 * 60);
const RENDEZVOUS_ANNOUNCEMENT_LIFETIME: Duration = Duration::from_secs(6 * 60 * 60);
const RENDEZVOUS_RETRY_INITIAL: Duration = Duration::from_secs(30);
const RENDEZVOUS_RETRY_MAX: Duration = Duration::from_secs(30 * 60);
const RENDEZVOUS_HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const RENDEZVOUS_ENVELOPE_MAX_BYTES: usize = 4_096;
const EXPECTED_SYNC_SERVER_SIZE: Option<&str> = option_env!("GRIDTIMER_EXPECTED_SYNC_SERVER_SIZE");
const EXPECTED_SYNC_SERVER_SHA256: Option<&str> =
    option_env!("GRIDTIMER_EXPECTED_SYNC_SERVER_SHA256");
const EXPECTED_SYNC_SERVER_BINDING_V1: Option<&str> =
    option_env!("GRIDTIMER_EXPECTED_SYNC_SERVER_BINDING_V1");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SyncServerExpectation {
    file_name: &'static str,
    expected_size: Option<u64>,
    expected_sha256: Option<&'static str>,
}

struct ActivePublicTunnel {
    url: String,
    child: Option<Child>,
    log_path: Option<PathBuf>,
    initial_health_verified: bool,
    health_failure_policy: PublicHealthFailurePolicy,
    quick_runtime: Option<QuickTunnelRuntime>,
}

impl ActivePublicTunnel {
    fn child_exit_status(&mut self) -> Result<Option<ExitStatus>, String> {
        match self.child.as_mut() {
            Some(child) => child.try_wait().map_err(|error| error.to_string()),
            None => Ok(None),
        }
    }

    fn log_limit_exceeded(&self) -> bool {
        let Some(path) = self.log_path.as_ref() else {
            return false;
        };
        match path.metadata() {
            Ok(metadata) => metadata.len() >= TUNNEL_LOG_MAX_BYTES,
            Err(error) => error.kind() != std::io::ErrorKind::NotFound,
        }
    }

    fn terminal_quick_tunnel_invalidation(&mut self) -> Result<bool, String> {
        let Some(runtime) = self.quick_runtime.as_mut() else {
            return Ok(false);
        };
        let Some(log_path) = self.log_path.as_ref() else {
            return Ok(false);
        };
        let new_lines = read_new_complete_log_lines(log_path, &mut runtime.log_offset)?;
        if runtime.metrics_addr.is_none() {
            runtime.metrics_addr = extract_quick_tunnel_metrics_addr(&new_lines);
        }
        Ok(contains_terminal_quick_tunnel_invalidation(&new_lines))
    }

    fn quick_tunnel_ready(&self) -> Option<bool> {
        let runtime = self.quick_runtime.as_ref()?;
        runtime.metrics_addr.and_then(verify_quick_tunnel_ready)
    }
}

#[derive(Debug)]
struct QuickTunnelRuntime {
    log_offset: u64,
    metrics_addr: Option<SocketAddr>,
}

struct DohAgents {
    proxy_aware: Option<ureq::Agent>,
    direct: ureq::Agent,
}

impl DohAgents {
    fn new() -> Self {
        Self {
            proxy_aware: proxy_environment_is_configured().then(build_proxy_aware_doh_agent),
            direct: build_direct_doh_agent(),
        }
    }

    fn resolve(&self, query: &[u8]) -> Option<Vec<u8>> {
        if let Some(agent) = self.proxy_aware.as_ref() {
            if let Some(response) = request_dns_over_https(agent, query) {
                return Some(response);
            }
        }
        request_dns_over_https(&self.direct, query)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicHealthFailurePolicy {
    RetainQuickTunnel,
    ReplaceAfterThreshold,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LocalServerHealth {
    Healthy,
    Busy,
    Unavailable,
}

impl LocalServerHealth {
    fn is_ready(self) -> bool {
        matches!(self, Self::Healthy | Self::Busy)
    }
}

impl PublicHealthFailurePolicy {
    fn should_replace(
        self,
        consecutive_health_failures: u32,
        _initial_health_verified: bool,
    ) -> bool {
        consecutive_health_failures >= MAX_CONSECUTIVE_HEALTH_FAILURES
            && matches!(self, Self::ReplaceAfterThreshold)
    }
}

#[derive(Debug, Default)]
struct SustainedQuickTunnelFailureState {
    consecutive_failures: u32,
    first_failure_at: Option<Instant>,
}

impl SustainedQuickTunnelFailureState {
    fn observe(&mut self, healthy: bool, now: Instant) {
        if healthy {
            self.consecutive_failures = 0;
            self.first_failure_at = None;
            return;
        }
        if self.consecutive_failures == 0 {
            self.first_failure_at = Some(now);
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    fn sustained_failure_reached(&self, now: Instant) -> bool {
        self.consecutive_failures >= VERIFIED_QUICK_TUNNEL_FAILURE_THRESHOLD
            && self.first_failure_at.is_some_and(|first_failure_at| {
                now.checked_duration_since(first_failure_at)
                    .is_some_and(|elapsed| elapsed >= VERIFIED_QUICK_TUNNEL_FAILURE_WINDOW)
            })
    }
}

fn should_rotate_managed_quick_tunnel(
    state: &SustainedQuickTunnelFailureState,
    now: Instant,
    initial_health_verified: bool,
    is_quick_tunnel: bool,
    owns_tunnel_child: bool,
) -> bool {
    is_quick_tunnel
        && owns_tunnel_child
        && if initial_health_verified {
            state.sustained_failure_reached(now)
        } else {
            state.consecutive_failures >= VERIFIED_QUICK_TUNNEL_FAILURE_THRESHOLD
                && state.first_failure_at.is_some_and(|started| {
                    now.saturating_duration_since(started) >= PENDING_QUICK_TUNNEL_FAILURE_WINDOW
                })
        }
}

#[derive(Debug, Default)]
struct RendezvousPublicationSchedule {
    attempted_url: Option<String>,
    consecutive_failures: u32,
    next_attempt_at: Option<Instant>,
}

impl RendezvousPublicationSchedule {
    fn should_publish(&self, now: Instant, public_url: &str, force: bool) -> bool {
        force
            || self.attempted_url.as_deref() != Some(public_url)
            || self.next_attempt_at.is_none_or(|next| now >= next)
    }

    fn record_success(&mut self, now: Instant, public_url: &str) {
        self.attempted_url = Some(public_url.to_string());
        self.consecutive_failures = 0;
        self.next_attempt_at = Some(now + RENDEZVOUS_RENEW_INTERVAL);
    }

    fn record_failure(&mut self, now: Instant, public_url: &str) -> Duration {
        self.attempted_url = Some(public_url.to_string());
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let delay = rendezvous_retry_delay(self.consecutive_failures);
        self.next_attempt_at = Some(now + delay);
        delay
    }
}

#[derive(Default)]
struct RendezvousPublisher {
    schedule: RendezvousPublicationSchedule,
    last_generation: u64,
}

impl RendezvousPublisher {
    fn publish_if_due(&mut self, public_url: &str, force: bool) {
        let now = Instant::now();
        if !self.schedule.should_publish(now, public_url, force) {
            return;
        }
        let issued_at_unix = unix_time_seconds();
        let publication =
            allocate_persistent_rendezvous_generation(issued_at_unix, self.last_generation)
                .and_then(|generation| {
                    self.last_generation = generation;
                    publish_signed_rendezvous_envelope(public_url, generation, issued_at_unix)
                });
        match publication {
            Ok(topic) => {
                self.schedule.record_success(now, public_url);
                write_log(&format!(
                    "published signed public tunnel rendezvous announcement to topic {topic}"
                ));
            }
            Err(error) => {
                let delay = self.schedule.record_failure(now, public_url);
                write_log(&format!(
                    "could not publish signed rendezvous announcement: {error}; local sync and the public tunnel remain active, retrying in {} seconds",
                    delay.as_secs()
                ));
            }
        }
    }
}

fn rendezvous_retry_delay(consecutive_failures: u32) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(16);
    let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
    Duration::from_secs(
        RENDEZVOUS_RETRY_INITIAL
            .as_secs()
            .saturating_mul(multiplier)
            .min(RENDEZVOUS_RETRY_MAX.as_secs()),
    )
}

fn unix_time_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn allocate_persistent_rendezvous_generation(
    now_unix: u64,
    last_in_memory: u64,
) -> Result<u64, String> {
    allocate_rendezvous_generation_in_file(
        &launcher_runtime_directory().join(RENDEZVOUS_GENERATION_FILE_NAME),
        now_unix,
        last_in_memory,
    )
}

fn allocate_rendezvous_generation_in_file(
    path: &Path,
    now_unix: u64,
    last_in_memory: u64,
) -> Result<u64, String> {
    let persisted = read_persisted_rendezvous_generation(path)?.unwrap_or(0);
    let generation = persisted
        .max(last_in_memory)
        .checked_add(1)
        .ok_or_else(|| "rendezvous generation is exhausted".to_string())?
        .max(now_unix)
        .max(1);
    atomic_write_text(path, &generation.to_string()).map_err(|error| {
        format!("could not persist the next rendezvous generation before publishing: {error}")
    })?;
    Ok(generation)
}

fn read_persisted_rendezvous_generation(path: &Path) -> Result<Option<u64>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "could not inspect persisted rendezvous generation: {error}"
            ));
        }
    };
    if metadata.file_type().is_symlink() {
        return Err("persisted rendezvous generation must not be a symbolic link".to_string());
    }
    if !metadata.is_file() {
        return Err("persisted rendezvous generation must be a regular file".to_string());
    }
    if metadata.len() == 0 || metadata.len() > RENDEZVOUS_GENERATION_MAX_BYTES {
        return Err("persisted rendezvous generation has an invalid size".to_string());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    fs::File::open(path)
        .map_err(|error| format!("could not open persisted rendezvous generation: {error}"))?
        .take(RENDEZVOUS_GENERATION_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read persisted rendezvous generation: {error}"))?;
    if bytes.is_empty()
        || bytes.len() as u64 > RENDEZVOUS_GENERATION_MAX_BYTES
        || !bytes.iter().all(u8::is_ascii_digit)
        || (bytes.len() > 1 && bytes[0] == b'0')
    {
        return Err("persisted rendezvous generation has an invalid format".to_string());
    }
    let value = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| "persisted rendezvous generation is out of range".to_string())?;
    Ok(Some(value))
}

fn publish_signed_rendezvous_envelope(
    public_url: &str,
    generation: u64,
    issued_at_unix: u64,
) -> Result<String, String> {
    let identity = ensure_local_rendezvous_identity()?;
    let expires_at_unix = issued_at_unix
        .checked_add(RENDEZVOUS_ANNOUNCEMENT_LIFETIME.as_secs())
        .ok_or_else(|| "rendezvous announcement expiry overflowed".to_string())?;
    let payload = RendezvousPayload::new_with_generation(
        public_url,
        SYNC_SERVER_BUILD_ID,
        generation,
        issued_at_unix,
        expires_at_unix,
    )?;
    let envelope = identity.sign_payload(&payload)?;
    if envelope.len() > RENDEZVOUS_ENVELOPE_MAX_BYTES {
        return Err("signed rendezvous envelope exceeds the ntfy message limit".to_string());
    }
    let topic = identity.topic().to_string();
    let endpoint = publish_url(&topic)?;
    post_rendezvous_envelope(&endpoint, &envelope)?;
    Ok(topic)
}

fn post_rendezvous_envelope(endpoint: &str, envelope: &str) -> Result<(), String> {
    let mut last_transport_error = None;
    let build_agent = |use_proxy_environment| {
        ureq::AgentBuilder::new()
            .try_proxy_from_env(use_proxy_environment)
            .https_only(true)
            .redirects(0)
            .timeout_connect(RENDEZVOUS_HTTP_TIMEOUT)
            .timeout(RENDEZVOUS_HTTP_TIMEOUT)
    };
    let agents = public_tunnel_probe_proxy_modes()
        .into_iter()
        .map(|use_proxy| Some(build_agent(use_proxy).build()))
        .chain(std::iter::once_with(|| {
            let hostname = rendezvous_dns_host(endpoint)?;
            Some(
                build_agent(false)
                    .resolver(resolve_public_dns(hostname)?)
                    .build(),
            )
        }));
    for agent in agents.flatten() {
        match agent
            .post(endpoint)
            .set("Content-Type", "text/plain; charset=utf-8")
            .send_string(envelope)
        {
            Ok(response) if (200..=299).contains(&response.status()) => return Ok(()),
            Ok(response) => {
                return Err(format!(
                    "rendezvous service returned HTTP {}",
                    response.status()
                ));
            }
            Err(ureq::Error::Status(status, _)) => {
                return Err(format!("rendezvous service returned HTTP {status}"));
            }
            Err(ureq::Error::Transport(error)) => {
                last_transport_error = Some(error.to_string());
            }
        }
    }
    Err(format!(
        "rendezvous service transport failed{}",
        last_transport_error
            .map(|error| format!(": {error}"))
            .unwrap_or_default()
    ))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicUrlAction {
    Publish,
    KeepPublished,
    Withdraw,
    KeepWithdrawn,
}

#[derive(Default)]
struct PublicUrlPublicationState {
    published: bool,
    consecutive_health_failures: u32,
}

impl PublicUrlPublicationState {
    fn observe(
        &mut self,
        healthy: bool,
        failure_policy: PublicHealthFailurePolicy,
    ) -> PublicUrlAction {
        if healthy {
            self.consecutive_health_failures = 0;
            if self.published {
                PublicUrlAction::KeepPublished
            } else {
                PublicUrlAction::Publish
            }
        } else {
            self.consecutive_health_failures = self.consecutive_health_failures.saturating_add(1);
            if self.published
                && matches!(failure_policy, PublicHealthFailurePolicy::RetainQuickTunnel)
            {
                PublicUrlAction::KeepPublished
            } else if self.published {
                self.published = false;
                PublicUrlAction::Withdraw
            } else {
                PublicUrlAction::KeepWithdrawn
            }
        }
    }

    fn mark_published(&mut self) {
        self.published = true;
    }
}

fn reconcile_public_url_publication(
    state: &mut PublicUrlPublicationState,
    healthy: bool,
    failure_policy: PublicHealthFailurePolicy,
    public_url_file: &Path,
    public_url: &str,
) {
    let failures_before_observation = state.consecutive_health_failures;
    match state.observe(healthy, failure_policy) {
        PublicUrlAction::Publish => {
            if let Err(error) = atomic_write_text(public_url_file, public_url) {
                write_log(&format!("could not publish verified public URL: {error}"));
                return;
            }
            state.mark_published();
            if failures_before_observation > 0 {
                write_log(&format!(
                    "public HTTPS tunnel recovered without changing URL: {public_url}"
                ));
            } else {
                write_log(&format!("verified public tunnel: {public_url}"));
            }
        }
        PublicUrlAction::KeepPublished => {
            if !healthy {
                let failures = state.consecutive_health_failures;
                if failures == 1 || failures % MAX_CONSECUTIVE_HEALTH_FAILURES == 0 {
                    write_log(&format!(
                        "public HTTPS health check failed {failures} time(s); retaining the already verified quick tunnel URL while its process remains active: {public_url}"
                    ));
                }
            }
        }
        PublicUrlAction::Withdraw => {
            remove_public_url_file(public_url_file);
            write_log(&format!(
                "public HTTPS tunnel health failed; withdrew the published URL and kept the same tunnel process: {public_url}"
            ));
        }
        PublicUrlAction::KeepWithdrawn => {
            let failures = state.consecutive_health_failures;
            if failures == 1 || failures % MAX_CONSECUTIVE_HEALTH_FAILURES == 0 {
                write_log(&format!(
                    "public HTTPS tunnel is still pending after {failures} health failure(s); keeping the same tunnel process: {public_url}"
                ));
            }
        }
    }
}

impl Drop for ActivePublicTunnel {
    fn drop(&mut self) {
        terminate_child(&mut self.child);
    }
}

#[derive(Default)]
struct SupervisorState {
    server_child: Option<Child>,
    firewall_checked_for: Option<PathBuf>,
    consecutive_local_health_failures: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StartupRegistration {
    command: String,
    executable: PathBuf,
    stable_entry: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct StartupRegistrationPlan {
    before_supervisor_lock: Option<StartupRegistration>,
    after_supervisor_lock: Option<StartupRegistration>,
}

impl Drop for SupervisorState {
    fn drop(&mut self) {
        terminate_child(&mut self.server_child);
    }
}

fn main() {
    if env::args().nth(1).as_deref() == Some("--check-public-url") {
        let args = env::args().collect::<Vec<_>>();
        let verified = args.len() == 3 && verify_public_tunnel(&args[2]);
        println!("{}", serde_json::json!({"verified": verified}));
        std::process::exit(if verified { 0 } else { 1 });
    }
    let current_exe = env::current_exe();
    let autostart_disabled = launcher_autostart_registration_disabled();
    if autostart_disabled {
        write_log(
            "startup registration disabled for this launcher run by GRID_TIMER_LAUNCHER_DISABLE_AUTOSTART=1",
        );
    }
    let startup_registration_plan = match current_exe.as_deref() {
        Ok(launcher_path) => startup_registration_plan(launcher_path, autostart_disabled),
        Err(error) => {
            write_log(&format!(
                "could not resolve launcher path for startup migration: {error}"
            ));
            StartupRegistrationPlan::default()
        }
    };
    let _single_instance = match acquire_supervisor_with_startup_registration(
        &startup_registration_plan,
        ensure_startup_task,
        SingleInstanceGuard::acquire,
    ) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            write_log("another sync supervisor instance is already active");
            return;
        }
        Err(error) => {
            write_log(&format!(
                "could not acquire supervisor instance lock: {error}"
            ));
            return;
        }
    };
    let process_job = match ProcessJob::new() {
        Ok(job) => job,
        Err(error) => {
            write_log(&format!("could not create child process job: {error}"));
            return;
        }
    };
    loop {
        match launch_sync_server(&process_job) {
            Ok(()) => return,
            Err(error) => {
                write_log(&format!("launcher failed: {error}; retrying"));
                thread::sleep(SERVER_RETRY_DELAY);
            }
        }
    }
}

fn acquire_supervisor_with_startup_registration<T>(
    plan: &StartupRegistrationPlan,
    mut register_startup: impl FnMut(&StartupRegistration),
    acquire_instance: impl FnOnce() -> Result<Option<T>, String>,
) -> Result<Option<T>, String> {
    // Only a fully validated stable entry may mutate startup state before the
    // cross-version mutex. A version-pinned compatibility entry is safe only
    // after this process has become the active supervisor.
    if let Some(registration) = plan.before_supervisor_lock.as_ref() {
        register_startup(registration);
    }
    let instance = acquire_instance()?;
    if instance.is_some() {
        if let Some(registration) = plan.after_supervisor_lock.as_ref() {
            register_startup(registration);
        }
    }
    Ok(instance)
}

fn launch_sync_server(process_job: &ProcessJob) -> Result<(), String> {
    let current_exe = env::current_exe().map_err(|error| error.to_string())?;
    let Some(app_dir) = current_exe.parent() else {
        return Err("could not resolve launcher directory".to_string());
    };

    let public_url_file = launcher_runtime_directory().join("sync_public_server_url.txt");
    if let Some(parent) = public_url_file.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut state = SupervisorState::default();
    let mut rendezvous_publisher = RendezvousPublisher::default();

    loop {
        let server_path = match find_sync_server(app_dir) {
            Ok(path) => path,
            Err(error) => {
                remove_public_url_file(&public_url_file);
                write_log(&format!(
                    "could not verify the exact sync server executable; retrying: {error}"
                ));
                thread::sleep(SERVER_RETRY_DELAY);
                continue;
            }
        };
        if state.firewall_checked_for.as_ref() != Some(&server_path) {
            ensure_firewall_rule(&server_path);
            state.firewall_checked_for = Some(server_path.clone());
        }
        if let Err(error) = ensure_sync_server_running(
            app_dir,
            &server_path,
            &public_url_file,
            process_job,
            &mut state,
        ) {
            remove_public_url_file(&public_url_file);
            write_log(&format!("local sync server is unavailable: {error}"));
            thread::sleep(SERVER_RETRY_DELAY);
            continue;
        }
        remove_public_url_file(&public_url_file);
        let Some(mut tunnel) = (match launch_public_tunnel(app_dir, process_job) {
            Ok(tunnel) => tunnel,
            Err(error) => {
                write_log(&format!("could not launch public HTTPS tunnel: {error}"));
                thread::sleep(TUNNEL_RETRY_DELAY);
                continue;
            }
        }) else {
            write_log("public HTTPS tunnel unavailable; retrying");
            thread::sleep(TUNNEL_RETRY_DELAY);
            continue;
        };
        let mut publication_state = PublicUrlPublicationState::default();
        let mut sustained_quick_failure_state = SustainedQuickTunnelFailureState::default();
        if tunnel.initial_health_verified {
            reconcile_public_url_publication(
                &mut publication_state,
                true,
                tunnel.health_failure_policy,
                &public_url_file,
                &tunnel.url,
            );
            rendezvous_publisher.publish_if_due(&tunnel.url, true);
            if tunnel.quick_runtime.is_some() {
                if let Err(error) = tunnel_retry::observe(true) {
                    write_log(&format!(
                        "could not record tunnel health for retry pacing: {error}"
                    ));
                }
            }
        } else {
            write_log(&format!(
                "public tunnel supervisor is waiting for initial health without changing the quick tunnel URL: {}",
                tunnel.url
            ));
        }

        loop {
            thread::sleep(SUPERVISOR_CHECK_INTERVAL);
            if tunnel.log_limit_exceeded() {
                write_log("public tunnel log reached its size limit; replacing the tunnel");
                break;
            }
            let expected_managed_pid = state.server_child.as_ref().map(Child::id);
            match probe_local_server(&server_path, expected_managed_pid) {
                LocalServerHealth::Healthy => {
                    state.consecutive_local_health_failures = 0;
                    reap_exited_child(&mut state.server_child)?;
                }
                LocalServerHealth::Busy => {
                    state.consecutive_local_health_failures = 0;
                    reap_exited_child(&mut state.server_child)?;
                    write_log(
                        "local sync server is healthy but temporarily at connection capacity; preserving the running process",
                    );
                }
                LocalServerHealth::Unavailable => {
                    let had_managed_child = state.server_child.is_some();
                    reap_exited_child(&mut state.server_child)?;
                    state.consecutive_local_health_failures =
                        state.consecutive_local_health_failures.saturating_add(1);
                    let child_exited = had_managed_child && state.server_child.is_none();
                    let restart_due = child_exited
                        || state.consecutive_local_health_failures
                            >= LOCAL_SERVER_RESTART_FAILURE_THRESHOLD;
                    if restart_due {
                        if state.server_child.is_some() {
                            write_log(&format!(
                                "managed sync server failed {} consecutive health checks; replacing it",
                                state.consecutive_local_health_failures
                            ));
                            terminate_child(&mut state.server_child);
                        } else {
                            write_log(
                                "local sync server process exited or no matching managed process remains; restarting it",
                            );
                        }
                        if let Err(error) = ensure_sync_server_running(
                            app_dir,
                            &server_path,
                            &public_url_file,
                            process_job,
                            &mut state,
                        ) {
                            write_log(&format!("could not restart local sync server: {error}"));
                            break;
                        }
                    } else {
                        write_log(&format!(
                            "managed sync server health check failed {}/{}; preserving the live process while confirming the failure",
                            state.consecutive_local_health_failures,
                            LOCAL_SERVER_RESTART_FAILURE_THRESHOLD
                        ));
                    }
                }
            }
            match tunnel.child_exit_status() {
                Ok(Some(status)) => {
                    write_log(&format!(
                        "public tunnel process exited with {status}; replacing the tunnel"
                    ));
                    break;
                }
                Ok(None) => {}
                Err(error) => {
                    write_log(&format!("could not inspect public tunnel process: {error}"));
                }
            }
            match tunnel.terminal_quick_tunnel_invalidation() {
                Ok(true) => {
                    write_log(
                        "quick tunnel was explicitly invalidated by the service; replacing the child and requesting a new URL",
                    );
                    break;
                }
                Ok(false) => {}
                Err(error) => {
                    write_log(&format!(
                        "could not inspect new quick tunnel log content: {error}"
                    ));
                }
            }
            let ready = tunnel.quick_tunnel_ready().unwrap_or(true);
            let healthy = ready && verify_public_tunnel(&tunnel.url);
            if tunnel.quick_runtime.is_some() {
                if let Err(error) = tunnel_retry::observe(healthy) {
                    write_log(&format!(
                        "could not record tunnel health for retry pacing: {error}"
                    ));
                }
            }
            let health_observed_at = Instant::now();
            sustained_quick_failure_state.observe(healthy, health_observed_at);
            let first_verified_health = healthy && !tunnel.initial_health_verified;
            if healthy {
                tunnel.initial_health_verified = true;
            }
            reconcile_public_url_publication(
                &mut publication_state,
                healthy,
                tunnel.health_failure_policy,
                &public_url_file,
                &tunnel.url,
            );
            if healthy {
                rendezvous_publisher.publish_if_due(&tunnel.url, first_verified_health);
            }
            if should_rotate_managed_quick_tunnel(
                &sustained_quick_failure_state,
                health_observed_at,
                tunnel.initial_health_verified,
                tunnel.quick_runtime.is_some(),
                tunnel.child.is_some(),
            ) {
                write_log(&format!(
                    "managed quick tunnel remained unhealthy for {} consecutive checks across at least {} seconds; replacing it and requesting a fresh public URL",
                    sustained_quick_failure_state.consecutive_failures,
                    if tunnel.initial_health_verified { VERIFIED_QUICK_TUNNEL_FAILURE_WINDOW } else { PENDING_QUICK_TUNNEL_FAILURE_WINDOW }.as_secs(),
                ));
                break;
            }
            if !healthy
                && tunnel.health_failure_policy.should_replace(
                    publication_state.consecutive_health_failures,
                    tunnel.initial_health_verified,
                )
            {
                if matches!(
                    tunnel.health_failure_policy,
                    PublicHealthFailurePolicy::RetainQuickTunnel
                ) {
                    write_log(
                        "quick tunnel failed its first three public health checks; replacing it instead of leaving discovery pending",
                    );
                } else {
                    write_log(
                        "stable public tunnel remained unhealthy; entering the existing fallback path",
                    );
                }
                break;
            }
        }
        remove_public_url_file(&public_url_file);
        drop(tunnel);
        thread::sleep(TUNNEL_RETRY_DELAY);
    }
}

fn launcher_autostart_registration_disabled() -> bool {
    autostart_registration_disabled(env::var_os(DISABLE_AUTOSTART_ENV).as_deref())
}

fn autostart_registration_disabled(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

fn ensure_sync_server_running(
    app_dir: &Path,
    server_path: &Path,
    public_url_file: &Path,
    process_job: &ProcessJob,
    state: &mut SupervisorState,
) -> Result<(), String> {
    let expected_managed_pid = state.server_child.as_ref().map(Child::id);
    if probe_local_server(server_path, expected_managed_pid).is_ready() {
        state.consecutive_local_health_failures = 0;
        reap_exited_child(&mut state.server_child)?;
        return Ok(());
    }
    reap_exited_child(&mut state.server_child)?;
    if state.server_child.is_some() {
        state.consecutive_local_health_failures =
            state.consecutive_local_health_failures.saturating_add(1);
        if state.consecutive_local_health_failures < LOCAL_SERVER_RESTART_FAILURE_THRESHOLD {
            return Err(format!(
                "managed sync server health check failed {}/{}; preserving it pending confirmation",
                state.consecutive_local_health_failures, LOCAL_SERVER_RESTART_FAILURE_THRESHOLD
            ));
        }
        terminate_child(&mut state.server_child);
    }
    if state.server_child.is_none() {
        verify_sync_server_file(server_path, app_dir)?;
        if local_sync_port_is_open() {
            return Err(
                "TCP 8917 is occupied by a service that failed the sync health check".to_string(),
            );
        }
        let mut server_command = Command::new(server_path);
        let (stdout_log, stderr_log) = prepare_server_output_logs()?;
        server_command
            .arg(SYNC_BIND_ADDR)
            .env("GRID_TIMER_PUBLIC_SERVER_URL_FILE", public_url_file)
            .current_dir(app_dir);
        configure_background_process(&mut server_command);
        server_command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut server_child = spawn_managed_child(
            &mut server_command,
            process_job,
            &format!("could not start {}", server_path.display()),
        )?;
        if let Err(error) = attach_server_output_logs(&mut server_child, stdout_log, stderr_log) {
            let mut child = Some(server_child);
            terminate_child(&mut child);
            return Err(error);
        }
        state.server_child = Some(server_child);
        state.consecutive_local_health_failures = 0;
    }
    let startup_storage_bytes = sync_startup_storage_bytes(&launcher_runtime_directory());
    let runtime_directory = launcher_runtime_directory();
    let backup_directory = gridtimer_native::sync_core::configured_runtime_backup_directory(
        &runtime_directory.join("server_store.sqlite3"),
    );
    let startup_archive_bytes = sync_startup_archive_bytes(&runtime_directory, &backup_directory);
    let startup_timeout =
        server_startup_timeout_for_bytes(startup_storage_bytes, startup_archive_bytes);
    write_log(&format!(
        "waiting up to {} seconds for database initialization and backup maintenance ({} live/legacy bytes, {} archive bytes)",
        startup_timeout.as_secs(),
        startup_storage_bytes,
        startup_archive_bytes
    ));
    let started = Instant::now();
    while started.elapsed() < startup_timeout {
        let expected_managed_pid = state.server_child.as_ref().map(Child::id);
        if probe_local_server(server_path, expected_managed_pid).is_ready() {
            state.consecutive_local_health_failures = 0;
            return Ok(());
        }
        reap_exited_child(&mut state.server_child)?;
        if state.server_child.is_none() {
            return Err("sync server process exited before it became healthy".to_string());
        }
        thread::sleep(Duration::from_millis(250));
    }
    terminate_child(&mut state.server_child);
    Err(format!(
        "sync server did not become healthy within {} seconds",
        startup_timeout.as_secs()
    ))
}

fn server_startup_timeout_for_bytes(storage_bytes: u64, archive_bytes: u64) -> Duration {
    const MIB: u64 = 1024 * 1024;
    let mebibytes = storage_bytes.saturating_add(MIB - 1) / MIB;
    let extra_seconds = mebibytes.saturating_mul(SERVER_STARTUP_SECONDS_PER_MIB);
    // Archive maintenance is sequential and resumable. Its volume must not
    // exhaust a deadline based only on the live database. Allow one second
    // per archive MiB while retaining the existing absolute upper bound.
    let archive_seconds = archive_bytes.saturating_add(MIB - 1) / MIB;
    Duration::from_secs(
        SERVER_STARTUP_BASE_TIMEOUT
            .as_secs()
            .saturating_add(extra_seconds)
            .saturating_add(archive_seconds)
            .min(SERVER_STARTUP_MAX_TIMEOUT.as_secs()),
    )
}

fn sync_startup_archive_bytes(runtime_directory: &Path, backup_directory: &Path) -> u64 {
    let mut total = 0_u64;
    let directories = if runtime_directory == backup_directory {
        vec![runtime_directory]
    } else {
        vec![runtime_directory, backup_directory]
    };
    for directory in directories {
        if !is_real_directory(directory) {
            continue;
        }
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|name| name.strip_suffix(".sqlite3")) else {
                continue;
            };
            let startup = directory == runtime_directory
                && (stem.starts_with("server_store_startup_")
                    || stem.starts_with("server_store_pre_schema_v"));
            let runtime = directory == backup_directory
                && stem.strip_prefix("sync_server_").is_some_and(|stem| {
                    stem.split_once("_runtime_").is_some_and(|(id, timestamp)| {
                        id.len() == 64
                            && id.bytes().all(|byte| byte.is_ascii_hexdigit())
                            && !timestamp.is_empty()
                            && timestamp
                                .bytes()
                                .all(|byte| byte.is_ascii_digit() || byte == b'_')
                    })
                });
            if !startup && !runtime {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if metadata.is_file() && !metadata_is_reparse_point(&metadata) {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    total
}

fn sync_startup_storage_bytes(runtime_directory: &Path) -> u64 {
    let mut total = 0_u64;
    for file_name in [
        "server_store.sqlite3",
        "server_store.sqlite3-wal",
        "server_store.json",
    ] {
        total = total.saturating_add(
            runtime_directory
                .join(file_name)
                .metadata()
                .map(|metadata| {
                    if metadata.is_file() {
                        metadata.len()
                    } else {
                        0
                    }
                })
                .unwrap_or(0),
        );
    }
    total
}

fn launch_public_tunnel(
    app_dir: &Path,
    process_job: &ProcessJob,
) -> Result<Option<ActivePublicTunnel>, String> {
    let cloudflared = app_dir.join("tools").join("cloudflared.exe");
    if let Some(stable_url) = configured_stable_public_url(app_dir) {
        if let Some(token_file) = configured_cloudflared_tunnel_token_file(app_dir) {
            if cloudflared.is_file() {
                if let Some((child, log_path)) = launch_named_public_tunnel(
                    app_dir,
                    &cloudflared,
                    &stable_url,
                    &token_file,
                    process_job,
                )? {
                    return Ok(Some(ActivePublicTunnel {
                        url: stable_url,
                        child: Some(child),
                        log_path: Some(log_path),
                        initial_health_verified: true,
                        health_failure_policy: PublicHealthFailurePolicy::ReplaceAfterThreshold,
                        quick_runtime: None,
                    }));
                }
                write_log("stable public tunnel did not verify; falling back to quick tunnel");
            } else if verify_public_tunnel(&stable_url) {
                write_log(&format!(
                    "using externally managed stable public URL: {stable_url}"
                ));
                return Ok(Some(ActivePublicTunnel {
                    url: stable_url,
                    child: None,
                    log_path: None,
                    initial_health_verified: true,
                    health_failure_policy: PublicHealthFailurePolicy::ReplaceAfterThreshold,
                    quick_runtime: None,
                }));
            }
        } else if verify_public_tunnel(&stable_url) {
            write_log(&format!("using configured stable public URL: {stable_url}"));
            return Ok(Some(ActivePublicTunnel {
                url: stable_url,
                child: None,
                log_path: None,
                initial_health_verified: true,
                health_failure_policy: PublicHealthFailurePolicy::ReplaceAfterThreshold,
                quick_runtime: None,
            }));
        } else {
            write_log(
                "stable public URL is configured but not reachable; falling back to quick tunnel",
            );
        }
    }
    if !cloudflared.is_file() {
        return Ok(None);
    }
    launch_quick_public_tunnel(app_dir, &cloudflared, process_job)
}

fn launch_quick_public_tunnel(
    app_dir: &Path,
    cloudflared: &Path,
    process_job: &ProcessJob,
) -> Result<Option<ActivePublicTunnel>, String> {
    let cooldown = quick_tunnel_provision::cooldown_seconds().map_err(|error| error.to_string())?;
    if cooldown > 0 {
        write_log(&format!(
            "public tunnel retry pacing or provider cooldown; next application in {cooldown} seconds"
        ));
        return Ok(None);
    }
    let provision_relay = quick_tunnel_provision::start()
        .map_err(|error| format!("could not prepare private tunnel provisioning route: {error}"))?;
    let log_path = launcher_runtime_directory().join("cloudflared_quick_tunnel.log");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    prepare_tunnel_log(&log_path)?;
    let edge_ips = resolve_cloudflared_edge_ips();
    let mut tunnel_command = Command::new(cloudflared);
    append_cloudflared_tunnel_prefix(&mut tunnel_command, &edge_ips);
    tunnel_command.args(["--quick-service", &provision_relay.url]);
    append_quick_tunnel_mode_args(&mut tunnel_command, &log_path);
    tunnel_command.current_dir(app_dir);
    configure_background_process(&mut tunnel_command);
    let mut tunnel_child = spawn_managed_child(
        &mut tunnel_command,
        process_job,
        "could not start public tunnel",
    )?;
    let mut candidate_url: Option<String> = None;
    let mut metrics_addr: Option<SocketAddr> = None;
    let mut log_offset = 0_u64;
    let mut startup_health_failures = 0_u32;
    let started_at = Instant::now();
    let mut last_verify = Instant::now() - Duration::from_secs(3);
    while started_at.elapsed() < Duration::from_secs(40) {
        thread::sleep(Duration::from_millis(500));
        match tunnel_child.try_wait() {
            Ok(Some(_)) => return Ok(None),
            Ok(None) => {}
            Err(error) => {
                let _ = tunnel_child.kill();
                let _ = tunnel_child.wait();
                return Err(format!("could not inspect public tunnel process: {error}"));
            }
        }
        if let Ok(new_lines) = read_new_complete_log_lines(&log_path, &mut log_offset) {
            if contains_terminal_quick_tunnel_invalidation(&new_lines) {
                write_log(
                    "new quick tunnel was explicitly rejected during startup; requesting another URL",
                );
                let _ = tunnel_child.kill();
                let _ = tunnel_child.wait();
                return Ok(None);
            }
            if candidate_url.is_none() {
                candidate_url = extract_trycloudflare_url(&new_lines);
            }
            if metrics_addr.is_none() {
                metrics_addr = extract_quick_tunnel_metrics_addr(&new_lines);
            }
        }
        if let Some(url) = candidate_url.as_deref() {
            if last_verify.elapsed() >= Duration::from_secs(2) {
                last_verify = Instant::now();
                let ready = metrics_addr
                    .and_then(verify_quick_tunnel_ready)
                    .unwrap_or(true);
                if ready && verify_public_tunnel(url) {
                    return Ok(Some(ActivePublicTunnel {
                        url: url.to_string(),
                        child: Some(tunnel_child),
                        log_path: Some(log_path),
                        initial_health_verified: true,
                        health_failure_policy: PublicHealthFailurePolicy::RetainQuickTunnel,
                        quick_runtime: Some(QuickTunnelRuntime {
                            log_offset,
                            metrics_addr,
                        }),
                    }));
                }
                startup_health_failures = startup_health_failures.saturating_add(1);
                if PublicHealthFailurePolicy::RetainQuickTunnel
                    .should_replace(startup_health_failures, false)
                {
                    write_log(
                        "new quick tunnel failed its first three public health checks; replacing it",
                    );
                    let _ = tunnel_child.kill();
                    let _ = tunnel_child.wait();
                    return Ok(None);
                }
            }
        }
    }
    if let Some(url) = candidate_url {
        match tunnel_child.try_wait() {
            Ok(None) => {
                write_log(&format!(
                    "quick tunnel URL acquired but public health is pending; retaining the same tunnel: {url}"
                ));
                return Ok(Some(ActivePublicTunnel {
                    url,
                    child: Some(tunnel_child),
                    log_path: Some(log_path),
                    initial_health_verified: false,
                    health_failure_policy: PublicHealthFailurePolicy::RetainQuickTunnel,
                    quick_runtime: Some(QuickTunnelRuntime {
                        log_offset,
                        metrics_addr,
                    }),
                }));
            }
            Ok(Some(status)) => {
                write_log(&format!(
                    "quick tunnel process exited before public health succeeded: {status}"
                ));
                return Ok(None);
            }
            Err(error) => {
                let _ = tunnel_child.kill();
                let _ = tunnel_child.wait();
                return Err(format!("could not inspect public tunnel process: {error}"));
            }
        }
    }
    let _ = tunnel_child.kill();
    let _ = tunnel_child.wait();
    Ok(None)
}

fn launch_named_public_tunnel(
    app_dir: &Path,
    cloudflared: &Path,
    stable_url: &str,
    token_file: &Path,
    process_job: &ProcessJob,
) -> Result<Option<(Child, PathBuf)>, String> {
    let log_path = launcher_runtime_directory().join("cloudflared_named_tunnel.log");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    prepare_tunnel_log(&log_path)?;
    let edge_ips = resolve_cloudflared_edge_ips();
    let mut tunnel_command = Command::new(cloudflared);
    append_cloudflared_tunnel_prefix(&mut tunnel_command, &edge_ips);
    tunnel_command
        .args([
            "--protocol",
            "http2",
            "--edge-ip-version",
            "4",
            "--no-autoupdate",
            "--logfile",
        ])
        .arg(&log_path)
        .args(["--loglevel", "info"]);
    append_named_tunnel_run_args(&mut tunnel_command, token_file);
    tunnel_command.current_dir(app_dir);
    configure_background_process(&mut tunnel_command);
    let mut tunnel_child = spawn_managed_child(
        &mut tunnel_command,
        process_job,
        "could not start stable public tunnel",
    )?;
    let started_at = Instant::now();
    let mut last_verify = Instant::now() - Duration::from_secs(3);
    while started_at.elapsed() < Duration::from_secs(40) {
        thread::sleep(Duration::from_millis(500));
        match tunnel_child.try_wait() {
            Ok(Some(_)) => return Ok(None),
            Ok(None) => {}
            Err(error) => {
                let _ = tunnel_child.kill();
                let _ = tunnel_child.wait();
                return Err(format!("could not inspect stable tunnel process: {error}"));
            }
        }
        let tunnel_registered = fs::read_to_string(&log_path)
            .map(|log| log.contains("Registered tunnel connection"))
            .unwrap_or(false);
        if last_verify.elapsed() >= Duration::from_secs(2) {
            last_verify = Instant::now();
            if verify_public_tunnel(stable_url) {
                write_log(&format!("verified stable public tunnel: {stable_url}"));
                return Ok(Some((tunnel_child, log_path)));
            }
            if tunnel_registered {
                write_log("stable tunnel registered; waiting for public health check");
            }
        }
    }
    let _ = tunnel_child.kill();
    let _ = tunnel_child.wait();
    Ok(None)
}

fn append_named_tunnel_run_args(command: &mut Command, token_file: &Path) {
    command.args(["run", "--token-file"]).arg(token_file);
}

fn append_quick_tunnel_mode_args(command: &mut Command, log_path: &Path) {
    command
        .args([
            "--url",
            "http://127.0.0.1:8917",
            "--no-chunked-encoding",
            "--protocol",
            "http2",
            "--edge-ip-version",
            "4",
            "--no-autoupdate",
            "--logfile",
        ])
        .arg(log_path)
        .args(["--loglevel", "info"])
        .args(["--metrics", QUICK_TUNNEL_METRICS_BIND_ADDR]);
}

fn resolve_cloudflared_edge_ips() -> Vec<Ipv4Addr> {
    let agents = DohAgents::new();
    let mut resolved = Vec::new();
    for (index, hostname) in CLOUDFLARE_TUNNEL_EDGE_HOSTS.iter().enumerate() {
        let transaction_id = next_dns_transaction_id(index as u16);
        let query = match build_dns_a_query(hostname, transaction_id) {
            Ok(query) => query,
            Err(error) => {
                write_log(&format!(
                    "could not build Cloudflare edge DNS query for {hostname}: {error}"
                ));
                continue;
            }
        };
        let Some(response) = agents.resolve(&query) else {
            write_log(&format!(
                "Cloudflare edge DoH lookup failed for {hostname}; normal cloudflared DNS fallback remains available"
            ));
            continue;
        };
        let addresses = match parse_cloudflare_edge_a_response(hostname, &query, &response) {
            Ok(addresses) => addresses,
            Err(error) => {
                write_log(&format!(
                    "Cloudflare edge DoH response was rejected for {hostname}: {error}"
                ));
                continue;
            }
        };
        if addresses.is_empty() {
            write_log(&format!(
                "Cloudflare edge DoH response contained no approved IPv4 address for {hostname}"
            ));
        }
        for address in addresses {
            if resolved.len() < MAX_STATIC_EDGE_IPS && !resolved.contains(&address) {
                resolved.push(address);
            }
        }
    }
    if resolved.len() < MIN_STATIC_EDGE_IPS {
        write_log(&format!(
            "Cloudflare edge DoH discovery returned fewer than {MIN_STATIC_EDGE_IPS} approved addresses; using normal cloudflared DNS fallback"
        ));
        Vec::new()
    } else {
        write_log(&format!(
            "resolved {} approved Cloudflare Tunnel edge addresses over HTTPS",
            resolved.len()
        ));
        resolved
    }
}

fn next_dns_transaction_id(salt: u16) -> u16 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    (nanos as u16)
        ^ ((nanos >> 16) as u16)
        ^ ((nanos >> 32) as u16)
        ^ (std::process::id() as u16)
        ^ salt.rotate_left(7)
}

fn build_dns_a_query(hostname: &str, transaction_id: u16) -> Result<Vec<u8>, String> {
    let hostname = hostname.trim_end_matches('.');
    if hostname.is_empty() || hostname.len() > 253 || !hostname.is_ascii() {
        return Err("hostname length or encoding is invalid".to_string());
    }
    let labels = hostname.split('.').collect::<Vec<_>>();
    if labels.iter().any(|label| {
        label.is_empty()
            || label.len() > 63
            || !label
                .as_bytes()
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
            || label.starts_with('-')
            || label.ends_with('-')
    }) {
        return Err("hostname contains an invalid DNS label".to_string());
    }

    let mut query = Vec::with_capacity(hostname.len() + 18);
    query.extend_from_slice(&transaction_id.to_be_bytes());
    query.extend_from_slice(&0x0100_u16.to_be_bytes());
    query.extend_from_slice(&1_u16.to_be_bytes());
    query.extend_from_slice(&0_u16.to_be_bytes());
    query.extend_from_slice(&0_u16.to_be_bytes());
    query.extend_from_slice(&0_u16.to_be_bytes());
    for label in labels {
        query.push(label.len() as u8);
        query.extend_from_slice(label.as_bytes());
    }
    query.push(0);
    query.extend_from_slice(&1_u16.to_be_bytes());
    query.extend_from_slice(&1_u16.to_be_bytes());
    if query.len() > DNS_WIRE_MAX_BYTES {
        return Err("DNS query exceeds the safe wire limit".to_string());
    }
    Ok(query)
}

fn build_proxy_aware_doh_agent() -> ureq::Agent {
    build_doh_agent(true)
}

fn build_direct_doh_agent() -> ureq::Agent {
    build_doh_agent(false)
}

fn build_doh_agent(use_proxy_environment: bool) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(use_proxy_environment)
        .https_only(true)
        .redirects(0)
        .timeout_connect(DOH_ATTEMPT_TIMEOUT)
        .timeout(DOH_ATTEMPT_TIMEOUT)
        .build()
}

fn proxy_environment_is_configured() -> bool {
    [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .any(|name| env::var_os(name).is_some_and(|value| !value.is_empty()))
}

fn request_dns_over_https(agent: &ureq::Agent, query: &[u8]) -> Option<Vec<u8>> {
    if !is_valid_dns_query(query) {
        return None;
    }
    let response = agent
        .post(DOH_ENDPOINT)
        .set("Accept", "application/dns-message")
        .set("Content-Type", "application/dns-message")
        .send_bytes(query)
        .ok()?;
    if response.status() != 200 || !is_dns_message_content_type(response.header("Content-Type")) {
        return None;
    }
    if response
        .header("Content-Length")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .is_some_and(|length| length > DNS_WIRE_MAX_BYTES)
    {
        return None;
    }
    let mut body = Vec::with_capacity(512);
    response
        .into_reader()
        .take((DNS_WIRE_MAX_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .ok()?;
    is_valid_dns_response(query, &body).then_some(body)
}

fn is_dns_message_content_type(value: Option<&str>) -> bool {
    value
        .and_then(|content_type| content_type.split(';').next())
        .map(str::trim)
        .is_some_and(|content_type| content_type.eq_ignore_ascii_case("application/dns-message"))
}

fn is_valid_dns_query(query: &[u8]) -> bool {
    (DNS_WIRE_MIN_BYTES..=DNS_WIRE_MAX_BYTES).contains(&query.len())
        && query[2] & 0x80 == 0
        && query[2] & 0x78 == 0
        && u16::from_be_bytes([query[4], query[5]]) == 1
}

fn is_valid_dns_response(query: &[u8], response: &[u8]) -> bool {
    is_valid_dns_query(query)
        && (DNS_WIRE_MIN_BYTES..=DNS_WIRE_MAX_BYTES).contains(&response.len())
        && response[..2] == query[..2]
        && response[2] & 0x80 != 0
        && response[2] & 0x78 == 0
        && response[2] & 0x02 == 0
        && response[3] & 0x0f == 0
        && u16::from_be_bytes([response[4], response[5]]) == 1
}

fn parse_cloudflare_edge_a_response(
    hostname: &str,
    query: &[u8],
    response: &[u8],
) -> Result<Vec<Ipv4Addr>, String> {
    parse_dns_a_response(hostname, query, response, is_cloudflare_tunnel_edge_ip)
}

fn parse_dns_a_response(
    hostname: &str,
    query: &[u8],
    response: &[u8],
    allow_address: impl Fn(Ipv4Addr) -> bool,
) -> Result<Vec<Ipv4Addr>, String> {
    if !is_valid_dns_response(query, response) {
        return Err("DNS response header is invalid or does not match the query".to_string());
    }
    let expected_hostname = hostname.trim_end_matches('.').to_ascii_lowercase();
    let mut query_cursor = DNS_WIRE_MIN_BYTES;
    let query_hostname = read_dns_name(query, &mut query_cursor)?;
    let query_type = take_dns_u16(query, &mut query_cursor)?;
    let query_class = take_dns_u16(query, &mut query_cursor)?;
    if query_cursor != query.len()
        || !query_hostname.eq_ignore_ascii_case(&expected_hostname)
        || query_type != 1
        || query_class != 1
    {
        return Err("DNS query question is not the expected IN A lookup".to_string());
    }

    let answer_count = u16::from_be_bytes([response[6], response[7]]) as usize;
    let authority_count = u16::from_be_bytes([response[8], response[9]]) as usize;
    let additional_count = u16::from_be_bytes([response[10], response[11]]) as usize;
    let total_records = answer_count
        .checked_add(authority_count)
        .and_then(|count| count.checked_add(additional_count))
        .ok_or_else(|| "DNS record count overflow".to_string())?;
    if total_records > DNS_MAX_RECORDS {
        return Err("DNS response contains too many records".to_string());
    }

    let mut cursor = DNS_WIRE_MIN_BYTES;
    let response_hostname = read_dns_name(response, &mut cursor)?;
    let response_type = take_dns_u16(response, &mut cursor)?;
    let response_class = take_dns_u16(response, &mut cursor)?;
    if !response_hostname.eq_ignore_ascii_case(&expected_hostname)
        || response_type != 1
        || response_class != 1
    {
        return Err("DNS response question does not match the requested IN A lookup".to_string());
    }

    let mut addresses = Vec::new();
    for _ in 0..answer_count {
        let (record_type, record_class, data_start, data_end) =
            take_dns_resource_record(response, &mut cursor)?;
        if record_type == 1 && record_class == 1 && data_end - data_start == 4 {
            let address = Ipv4Addr::new(
                response[data_start],
                response[data_start + 1],
                response[data_start + 2],
                response[data_start + 3],
            );
            if allow_address(address)
                && addresses.len() < MAX_STATIC_EDGE_IPS
                && !addresses.contains(&address)
            {
                addresses.push(address);
            }
        }
    }
    for _ in 0..authority_count + additional_count {
        take_dns_resource_record(response, &mut cursor)?;
    }
    if cursor != response.len() {
        return Err("DNS response has unparsed trailing bytes".to_string());
    }
    Ok(addresses)
}

fn read_dns_name(message: &[u8], cursor: &mut usize) -> Result<String, String> {
    if *cursor >= message.len() {
        return Err("DNS name starts outside the message".to_string());
    }
    let mut position = *cursor;
    let mut resume_at = None;
    let mut pointer_targets = Vec::new();
    let mut labels = Vec::new();
    let mut expanded_length = 0_usize;
    loop {
        let length = *message
            .get(position)
            .ok_or_else(|| "DNS name label is truncated".to_string())?;
        if length & 0xc0 == 0xc0 {
            let second = *message
                .get(position + 1)
                .ok_or_else(|| "DNS compression pointer is truncated".to_string())?;
            let target = (((length & 0x3f) as usize) << 8) | second as usize;
            if target >= message.len() || pointer_targets.contains(&target) {
                return Err("DNS compression pointer is invalid or cyclic".to_string());
            }
            if pointer_targets.len() >= DNS_MAX_RECORDS {
                return Err("DNS name contains too many compression pointers".to_string());
            }
            pointer_targets.push(target);
            resume_at.get_or_insert(position + 2);
            position = target;
            continue;
        }
        if length & 0xc0 != 0 {
            return Err("DNS name uses an unsupported label encoding".to_string());
        }
        position += 1;
        if length == 0 {
            *cursor = resume_at.unwrap_or(position);
            break;
        }
        let length = length as usize;
        if length > 63 {
            return Err("DNS label exceeds 63 bytes".to_string());
        }
        let end = position
            .checked_add(length)
            .filter(|end| *end <= message.len())
            .ok_or_else(|| "DNS name label exceeds the message".to_string())?;
        let label = &message[position..end];
        if !label
            .iter()
            .all(|byte| byte.is_ascii_graphic() && *byte != b'.')
        {
            return Err("DNS name contains a non-text label".to_string());
        }
        expanded_length = expanded_length
            .checked_add(length + usize::from(!labels.is_empty()))
            .ok_or_else(|| "DNS name length overflow".to_string())?;
        if expanded_length > 253 || labels.len() >= 127 {
            return Err("expanded DNS name is too long".to_string());
        }
        labels.push(
            std::str::from_utf8(label)
                .map_err(|_| "DNS name label is not UTF-8".to_string())?
                .to_ascii_lowercase(),
        );
        position = end;
    }
    Ok(labels.join("."))
}

fn take_dns_u16(message: &[u8], cursor: &mut usize) -> Result<u16, String> {
    let end = cursor
        .checked_add(2)
        .filter(|end| *end <= message.len())
        .ok_or_else(|| "DNS 16-bit field is truncated".to_string())?;
    let value = u16::from_be_bytes([message[*cursor], message[*cursor + 1]]);
    *cursor = end;
    Ok(value)
}

fn take_dns_resource_record(
    message: &[u8],
    cursor: &mut usize,
) -> Result<(u16, u16, usize, usize), String> {
    read_dns_name(message, cursor)?;
    let record_type = take_dns_u16(message, cursor)?;
    let record_class = take_dns_u16(message, cursor)?;
    let fixed_end = cursor
        .checked_add(4)
        .filter(|end| *end <= message.len())
        .ok_or_else(|| "DNS record TTL is truncated".to_string())?;
    *cursor = fixed_end;
    let data_length = take_dns_u16(message, cursor)? as usize;
    let data_start = *cursor;
    let data_end = data_start
        .checked_add(data_length)
        .filter(|end| *end <= message.len())
        .ok_or_else(|| "DNS record data exceeds the message".to_string())?;
    *cursor = data_end;
    Ok((record_type, record_class, data_start, data_end))
}

fn is_cloudflare_tunnel_edge_ip(address: Ipv4Addr) -> bool {
    matches!(address.octets(), [198, 41, 192, _] | [198, 41, 200, _])
}

fn sanitize_cloudflared_edge_ips(candidates: &[Ipv4Addr]) -> Vec<Ipv4Addr> {
    let mut approved = Vec::new();
    for address in candidates {
        if approved.len() >= MAX_STATIC_EDGE_IPS {
            break;
        }
        if is_cloudflare_tunnel_edge_ip(*address) && !approved.contains(address) {
            approved.push(*address);
        }
    }
    approved
}

fn append_cloudflared_tunnel_prefix(command: &mut Command, candidates: &[Ipv4Addr]) -> usize {
    command.arg("tunnel");
    let approved = sanitize_cloudflared_edge_ips(candidates);
    if approved.len() < MIN_STATIC_EDGE_IPS {
        return 0;
    }
    for address in &approved {
        command
            .arg("--edge")
            .arg(format!("{address}:{CLOUDFLARE_TUNNEL_EDGE_PORT}"));
    }
    approved.len()
}

fn verify_public_tunnel(public_url: &str) -> bool {
    let Some(public_url) = normalize_public_url(public_url) else {
        return false;
    };
    let health_url = format!("{public_url}/health");
    public_tunnel_health_with_fallback(
        || {
            public_tunnel_probe_proxy_modes()
                .into_iter()
                .any(|use_proxy| {
                    let agent = public_health_agent_builder()
                        .try_proxy_from_env(use_proxy)
                        .build();
                    verify_public_health_with_agent(&agent, &health_url)
                })
        },
        || verify_public_tunnel_over_doh(&public_url, &health_url),
    )
}

fn public_tunnel_health_with_fallback(
    standard_probe: impl FnOnce() -> bool,
    dns_probe: impl FnOnce() -> bool,
) -> bool {
    standard_probe() || dns_probe()
}

fn public_health_agent_builder() -> ureq::AgentBuilder {
    ureq::AgentBuilder::new()
        .https_only(true)
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(6))
        .redirects(0)
}

fn verify_public_health_with_agent(agent: &ureq::Agent, health_url: &str) -> bool {
    match agent
        .get(health_url)
        .set("Accept", "application/json")
        .call()
    {
        Ok(response) if (200..=299).contains(&response.status()) => {
            response_has_valid_health_body(response)
        }
        _ => false,
    }
}

fn public_tunnel_dns_host(public_url: &str) -> Option<String> {
    let normalized = normalize_trycloudflare_url(public_url)?;
    let parsed = url::Url::parse(&normalized).ok()?;
    let hostname = parsed.host_str()?;
    let label = hostname.strip_suffix(".trycloudflare.com")?;
    if parsed.port_or_known_default() != Some(443) || label.is_empty() || label.contains('.') {
        return None;
    }
    build_dns_a_query(hostname, 0).ok()?;
    Some(hostname.to_string())
}

fn is_public_dns_address(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !address.is_private()
        && !address.is_loopback()
        && !address.is_link_local()
        && !address.is_documentation()
        && !(a == 0 || a >= 224)
        && !(a == 100 && (64..=127).contains(&b))
        && !(a == 192 && b == 0 && c == 0)
        && !(a == 198 && (b == 18 || b == 19))
}

struct PublicTunnelResolver {
    authority: String,
    addresses: Vec<SocketAddr>,
}

impl ureq::Resolver for PublicTunnelResolver {
    fn resolve(&self, authority: &str) -> std::io::Result<Vec<SocketAddr>> {
        // Restrict this one-request override to the exact validated HTTPS host.
        // The URL and TLS server name remain the original public hostname.
        if authority != self.authority || self.addresses.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "public tunnel DNS override does not match this request",
            ));
        }
        Ok(self.addresses.clone())
    }
}

fn rendezvous_dns_host(endpoint: &str) -> Option<&'static str> {
    let parsed = url::Url::parse(endpoint).ok()?;
    (parsed.scheme() == "https"
        && parsed.host_str() == Some("ntfy.sh")
        && parsed.port_or_known_default() == Some(443)
        && parsed.username().is_empty()
        && parsed.password().is_none())
    .then_some("ntfy.sh")
}

fn resolve_public_dns(hostname: &str) -> Option<PublicTunnelResolver> {
    if hostname != "ntfy.sh" && public_tunnel_dns_host(&format!("https://{hostname}")).is_none() {
        return None;
    }
    let query = build_dns_a_query(hostname, next_dns_transaction_id(2)).ok()?;
    let response = DohAgents::new().resolve(&query)?;
    let addresses =
        parse_dns_a_response(hostname, &query, &response, is_public_dns_address).ok()?;
    if addresses.is_empty() {
        return None;
    }
    Some(PublicTunnelResolver {
        authority: format!("{hostname}:443"),
        addresses: addresses
            .into_iter()
            .map(|ip| SocketAddr::from((ip, 443)))
            .collect(),
    })
}

fn verify_public_tunnel_over_doh(public_url: &str, health_url: &str) -> bool {
    let Some(hostname) = public_tunnel_dns_host(public_url) else {
        return false;
    };
    let Some(resolver) = resolve_public_dns(&hostname) else {
        return false;
    };
    let agent = public_health_agent_builder()
        .try_proxy_from_env(false)
        .resolver(resolver)
        .build();
    // Never treat a DNS answer as proof of health: TLS, HTTP and exact release
    // identity checks are identical to the normal public probe.
    verify_public_health_with_agent(&agent, health_url)
}

fn public_tunnel_probe_proxy_modes() -> [bool; 2] {
    [false, true]
}

fn verify_quick_tunnel_ready(metrics_addr: SocketAddr) -> Option<bool> {
    let ready_url = format!("http://{metrics_addr}/ready");
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout_connect(Duration::from_millis(500))
        .timeout_read(Duration::from_millis(750))
        .redirects(0)
        .build();
    match agent.get(&ready_url).call() {
        Ok(response) => Some((200..=299).contains(&response.status())),
        Err(ureq::Error::Status(_, _)) => Some(false),
        Err(ureq::Error::Transport(_)) => None,
    }
}

fn probe_local_server(server_path: &Path, expected_managed_pid: Option<u32>) -> LocalServerHealth {
    let Some(expected_process_name) = server_path.file_name().and_then(|name| name.to_str()) else {
        return LocalServerHealth::Unavailable;
    };
    let Some(owner_before_request) =
        verified_local_listener_owner(server_path, expected_managed_pid)
    else {
        return LocalServerHealth::Unavailable;
    };
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout_connect(Duration::from_secs(2))
        .timeout_read(Duration::from_secs(3))
        .redirects(0)
        .build();
    let response_health = match agent
        .get("http://127.0.0.1:8917/health")
        .set("Accept", "application/json")
        .call()
    {
        Ok(response) if (200..=299).contains(&response.status()) => {
            if response_has_expected_health_body(
                response,
                expected_process_name,
                owner_before_request,
            ) {
                LocalServerHealth::Healthy
            } else {
                LocalServerHealth::Unavailable
            }
        }
        Err(ureq::Error::Status(503, response)) => {
            if response_has_expected_busy_body(
                response,
                expected_process_name,
                owner_before_request,
            ) {
                LocalServerHealth::Busy
            } else {
                LocalServerHealth::Unavailable
            }
        }
        _ => LocalServerHealth::Unavailable,
    };
    if !response_health.is_ready() {
        return response_health;
    }
    let owner_after_request = verified_local_listener_owner(server_path, expected_managed_pid);
    health_with_stable_listener_owner(
        response_health,
        Some(owner_before_request),
        owner_after_request,
    )
}

fn verified_local_listener_owner(
    server_path: &Path,
    expected_managed_pid: Option<u32>,
) -> Option<u32> {
    let owner_pid = local_tcp_listener_owner_pid(LOCAL_SYNC_PORT)?;
    listener_owner_matches_expected(Some(owner_pid), expected_managed_pid, |pid| {
        external_process_image_matches_server(pid, server_path)
    })
    .then_some(owner_pid)
}

fn listener_owner_matches_expected(
    owner_pid: Option<u32>,
    expected_managed_pid: Option<u32>,
    _external_process_image_matches: impl FnOnce(u32) -> bool,
) -> bool {
    let Some(owner_pid) = owner_pid else {
        return false;
    };
    match expected_managed_pid {
        Some(expected_pid) => owner_pid == expected_pid,
        // A matching executable image is not an ownership proof: another
        // launcher instance may have started the same binary against a
        // different store. Without our exact live Child PID, publishing a
        // tunnel could expose that unrelated listener.
        None => false,
    }
}

fn health_with_stable_listener_owner(
    health: LocalServerHealth,
    owner_before_request: Option<u32>,
    owner_after_request: Option<u32>,
) -> LocalServerHealth {
    if health.is_ready()
        && owner_before_request.is_some()
        && owner_before_request == owner_after_request
    {
        health
    } else {
        LocalServerHealth::Unavailable
    }
}

#[cfg(windows)]
fn local_tcp_listener_owner_pid(port: u16) -> Option<u32> {
    windows_native::tcp_listener_owner_pid(port)
}

#[cfg(not(windows))]
fn local_tcp_listener_owner_pid(_port: u16) -> Option<u32> {
    None
}

#[cfg(windows)]
fn external_process_image_matches_server(pid: u32, server_path: &Path) -> bool {
    let Some(process_image) = windows_native::process_image_path(pid) else {
        return false;
    };
    let Ok(process_image) = fs::canonicalize(process_image) else {
        return false;
    };
    let Ok(server_path) = fs::canonicalize(server_path) else {
        return false;
    };
    windows_native::paths_equal_case_insensitive(&process_image, &server_path)
}

#[cfg(not(windows))]
fn external_process_image_matches_server(_pid: u32, _server_path: &Path) -> bool {
    false
}

fn response_has_expected_health_body(
    response: ureq::Response,
    expected_process_name: &str,
    expected_process_id: u32,
) -> bool {
    read_health_response_body(response)
        .as_deref()
        .map(|body| is_expected_health_response(body, expected_process_name, expected_process_id))
        .unwrap_or(false)
}

fn response_has_expected_busy_body(
    response: ureq::Response,
    expected_process_name: &str,
    expected_process_id: u32,
) -> bool {
    read_health_response_body(response)
        .as_deref()
        .map(|body| is_expected_busy_response(body, expected_process_name, expected_process_id))
        .unwrap_or(false)
}

fn response_has_valid_health_body(response: ureq::Response) -> bool {
    read_health_response_body(response)
        .as_deref()
        .map(is_valid_health_response)
        .unwrap_or(false)
}

fn read_health_response_body(response: ureq::Response) -> Option<String> {
    let mut bytes = Vec::new();
    if response
        .into_reader()
        .take(HEALTH_RESPONSE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > HEALTH_RESPONSE_MAX_BYTES
    {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn is_valid_health_response(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            Some(
                value.get("ok")?.as_bool()?
                    && value.get("mode")?.as_str()? == "health"
                    && value.get("productId")?.as_str()? == product_identity::PRODUCT.internal_id
                    && value.get("serviceRole")?.as_str()? == "sync_server"
                    && value.get("syncProtocolVersion")?.as_i64()?
                        == product_identity::PRODUCT.sync_protocol_version
                    && value.get("serverBuildId")?.as_str()? == SYNC_SERVER_BUILD_ID
                    && value.get("serverGitCommit")?.as_str()?
                        == product_identity::BUILD_GIT_COMMIT
                    && value.get("serverSourceSnapshotSha256")?.as_str()?
                        == product_identity::BUILD_SOURCE_SNAPSHOT_SHA256
                    && value.get("serverProcessId")?.as_u64()? > 0,
            )
        })
        .unwrap_or(false)
}

fn is_expected_health_response(
    body: &str,
    expected_process_name: &str,
    expected_process_id: u32,
) -> bool {
    is_expected_local_server_response(
        body,
        expected_process_name,
        expected_process_id,
        true,
        "health",
    )
}

fn is_expected_busy_response(
    body: &str,
    expected_process_name: &str,
    expected_process_id: u32,
) -> bool {
    is_expected_local_server_response(
        body,
        expected_process_name,
        expected_process_id,
        false,
        "busy",
    )
}

fn is_expected_local_server_response(
    body: &str,
    expected_process_name: &str,
    expected_process_id: u32,
    expected_ok: bool,
    expected_mode: &str,
) -> bool {
    let Some(expected_build_id) = sync_server_build_id_from_process_name(expected_process_name)
    else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            Some(
                value.get("ok")?.as_bool()? == expected_ok
                    && value.get("mode")?.as_str()? == expected_mode
                    && value.get("productId")?.as_str()?
                        == gridtimer_native::product_identity::PRODUCT.internal_id
                    && value.get("serviceRole")?.as_str()? == "sync_server"
                    && value.get("syncProtocolVersion")?.as_i64()?
                        == gridtimer_native::product_identity::PRODUCT.sync_protocol_version
                    && value.get("serverBuildId")?.as_str()? == expected_build_id
                    && value.get("serverGitCommit")?.as_str()?
                        == product_identity::BUILD_GIT_COMMIT
                    && value.get("serverSourceSnapshotSha256")?.as_str()?
                        == product_identity::BUILD_SOURCE_SNAPSHOT_SHA256
                    && value.get("serverProcessId")?.as_u64()? == expected_process_id as u64
                    && value
                        .get("serverProcessName")?
                        .as_str()?
                        .eq_ignore_ascii_case(expected_process_name),
            )
        })
        .unwrap_or(false)
}

fn sync_server_build_id_from_process_name(process_name: &str) -> Option<&str> {
    let build_id = process_name
        .strip_prefix("grid_timer_sync_server_v")?
        .strip_suffix(".exe")?;
    (!build_id.is_empty()
        && !build_id.chars().any(char::is_whitespace)
        && !build_id.chars().any(char::is_control))
    .then_some(build_id)
}

fn configure_background_process(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn spawn_managed_child(
    command: &mut Command,
    process_job: &ProcessJob,
    context: &str,
) -> Result<Child, String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("{context}: {error}"))?;
    if let Err(error) = process_job.assign(&child) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!(
            "{context}: could not attach child to supervisor: {error}"
        ));
    }
    Ok(child)
}

fn reap_exited_child(child_slot: &mut Option<Child>) -> Result<(), String> {
    let exited = match child_slot.as_mut() {
        Some(child) => child
            .try_wait()
            .map_err(|error| format!("could not inspect sync server process: {error}"))?
            .is_some(),
        None => false,
    };
    if exited {
        child_slot.take();
    }
    Ok(())
}

fn terminate_child(child_slot: &mut Option<Child>) {
    let Some(mut child) = child_slot.take() else {
        return;
    };
    match child.try_wait() {
        Ok(Some(_)) => {}
        _ => {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn local_sync_port_is_open() -> bool {
    let Ok(address) = LOCAL_SYNC_ADDR.parse::<SocketAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&address, Duration::from_millis(500)).is_ok()
}

fn prepare_tunnel_log(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "tunnel log path has no valid file name".to_string())?;
    let previous = path.with_file_name(format!("{file_name}.previous"));
    if let Err(error) = fs::remove_file(&previous) {
        if error.kind() != std::io::ErrorKind::NotFound {
            return Err(format!("could not clear previous tunnel log: {error}"));
        }
    }
    let length = path.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    if length > TUNNEL_LOG_MAX_BYTES {
        fs::remove_file(path)
            .map_err(|error| format!("could not clear oversized tunnel log: {error}"))
    } else {
        fs::rename(path, previous).map_err(|error| {
            format!("could not rotate tunnel log; another tunnel may still own it: {error}")
        })
    }
}

fn read_new_complete_log_lines(path: &Path, offset: &mut u64) -> Result<String, String> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(format!("could not open tunnel log: {error}")),
    };
    let length = file
        .metadata()
        .map_err(|error| format!("could not inspect tunnel log: {error}"))?
        .len();
    if length < *offset {
        return Err("tunnel log was truncated or replaced while its child was active".to_string());
    }
    let unread = length.saturating_sub(*offset);
    if unread > TUNNEL_LOG_MAX_BYTES {
        return Err("unread tunnel log content exceeded the size limit".to_string());
    }
    file.seek(SeekFrom::Start(*offset))
        .map_err(|error| format!("could not seek tunnel log: {error}"))?;
    let mut bytes = Vec::with_capacity(unread as usize);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("could not read tunnel log: {error}"))?;
    let complete_length = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    if complete_length == 0 {
        return Ok(String::new());
    }
    *offset = (*offset).saturating_add(complete_length as u64);
    Ok(String::from_utf8_lossy(&bytes[..complete_length]).into_owned())
}

fn contains_terminal_quick_tunnel_invalidation(lines: &str) -> bool {
    lines.lines().any(|line| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            return false;
        };
        let message = value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let error = value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        message
            .trim()
            .eq_ignore_ascii_case("Register tunnel error from server side")
            && error
                .trim()
                .eq_ignore_ascii_case("Unauthorized: Tunnel not found")
    })
}

fn prepare_server_output_logs() -> Result<(PathBuf, PathBuf), String> {
    let directory = launcher_runtime_directory();
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create sync server log directory: {error}"))?;
    let stdout_path = directory.join("sync_server_stdout.log");
    let stderr_path = directory.join("sync_server_stderr.log");
    for path in [&stdout_path, &stderr_path] {
        rotate_log_files_if_needed(path, SERVER_LOG_MAX_BYTES, SERVER_LOG_BACKUPS, 1)
            .map_err(|error| format!("could not rotate {}: {error}", path.display()))?;
    }
    Ok((stdout_path, stderr_path))
}

fn attach_server_output_logs(
    child: &mut Child,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
) -> Result<(), String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sync server stdout pipe was not available".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "sync server stderr pipe was not available".to_string())?;
    thread::Builder::new()
        .name("sync-server-stdout-log".to_string())
        .spawn(move || pump_bounded_output(stdout, stdout_path))
        .map_err(|error| format!("could not start sync server stdout logger: {error}"))?;
    thread::Builder::new()
        .name("sync-server-stderr-log".to_string())
        .spawn(move || pump_bounded_output(stderr, stderr_path))
        .map_err(|error| format!("could not start sync server stderr logger: {error}"))?;
    Ok(())
}

fn pump_bounded_output<R: Read>(mut reader: R, path: PathBuf) {
    let mut buffer = [0_u8; 8_192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(length) => {
                let _ = append_bounded_log_chunk(&path, &buffer[..length]);
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

fn append_bounded_log_chunk(path: &Path, chunk: &[u8]) -> Result<(), String> {
    rotate_log_files_if_needed(
        path,
        SERVER_LOG_MAX_BYTES,
        SERVER_LOG_BACKUPS,
        chunk.len() as u64,
    )?;
    let mut output = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    output.write_all(chunk).map_err(|error| error.to_string())
}

#[cfg(windows)]
mod windows_native {
    use super::*;
    use std::ffi::c_void;
    use std::ffi::OsString;
    use std::io;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut c_void;

    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    const NO_ERROR: u32 = 0;
    const AF_INET: u32 = 2;
    const TCP_TABLE_OWNER_PID_LISTENER: u32 = 3;
    const MIB_TCP_STATE_LISTEN: u32 = 2;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x0000_1000;
    const MAX_PROCESS_IMAGE_PATH_CHARS: usize = 32_768;
    const CSTR_EQUAL: i32 = 2;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS: i32 = 9;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
    const MOVE_FILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVE_FILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: u32,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[derive(Clone, Copy)]
    #[repr(C)]
    struct MibTcpRowOwnerPid {
        state: u32,
        local_addr: u32,
        local_port: u32,
        remote_addr: u32,
        remote_port: u32,
        owning_pid: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "CreateJobObjectW"]
        fn create_job_object(job_attributes: *const c_void, name: *const u16) -> Handle;
        #[link_name = "SetInformationJobObject"]
        fn set_information_job_object(
            job: Handle,
            information_class: i32,
            information: *const c_void,
            information_length: u32,
        ) -> i32;
        #[link_name = "AssignProcessToJobObject"]
        fn assign_process_to_job_object(job: Handle, process: Handle) -> i32;
        #[link_name = "MoveFileExW"]
        fn move_file_ex(existing: *const u16, new: *const u16, flags: u32) -> i32;
        #[link_name = "OpenProcess"]
        fn open_process(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        #[link_name = "QueryFullProcessImageNameW"]
        fn query_full_process_image_name(
            process: Handle,
            flags: u32,
            executable_name: *mut u16,
            size: *mut u32,
        ) -> i32;
        #[link_name = "CompareStringOrdinal"]
        fn compare_string_ordinal(
            left: *const u16,
            left_length: i32,
            right: *const u16,
            right_length: i32,
            ignore_case: i32,
        ) -> i32;
        #[link_name = "CloseHandle"]
        fn close_handle(handle: Handle) -> i32;
    }

    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        #[link_name = "GetExtendedTcpTable"]
        fn get_extended_tcp_table(
            table: *mut c_void,
            size: *mut u32,
            order: i32,
            address_family: u32,
            table_class: u32,
            reserved: u32,
        ) -> u32;
    }

    pub(super) fn tcp_listener_owner_pid(port: u16) -> Option<u32> {
        let mut required_size = 0_u32;
        let initial_status = unsafe {
            get_extended_tcp_table(
                std::ptr::null_mut(),
                &mut required_size,
                0,
                AF_INET,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if initial_status != ERROR_INSUFFICIENT_BUFFER || required_size < size_of::<u32>() as u32 {
            return None;
        }

        for _ in 0..3 {
            let word_count = usize::try_from(required_size)
                .ok()?
                .saturating_add(size_of::<u32>() - 1)
                / size_of::<u32>();
            let mut table = vec![0_u32; word_count];
            let mut actual_size = required_size;
            let status = unsafe {
                get_extended_tcp_table(
                    table.as_mut_ptr().cast(),
                    &mut actual_size,
                    0,
                    AF_INET,
                    TCP_TABLE_OWNER_PID_LISTENER,
                    0,
                )
            };
            if status == ERROR_INSUFFICIENT_BUFFER {
                required_size = actual_size;
                continue;
            }
            if status != NO_ERROR
                || actual_size < size_of::<u32>() as u32
                || usize::try_from(actual_size).ok()? > table.len() * size_of::<u32>()
            {
                return None;
            }

            let entry_count = table[0] as usize;
            let rows_size = entry_count.checked_mul(size_of::<MibTcpRowOwnerPid>())?;
            let required_table_size = size_of::<u32>().checked_add(rows_size)?;
            if required_table_size > usize::try_from(actual_size).ok()? {
                return None;
            }

            let rows = table.as_ptr().cast::<u8>();
            let mut matching_owner = None;
            for index in 0..entry_count {
                let row_offset = size_of::<u32>()
                    .checked_add(index.checked_mul(size_of::<MibTcpRowOwnerPid>())?)?;
                let row = unsafe {
                    std::ptr::read_unaligned(rows.add(row_offset).cast::<MibTcpRowOwnerPid>())
                };
                let local_port = u16::from_be(row.local_port as u16);
                if row.state != MIB_TCP_STATE_LISTEN || local_port != port || row.owning_pid == 0 {
                    continue;
                }
                match matching_owner {
                    None => matching_owner = Some(row.owning_pid),
                    Some(existing) if existing == row.owning_pid => {}
                    Some(_) => return None,
                }
            }
            return matching_owner;
        }
        None
    }

    pub(super) fn process_image_path(pid: u32) -> Option<PathBuf> {
        if pid == 0 {
            return None;
        }
        let process = unsafe { open_process(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return None;
        }
        let mut buffer = vec![0_u16; MAX_PROCESS_IMAGE_PATH_CHARS];
        let mut length = MAX_PROCESS_IMAGE_PATH_CHARS as u32;
        let queried =
            unsafe { query_full_process_image_name(process, 0, buffer.as_mut_ptr(), &mut length) };
        unsafe {
            close_handle(process);
        }
        let length = usize::try_from(length).ok()?;
        if queried == 0 || length == 0 || length > buffer.len() {
            return None;
        }
        Some(PathBuf::from(OsString::from_wide(&buffer[..length])))
    }

    pub(super) fn paths_equal_case_insensitive(left: &Path, right: &Path) -> bool {
        let left = left.as_os_str().encode_wide().collect::<Vec<_>>();
        let right = right.as_os_str().encode_wide().collect::<Vec<_>>();
        let (Ok(left_length), Ok(right_length)) =
            (i32::try_from(left.len()), i32::try_from(right.len()))
        else {
            return false;
        };
        unsafe {
            compare_string_ordinal(left.as_ptr(), left_length, right.as_ptr(), right_length, 1)
                == CSTR_EQUAL
        }
    }

    pub(super) struct ProcessJob {
        handle: Handle,
    }

    impl ProcessJob {
        pub(super) fn new() -> Result<Self, String> {
            let handle = unsafe { create_job_object(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(io::Error::last_os_error().to_string());
            }
            let mut information: JobObjectExtendedLimitInformation = unsafe { zeroed() };
            information.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                set_information_job_object(
                    handle,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION_CLASS,
                    (&information as *const JobObjectExtendedLimitInformation).cast(),
                    size_of::<JobObjectExtendedLimitInformation>() as u32,
                )
            };
            if configured == 0 {
                let error = io::Error::last_os_error().to_string();
                unsafe {
                    close_handle(handle);
                }
                return Err(error);
            }
            Ok(Self { handle })
        }

        pub(super) fn assign(&self, child: &Child) -> Result<(), String> {
            let assigned =
                unsafe { assign_process_to_job_object(self.handle, child.as_raw_handle().cast()) };
            if assigned == 0 {
                Err(io::Error::last_os_error().to_string())
            } else {
                Ok(())
            }
        }
    }

    impl Drop for ProcessJob {
        fn drop(&mut self) {
            unsafe {
                close_handle(self.handle);
            }
        }
    }

    pub(super) struct SingleInstanceGuard {
        _ownership: gridtimer_native::runtime::named_mutex::NamedMutexGuard,
    }

    impl SingleInstanceGuard {
        pub(super) fn acquire() -> Result<Option<Self>, String> {
            Self::acquire_for_name(product_identity::SYNC_SUPERVISOR_MUTEX_NAME)
        }

        pub(super) fn acquire_for_name(mutex_name: &str) -> Result<Option<Self>, String> {
            gridtimer_native::runtime::named_mutex::NamedMutexGuard::try_acquire(mutex_name)
                .map(|guard| {
                    guard.map(|ownership| Self {
                        _ownership: ownership,
                    })
                })
                .map_err(|error| error.to_string())
        }
    }

    pub(super) fn replace_file_atomically(source: &Path, destination: &Path) -> Result<(), String> {
        let source = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let replaced = unsafe {
            move_file_ex(
                source.as_ptr(),
                destination.as_ptr(),
                MOVE_FILE_REPLACE_EXISTING | MOVE_FILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            Err(io::Error::last_os_error().to_string())
        } else {
            Ok(())
        }
    }
}

#[cfg(windows)]
use windows_native::{ProcessJob, SingleInstanceGuard};

#[cfg(windows)]
fn replace_file_atomically(source: &Path, destination: &Path) -> Result<(), String> {
    windows_native::replace_file_atomically(source, destination)
}

#[cfg(not(windows))]
struct ProcessJob;

#[cfg(not(windows))]
impl ProcessJob {
    fn new() -> Result<Self, String> {
        Ok(Self)
    }

    fn assign(&self, _child: &Child) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(not(windows))]
struct SingleInstanceGuard;

#[cfg(not(windows))]
impl SingleInstanceGuard {
    fn acquire() -> Result<Option<Self>, String> {
        Ok(Some(Self))
    }
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(source, destination).map_err(|error| error.to_string())
}

fn extract_trycloudflare_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|part| {
            part.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '|' | '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            })
        })
        .find_map(normalize_trycloudflare_url)
}

fn extract_quick_tunnel_metrics_addr(lines: &str) -> Option<SocketAddr> {
    lines.lines().find_map(|line| {
        let value = serde_json::from_str::<serde_json::Value>(line.trim()).ok()?;
        let message = value.get("message")?.as_str()?;
        let address = message
            .strip_prefix("Starting metrics server on ")?
            .strip_suffix("/metrics")?
            .parse::<SocketAddr>()
            .ok()?;
        (address.ip().is_loopback() && address.port() != 0).then_some(address)
    })
}

fn configured_stable_public_url(app_dir: &Path) -> Option<String> {
    env::var("GRID_TIMER_STABLE_PUBLIC_SERVER_URL")
        .ok()
        .and_then(|value| normalize_public_url(&value))
        .or_else(|| {
            env::var("GRID_TIMER_PUBLIC_SERVER_URL")
                .ok()
                .and_then(|value| normalize_public_url(&value))
        })
        .or_else(|| {
            configured_stable_public_url_from_files(
                &launcher_runtime_directory().join(STABLE_PUBLIC_URL_FILE_NAME),
                &app_dir.join("tmp").join(STABLE_PUBLIC_URL_FILE_NAME),
            )
        })
}

fn configured_stable_public_url_from_files(
    local_path: &Path,
    legacy_path: &Path,
) -> Option<String> {
    if let Some(url) = read_trimmed_file(local_path).and_then(|value| normalize_public_url(&value))
    {
        return Some(url);
    }
    let legacy_url =
        read_trimmed_file(legacy_path).and_then(|value| normalize_public_url(&value))?;
    if let Err(error) = atomic_write_text(local_path, &legacy_url) {
        write_log(&format!(
            "could not migrate stable public URL into the persistent launcher directory: {error}"
        ));
    }
    Some(legacy_url)
}

fn configured_cloudflared_tunnel_token_file(app_dir: &Path) -> Option<PathBuf> {
    let local_path = launcher_runtime_directory().join(CLOUDFLARED_TOKEN_FILE_NAME);
    if let Some(token) = env::var("GRID_TIMER_CLOUDFLARED_TUNNEL_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return persist_environment_tunnel_token(&local_path, &token);
    }
    let legacy_path = app_dir.join("tmp").join(CLOUDFLARED_TOKEN_FILE_NAME);
    configured_cloudflared_tunnel_token_file_from_files(&local_path, &legacy_path)
}

fn configured_cloudflared_tunnel_token_file_from_files(
    local_path: &Path,
    legacy_path: &Path,
) -> Option<PathBuf> {
    if read_trimmed_file(local_path).is_some() {
        return Some(local_path.to_path_buf());
    }
    let legacy_token = read_trimmed_file(legacy_path)?;
    match atomic_write_text(local_path, &legacy_token) {
        Ok(()) => Some(local_path.to_path_buf()),
        Err(error) => {
            write_log(&format!(
                "could not migrate the Cloudflare token into the persistent launcher directory: {error}; using the legacy token file"
            ));
            Some(legacy_path.to_path_buf())
        }
    }
}

fn persist_environment_tunnel_token(local_path: &Path, token: &str) -> Option<PathBuf> {
    match atomic_write_text(local_path, token) {
        Ok(()) => Some(local_path.to_path_buf()),
        Err(error) => {
            write_log(&format!(
                "could not persist the configured Cloudflare token for --token-file use: {error}"
            ));
            None
        }
    }
}

fn read_trimmed_file(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_trycloudflare_url(value: &str) -> Option<String> {
    let normalized = normalize_public_url(value)?;
    let remainder = normalized.strip_prefix("https://")?;
    let authority = remainder.split('/').next()?.to_ascii_lowercase();
    if authority.ends_with(".trycloudflare.com")
        && !authority.contains(':')
        && !remainder.contains('/')
    {
        Some(normalized)
    } else {
        None
    }
}

fn atomic_write_text(path: &Path, value: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "public URL file has no parent directory".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = parent.join(format!(
        ".sync_public_server_url.{}.{}.tmp",
        std::process::id(),
        nonce
    ));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| error.to_string())?;
        file.write_all(value.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    replace_file_atomically(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        error
    })
}

fn remove_public_url_file(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            write_log(&format!("could not clear stale public URL: {error}"));
        }
    }
}

const STARTUP_TASK_SCRIPT: &str = include_str!("../sync_autostart.ps1");

#[cfg(windows)]
fn startup_task_command(registration: &StartupRegistration) -> Command {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        STARTUP_TASK_SCRIPT
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut command = Command::new("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
        &encoded,
    ]);
    command.env("GRID_TIMER_STARTUP_EXE", &registration.executable);
    command.env(
        "GRID_TIMER_STARTUP_ARGUMENTS",
        if registration.stable_entry {
            "--ensure-sync"
        } else {
            ""
        },
    );
    configure_background_process(&mut command);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command
}

#[cfg(windows)]
fn ensure_startup_task(registration: &StartupRegistration) {
    match startup_task_command(registration).output() {
        Ok(output) if output.status.success() => {
            remove_hkcu_startup_fallback();
            write_log("verified current-user logon and one-minute recovery task through the stable desktop entry");
        }
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1000)
                .collect::<String>();
            write_log(&format!(
                "could not register current-user recovery task: {} {detail}",
                output.status
            ));
            ensure_hkcu_startup_fallback(&registration.command, registration.stable_entry);
        }
        Err(error) => {
            write_log(&format!(
                "could not register current-user recovery task: {error}"
            ));
            ensure_hkcu_startup_fallback(&registration.command, registration.stable_entry);
        }
    }
}

#[cfg(windows)]
fn ensure_hkcu_startup_fallback(command_value: &str, stable_entry: bool) {
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "GridTimerSyncService";
    let mut command = Command::new("reg.exe");
    command.args([
        "ADD",
        RUN_KEY,
        "/v",
        VALUE_NAME,
        "/t",
        "REG_SZ",
        "/d",
        command_value,
        "/f",
    ]);
    configure_background_process(&mut command);
    match command.status() {
        Ok(status) if status.success() => write_log(if stable_entry {
            "HKCU startup fallback is active through the stable desktop entry"
        } else {
            "HKCU startup fallback is active through the versioned compatibility entry"
        }),
        Ok(status) => write_log(&format!(
            "could not create HKCU startup fallback: exit code {:?}",
            status.code()
        )),
        Err(error) => write_log(&format!("could not create HKCU startup fallback: {error}")),
    }
}

#[cfg(windows)]
fn remove_hkcu_startup_fallback() {
    const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
    let mut command = Command::new("reg.exe");
    command.args(["DELETE", RUN_KEY, "/v", "GridTimerSyncService", "/f"]);
    configure_background_process(&mut command);
    let _ = command.status();
}

fn quoted_executable_command(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn startup_registration_plan(
    versioned_launcher_path: &Path,
    registration_disabled: bool,
) -> StartupRegistrationPlan {
    if registration_disabled {
        return StartupRegistrationPlan::default();
    }
    if let Some(stable_entry_path) = stable_desktop_entry_path(versioned_launcher_path) {
        return StartupRegistrationPlan {
            before_supervisor_lock: Some(StartupRegistration {
                command: format!(
                    "{} {}",
                    quoted_executable_command(&stable_entry_path),
                    STABLE_SYNC_ENTRY_ARGUMENT
                ),
                executable: stable_entry_path,
                stable_entry: true,
            }),
            after_supervisor_lock: None,
        };
    }
    StartupRegistrationPlan {
        before_supervisor_lock: None,
        after_supervisor_lock: Some(StartupRegistration {
            command: quoted_executable_command(versioned_launcher_path),
            executable: versioned_launcher_path.to_path_buf(),
            stable_entry: false,
        }),
    }
}

fn stable_desktop_entry_path(versioned_launcher_path: &Path) -> Option<PathBuf> {
    let current_directory = versioned_launcher_path.parent()?;
    if !path_file_name_eq(current_directory, "current") {
        return None;
    }
    if !is_real_directory(current_directory) {
        return None;
    }
    let release_directory = current_directory.parent()?;
    if !path_file_name_eq(release_directory, "release_artifacts") {
        return None;
    }
    if !is_real_directory(release_directory) {
        return None;
    }
    let entry_directory = release_directory.join(STABLE_DESKTOP_ENTRY_DIRECTORY);
    if !is_real_directory(&entry_directory) {
        return None;
    }
    let stable_entry_path = entry_directory.join(STABLE_DESKTOP_ENTRY_FILE_NAME);
    let metadata = fs::symlink_metadata(&stable_entry_path).ok()?;
    if !metadata.file_type().is_file()
        || metadata_is_reparse_point(&metadata)
        || !is_valid_pe_file(&stable_entry_path, metadata.len())
    {
        return None;
    }
    Some(stable_entry_path)
}

fn is_real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).ok().is_some_and(|metadata| {
        metadata.file_type().is_dir() && !metadata_is_reparse_point(&metadata)
    })
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_attributes() & 0x0000_0400 != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn is_valid_pe_file(path: &Path, length: u64) -> bool {
    if length < 68 {
        return false;
    }
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut dos_header = [0_u8; 64];
    if file.read_exact(&mut dos_header).is_err() || &dos_header[..2] != b"MZ" {
        return false;
    }
    let Ok(offset_bytes) = dos_header[60..64].try_into() else {
        return false;
    };
    let pe_offset = u32::from_le_bytes(offset_bytes) as u64;
    if pe_offset > length.saturating_sub(4) || file.seek(SeekFrom::Start(pe_offset)).is_err() {
        return false;
    }
    let mut signature = [0_u8; 4];
    file.read_exact(&mut signature).is_ok() && &signature == b"PE\0\0"
}

fn path_file_name_eq(path: &Path, expected: &str) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case(expected))
}

#[cfg(not(windows))]
fn ensure_startup_task(_registration: &StartupRegistration) {}

fn normalize_public_url(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/').to_string();
    let remainder = trimmed.strip_prefix("https://")?;
    let authority = remainder.split('/').next()?;
    if authority.is_empty()
        || authority.contains('@')
        || authority.starts_with(':')
        || trimmed.chars().any(char::is_whitespace)
        || trimmed.chars().any(char::is_control)
        || trimmed.contains('\\')
        || trimmed.contains('?')
        || trimmed.contains('#')
    {
        None
    } else {
        Some(trimmed)
    }
}

fn find_sync_server(app_dir: &Path) -> Result<PathBuf, String> {
    let expectation = sync_server_expectation()?;
    let candidate = app_dir.join(expectation.file_name);
    verify_sync_server_candidate(&candidate, app_dir, expectation)?;
    Ok(candidate)
}

fn sync_server_expectation() -> Result<SyncServerExpectation, String> {
    match (
        EXPECTED_SYNC_SERVER_SIZE,
        EXPECTED_SYNC_SERVER_SHA256,
        EXPECTED_SYNC_SERVER_BINDING_V1,
    ) {
        (Some(size), Some(sha256), Some(binding)) => {
            let expected_size = size
                .parse::<u64>()
                .map_err(|_| "launcher contains an invalid sync-server size".to_string())?;
            let expected_binding = executable_integrity_binding_marker(
                "sync_server",
                expected_size,
                &sha256.to_ascii_lowercase(),
            );
            if expected_size == 0
                || !valid_sha256_hex(sha256)
                || binding != expected_binding
            {
                return Err(
                    "launcher contains invalid sync-server integrity metadata".to_string(),
                );
            }
            Ok(SyncServerExpectation {
                file_name: product_identity::SYNC_SERVER_FILE_NAME,
                expected_size: Some(expected_size),
                expected_sha256: Some(sha256),
            })
        }
        (None, None, None) if cfg!(debug_assertions) => Ok(SyncServerExpectation {
            file_name: "timer_sync_server.exe",
            expected_size: None,
            expected_sha256: None,
        }),
        _ => Err(
            "release launcher is missing its sync-server integrity binding; rebuild with the formal packager"
                .to_string(),
        ),
    }
}

fn executable_integrity_binding_marker(role: &str, size: u64, sha256: &str) -> String {
    format!("gridtimer-executable-binding-v1|{role}|{size}|{sha256}")
}

fn verify_sync_server_file(server_path: &Path, app_dir: &Path) -> Result<(), String> {
    verify_sync_server_candidate(server_path, app_dir, sync_server_expectation()?)
}

fn verify_sync_server_candidate(
    candidate: &Path,
    app_dir: &Path,
    expectation: SyncServerExpectation,
) -> Result<(), String> {
    if !path_file_name_eq(candidate, expectation.file_name) {
        return Err("sync server does not have the exact expected file name".to_string());
    }
    let metadata = fs::symlink_metadata(candidate)
        .map_err(|error| format!("sync server is missing at {}: {error}", candidate.display()))?;
    if !metadata.file_type().is_file() || metadata_is_reparse_point(&metadata) {
        return Err(
            "sync server must be a real regular file in the launcher directory".to_string(),
        );
    }
    let canonical_app_dir = fs::canonicalize(app_dir)
        .map_err(|error| format!("could not resolve launcher directory: {error}"))?;
    let canonical_candidate = fs::canonicalize(candidate)
        .map_err(|error| format!("could not resolve sync server: {error}"))?;
    let Some(candidate_parent) = canonical_candidate.parent() else {
        return Err("sync server path has no parent directory".to_string());
    };
    if !paths_equal_for_install(candidate_parent, &canonical_app_dir) {
        return Err("sync server resolves outside the launcher directory".to_string());
    }
    if !is_valid_pe_file(&canonical_candidate, metadata.len()) {
        return Err("sync server is not a valid Windows PE executable".to_string());
    }
    if let Some(expected_size) = expectation.expected_size {
        if metadata.len() != expected_size {
            return Err(format!(
                "sync server size mismatch: expected {expected_size}, found {}",
                metadata.len()
            ));
        }
    }
    if let Some(expected_sha256) = expectation.expected_sha256 {
        let (actual_size, actual_sha256) = sha256_file_hex(&canonical_candidate)?;
        if Some(actual_size) != expectation.expected_size
            || !actual_sha256.eq_ignore_ascii_case(expected_sha256)
        {
            return Err("sync server SHA-256 verification failed".to_string());
        }
    }
    Ok(())
}

fn sha256_file_hex(path: &Path) -> Result<(u64, String), String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("could not open executable for hashing: {error}"))?;
    let mut digest = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("could not hash executable: {error}"))?;
        if read == 0 {
            break;
        }
        size = size
            .checked_add(read as u64)
            .ok_or_else(|| "executable size overflow while hashing".to_string())?;
        digest.update(&buffer[..read]);
    }
    if size == 0 {
        return Err("executable is empty".to_string());
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(windows)]
fn paths_equal_for_install(left: &Path, right: &Path) -> bool {
    windows_native::paths_equal_case_insensitive(left, right)
}

#[cfg(not(windows))]
fn paths_equal_for_install(left: &Path, right: &Path) -> bool {
    left == right
}

fn ensure_firewall_rule(server_path: &Path) {
    if firewall_rule_exists() {
        return;
    }
    if try_add_firewall_rule(server_path) {
        write_log("firewall rule for TCP 8917 is active");
        return;
    }
    write_log("firewall rule is missing; no interactive elevation was requested");
}

fn firewall_rule_exists() -> bool {
    let mut tcp_command = Command::new("netsh");
    tcp_command.args([
        "advfirewall",
        "firewall",
        "show",
        "rule",
        "name=Grid Timer Sync 8917",
    ]);
    configure_background_process(&mut tcp_command);
    let tcp_ok = tcp_command
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let mut udp_command = Command::new("netsh");
    udp_command.args([
        "advfirewall",
        "firewall",
        "show",
        "rule",
        "name=Grid Timer Sync Discovery 8918",
    ]);
    configure_background_process(&mut udp_command);
    let udp_ok = udp_command
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    tcp_ok && udp_ok
}

fn try_add_firewall_rule(server_path: &Path) -> bool {
    let mut port_rule_command = Command::new("netsh");
    port_rule_command.args([
        "advfirewall",
        "firewall",
        "add",
        "rule",
        "name=Grid Timer Sync 8917",
        "dir=in",
        "action=allow",
        "protocol=TCP",
        "localport=8917",
        "profile=private",
    ]);
    configure_background_process(&mut port_rule_command);
    let port_rule = port_rule_command
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let mut discovery_rule_command = Command::new("netsh");
    discovery_rule_command.args([
        "advfirewall",
        "firewall",
        "add",
        "rule",
        "name=Grid Timer Sync Discovery 8918",
        "dir=in",
        "action=allow",
        "protocol=UDP",
        "localport=8918",
        "profile=private",
    ]);
    configure_background_process(&mut discovery_rule_command);
    let discovery_rule = discovery_rule_command
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    let mut program_rule_command = Command::new("netsh");
    program_rule_command
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            "name=Grid Timer Sync Server",
            "dir=in",
            "action=allow",
        ])
        .arg(format!("program={}", server_path.display()))
        .args(["enable=yes", "profile=private"]);
    configure_background_process(&mut program_rule_command);
    let program_rule = program_rule_command
        .status()
        .map(|status| status.success())
        .unwrap_or(false);
    port_rule && discovery_rule && program_rule
}

fn launcher_runtime_directory() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("GridTimerSync")
}

fn write_log(message: &str) {
    let base_dir = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base_dir.join("GridTimerSync");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("sync_launcher.log");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let line = format!("{timestamp} {message}\n");
    if rotate_log_files_if_needed(
        &path,
        LAUNCHER_LOG_MAX_BYTES,
        LAUNCHER_LOG_BACKUPS,
        line.len() as u64,
    )
    .is_err()
    {
        return;
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(line.as_bytes());
    }
}

fn rotate_log_files_if_needed(
    path: &Path,
    max_bytes: u64,
    backups: usize,
    incoming: u64,
) -> Result<(), String> {
    if incoming > max_bytes {
        return Err("single log entry exceeds the configured log size limit".to_string());
    }
    for index in 1..=backups {
        let backup = numbered_log_path(path, index);
        if backup
            .metadata()
            .map(|metadata| metadata.len() > max_bytes)
            .unwrap_or(false)
        {
            fs::remove_file(&backup).map_err(|error| error.to_string())?;
        }
    }
    let mut current_length = path.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    if current_length > max_bytes {
        fs::remove_file(path).map_err(|error| error.to_string())?;
        current_length = 0;
    }
    if current_length.saturating_add(incoming) <= max_bytes {
        return Ok(());
    }
    if backups == 0 {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    }
    for index in (1..=backups).rev() {
        let current = numbered_log_path(path, index);
        if index == backups {
            if let Err(error) = fs::remove_file(&current) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error.to_string());
                }
            }
        } else {
            let next = numbered_log_path(path, index + 1);
            if let Err(error) = fs::rename(&current, next) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error.to_string());
                }
            }
        }
    }
    match fs::rename(path, numbered_log_path(path, 1)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && incoming <= max_bytes => {
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn numbered_log_path(path: &Path, index: usize) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("sync_launcher.log");
    path.with_file_name(format!("{file_name}.{index}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    mod mutex_fixture {
        include!("../runtime/mutex_test_support.rs");
    }
    #[test]
    #[cfg(windows)]
    fn mutex_unowned_object_does_not_block_sync_start() {
        let name = mutex_fixture::unique_name("launcher-unowned");
        let _object = mutex_fixture::unowned(&name);
        assert!(
            SingleInstanceGuard::acquire_for_name(&name)
                .unwrap()
                .is_some(),
            "a retained unowned mutex must allow a new supervisor"
        );
    }

    #[test]
    #[cfg(windows)]
    fn mutex_live_owner_excludes_duplicate_and_release_allows_the_next_owner() {
        let name = mutex_fixture::unique_name("supervisor-live");
        let _retained = mutex_fixture::unowned(&name);
        let first = SingleInstanceGuard::acquire_for_name(&name)
            .unwrap()
            .unwrap();
        let other = name.clone();
        assert!(
            std::thread::spawn(move || SingleInstanceGuard::acquire_for_name(&other)
                .unwrap()
                .is_none())
            .join()
            .unwrap()
        );
        drop(first);
        assert!(
            std::thread::spawn(move || SingleInstanceGuard::acquire_for_name(&name)
                .unwrap()
                .is_some())
            .join()
            .unwrap()
        );
    }

    #[test]
    #[cfg(windows)]
    fn mutex_liveness_after_owner_process_exit() {
        if mutex_fixture::owner_child_if_requested() {
            return;
        }
        let fixture = mutex_fixture::abandoned("supervisor-crash");
        let owner = SingleInstanceGuard::acquire_for_name(&fixture.name).unwrap();
        assert!(
            owner.is_some(),
            "abandoned owner must allow supervisor recovery"
        );
        let other = fixture.name.clone();
        assert!(
            std::thread::spawn(move || SingleInstanceGuard::acquire_for_name(&other)
                .unwrap()
                .is_none())
            .join()
            .unwrap()
        );
        drop(owner);
        let other = fixture.name.clone();
        assert!(
            std::thread::spawn(move || SingleInstanceGuard::acquire_for_name(&other)
                .unwrap()
                .is_some())
            .join()
            .unwrap()
        );
    }

    #[test]
    #[cfg(windows)]
    fn mutex_open_failure_does_not_admit_an_unverified_supervisor() {
        let name = mutex_fixture::unique_name("supervisor-wrong-object");
        let _wrong = mutex_fixture::wrong_object(&name);
        assert!(SingleInstanceGuard::acquire_for_name(&name).is_err());
    }

    #[test]
    #[cfg(windows)]
    fn sync_autostart_definition_is_user_scoped_and_recovers_without_network_or_ac_power() {
        let registration = StartupRegistration {
            command: String::new(),
            executable: PathBuf::from(r"C:\Ten Rate & test\TenRate_Desktop_Launcher.exe"),
            stable_entry: true,
        };
        let output = startup_task_command(&registration)
            .env("GRID_TIMER_STARTUP_DRY_RUN", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["valid"], true);
        let xml = result["xml"].as_str().unwrap();
        assert!(xml.contains("<Arguments>--ensure-sync</Arguments>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<Interval>PT1M</Interval>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("Ten Rate &amp; test"));
    }

    #[test]
    fn sync_autostart_uses_the_version_independent_desktop_entry() {
        let root = unique_test_directory("stable-sync-entry");
        let current = root.join("release_artifacts").join("current");
        let entry_directory = root.join("release_artifacts").join("desktop_entry");
        fs::create_dir_all(&current).unwrap();
        fs::create_dir_all(&entry_directory).unwrap();
        let versioned = current.join("grid_timer_sync_launcher_v2.22.20-windows-stability.exe");
        let stable = entry_directory.join(STABLE_DESKTOP_ENTRY_FILE_NAME);
        write_minimal_pe_for_test(&stable);

        let plan = startup_registration_plan(&versioned, false);
        let registration = plan
            .before_supervisor_lock
            .as_ref()
            .expect("a validated stable entry must be registered before the mutex");
        assert!(registration.stable_entry);
        assert_eq!(
            format!("\"{}\" --sync-supervisor", stable.display()),
            registration.command
        );
        assert!(plan.after_supervisor_lock.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sync_autostart_falls_back_to_the_versioned_entry_if_stable_entry_is_invalid() {
        let root = unique_test_directory("missing-stable-sync-entry");
        let current = root.join("release_artifacts").join("current");
        let entry_directory = root.join("release_artifacts").join("desktop_entry");
        fs::create_dir_all(&current).unwrap();
        fs::create_dir_all(&entry_directory).unwrap();
        fs::write(
            entry_directory.join(STABLE_DESKTOP_ENTRY_FILE_NAME),
            b"not a PE",
        )
        .unwrap();
        let versioned = current.join("grid_timer_sync_launcher_v2.22.20-windows-stability.exe");

        let plan = startup_registration_plan(&versioned, false);
        assert!(plan.before_supervisor_lock.is_none());
        let registration = plan
            .after_supervisor_lock
            .as_ref()
            .expect("a versioned fallback may be registered only after lock acquisition");
        assert!(!registration.stable_entry);
        assert_eq!(format!("\"{}\"", versioned.display()), registration.command);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hot_upgrade_migrates_autostart_before_an_old_supervisor_rejects_the_mutex() {
        let events = std::cell::RefCell::new(Vec::new());
        let plan = StartupRegistrationPlan {
            before_supervisor_lock: Some(StartupRegistration {
                command: "stable".to_string(),
                executable: PathBuf::from("stable.exe"),
                stable_entry: true,
            }),
            after_supervisor_lock: None,
        };
        let acquired = acquire_supervisor_with_startup_registration(
            &plan,
            |_| events.borrow_mut().push("registered-stable-entry"),
            || {
                events.borrow_mut().push("old-supervisor-lock");
                Ok(None::<()>)
            },
        )
        .unwrap();

        assert!(acquired.is_none());
        assert_eq!(
            vec!["registered-stable-entry", "old-supervisor-lock"],
            *events.borrow()
        );
    }

    #[test]
    fn invalid_or_old_launcher_never_persists_a_fallback_before_owning_the_mutex() {
        let old_launcher =
            Path::new(r"C:\release_artifacts\old_exes\grid_timer_sync_launcher_v2.22.19.exe");
        let plan = startup_registration_plan(old_launcher, false);
        assert!(plan.before_supervisor_lock.is_none());
        assert!(plan.after_supervisor_lock.is_some());

        let events = std::cell::RefCell::new(Vec::new());
        let acquired = acquire_supervisor_with_startup_registration(
            &plan,
            |_| events.borrow_mut().push("persisted"),
            || {
                events.borrow_mut().push("mutex-denied");
                Ok(None::<()>)
            },
        )
        .unwrap();
        assert!(acquired.is_none());
        assert_eq!(vec!["mutex-denied"], *events.borrow());

        events.borrow_mut().clear();
        let error = acquire_supervisor_with_startup_registration(
            &plan,
            |_| events.borrow_mut().push("persisted"),
            || {
                events.borrow_mut().push("mutex-error");
                Err::<Option<()>, _>("lock failed".to_string())
            },
        )
        .unwrap_err();
        assert_eq!("lock failed", error);
        assert_eq!(vec!["mutex-error"], *events.borrow());
    }

    #[test]
    fn versioned_fallback_is_persisted_only_after_mutex_acquisition() {
        let plan = StartupRegistrationPlan {
            before_supervisor_lock: None,
            after_supervisor_lock: Some(StartupRegistration {
                command: "versioned".to_string(),
                executable: PathBuf::from("versioned.exe"),
                stable_entry: false,
            }),
        };
        let events = std::cell::RefCell::new(Vec::new());
        let acquired = acquire_supervisor_with_startup_registration(
            &plan,
            |_| events.borrow_mut().push("registered-versioned-entry"),
            || {
                events.borrow_mut().push("mutex-acquired");
                Ok(Some(()))
            },
        )
        .unwrap();
        assert!(acquired.is_some());
        assert_eq!(
            vec!["mutex-acquired", "registered-versioned-entry"],
            *events.borrow()
        );
    }

    #[test]
    fn autostart_registration_is_disabled_only_by_exact_one() {
        assert!(autostart_registration_disabled(Some(OsStr::new("1"))));
        assert_eq!(
            StartupRegistrationPlan::default(),
            startup_registration_plan(Path::new("untrusted-old-launcher.exe"), true)
        );

        for value in [
            None,
            Some(""),
            Some("0"),
            Some("true"),
            Some("TRUE"),
            Some(" 1"),
        ] {
            assert!(!autostart_registration_disabled(value.map(OsStr::new)));
        }
    }

    #[test]
    fn startup_backup_budget_scales_with_database_size_and_remains_bounded() {
        assert_eq!(
            SERVER_STARTUP_BASE_TIMEOUT,
            server_startup_timeout_for_bytes(0, 0)
        );
        assert_eq!(
            Duration::from_secs(
                SERVER_STARTUP_BASE_TIMEOUT.as_secs() + 512 * SERVER_STARTUP_SECONDS_PER_MIB
            ),
            server_startup_timeout_for_bytes(512 * 1024 * 1024, 0)
        );
        assert_eq!(
            SERVER_STARTUP_MAX_TIMEOUT,
            server_startup_timeout_for_bytes(u64::MAX, u64::MAX)
        );

        let directory = unique_test_directory("startup-storage-budget");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("server_store.sqlite3"), vec![0_u8; 11]).unwrap();
        fs::write(directory.join("server_store.sqlite3-wal"), vec![0_u8; 7]).unwrap();
        fs::write(directory.join("server_store.json"), vec![0_u8; 5]).unwrap();
        fs::write(directory.join("sync_launcher.log"), vec![0_u8; 101]).unwrap();
        assert_eq!(23, sync_startup_storage_bytes(&directory));
        let backups = directory.join("recovery");
        fs::create_dir_all(&backups).unwrap();
        let mib = 1024 * 1024;
        for (path, size) in [
            (directory.join("server_store_startup_100.sqlite3"), mib),
            (
                directory.join("server_store_pre_schema_v14_to_v15_100.sqlite3"),
                2 * mib,
            ),
            (
                backups.join(format!(
                    "sync_server_{}_runtime_100.sqlite3",
                    "a".repeat(64)
                )),
                3 * mib,
            ),
            (backups.join("unrelated.sqlite3"), 64 * mib),
        ] {
            fs::File::create(path).unwrap().set_len(size).unwrap();
        }
        let archives = sync_startup_archive_bytes(&directory, &backups);
        assert_eq!(6 * mib, archives);
        let live = 75 * mib;
        assert_eq!(
            server_startup_timeout_for_bytes(live, 0) + Duration::from_secs(6),
            server_startup_timeout_for_bytes(live, archives)
        );
        assert_eq!(3 * mib, sync_startup_archive_bytes(&directory, &directory));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn dns_wire_messages_are_strictly_bounded_and_correlated() {
        let query = sample_dns_query();
        assert!(is_valid_dns_query(&query));

        let mut response = query.clone();
        response[2] |= 0x80;
        assert!(is_valid_dns_response(&query, &response));

        let mut response_with_wrong_id = response.clone();
        response_with_wrong_id[1] ^= 1;
        assert!(!is_valid_dns_response(&query, &response_with_wrong_id));

        let mut response_flagged_as_query = response;
        response_flagged_as_query[2] &= !0x80;
        assert!(!is_valid_dns_response(&query, &response_flagged_as_query));

        let mut no_questions = query.clone();
        no_questions[4] = 0;
        no_questions[5] = 0;
        assert!(!is_valid_dns_query(&no_questions));
        assert!(!is_valid_dns_query(&query[..DNS_WIRE_MIN_BYTES - 1]));
        assert!(!is_valid_dns_query(&vec![0; DNS_WIRE_MAX_BYTES + 1]));
    }

    #[test]
    fn dns_a_query_builder_is_strict_and_encodes_the_expected_question() {
        let query = build_dns_a_query(CLOUDFLARE_TUNNEL_EDGE_HOSTS[0], 0x4a31).unwrap();
        assert_eq!([0x4a, 0x31], query[..2]);
        assert_eq!([0x01, 0x00, 0x00, 0x01], query[2..6]);
        let mut cursor = DNS_WIRE_MIN_BYTES;
        assert_eq!(
            CLOUDFLARE_TUNNEL_EDGE_HOSTS[0],
            read_dns_name(&query, &mut cursor).unwrap()
        );
        assert_eq!(1, take_dns_u16(&query, &mut cursor).unwrap());
        assert_eq!(1, take_dns_u16(&query, &mut cursor).unwrap());
        assert_eq!(query.len(), cursor);

        for invalid in [
            "",
            ".",
            "bad..example",
            "-bad.example",
            "bad-.example",
            "bad_name.example",
            "例子.example",
        ] {
            assert!(build_dns_a_query(invalid, 1).is_err(), "{invalid}");
        }
        assert!(build_dns_a_query(&format!("{}.example", "a".repeat(64)), 1).is_err());
    }

    #[test]
    fn cloudflare_edge_dns_parser_checks_bounds_correlation_and_pointer_cycles() {
        let hostname = CLOUDFLARE_TUNNEL_EDGE_HOSTS[0];
        let (query, response) = sample_dns_a_response(
            hostname,
            0x1234,
            &[
                Ipv4Addr::new(198, 41, 192, 7),
                Ipv4Addr::new(198, 41, 200, 167),
            ],
        );
        assert_eq!(
            vec![
                Ipv4Addr::new(198, 41, 192, 7),
                Ipv4Addr::new(198, 41, 200, 167)
            ],
            parse_cloudflare_edge_a_response(hostname, &query, &response).unwrap()
        );

        for truncated_at in [0, DNS_WIRE_MIN_BYTES - 1, response.len() - 1] {
            assert!(
                parse_cloudflare_edge_a_response(hostname, &query, &response[..truncated_at])
                    .is_err()
            );
        }

        let mut wrong_id = response.clone();
        wrong_id[1] ^= 1;
        assert!(parse_cloudflare_edge_a_response(hostname, &query, &wrong_id).is_err());

        let answer_start = query.len();
        let mut cyclic_pointer = response.clone();
        cyclic_pointer[answer_start] = 0xc0 | ((answer_start >> 8) as u8 & 0x3f);
        cyclic_pointer[answer_start + 1] = answer_start as u8;
        assert!(parse_cloudflare_edge_a_response(hostname, &query, &cyclic_pointer).is_err());

        let mut excessive_records = response;
        excessive_records[6..8].copy_from_slice(&((DNS_MAX_RECORDS + 1) as u16).to_be_bytes());
        assert!(parse_cloudflare_edge_a_response(hostname, &query, &excessive_records).is_err());
    }

    #[test]
    fn cloudflare_edge_dns_parser_rejects_other_networks_deduplicates_and_caps_results() {
        let hostname = CLOUDFLARE_TUNNEL_EDGE_HOSTS[1];
        let (query, response) = sample_dns_a_response(
            hostname,
            0x4321,
            &[
                Ipv4Addr::new(1, 1, 1, 1),
                Ipv4Addr::new(198, 18, 0, 1),
                Ipv4Addr::new(198, 41, 192, 9),
                Ipv4Addr::new(198, 41, 192, 9),
                Ipv4Addr::new(203, 0, 113, 8),
            ],
        );
        assert_eq!(
            vec![Ipv4Addr::new(198, 41, 192, 9)],
            parse_cloudflare_edge_a_response(hostname, &query, &response).unwrap()
        );

        let many = (1..=(MAX_STATIC_EDGE_IPS + 5))
            .map(|last| Ipv4Addr::new(198, 41, 200, last as u8))
            .collect::<Vec<_>>();
        let (many_query, many_response) = sample_dns_a_response(hostname, 7, &many);
        assert_eq!(
            MAX_STATIC_EDGE_IPS,
            parse_cloudflare_edge_a_response(hostname, &many_query, &many_response)
                .unwrap()
                .len()
        );
    }

    #[test]
    fn cloudflared_quick_arguments_disable_chunked_and_named_mode_keeps_token_private() {
        let mut quick = Command::new("cloudflared");
        assert_eq!(
            2,
            append_cloudflared_tunnel_prefix(
                &mut quick,
                &[
                    Ipv4Addr::new(198, 41, 192, 167),
                    Ipv4Addr::new(1, 1, 1, 1),
                    Ipv4Addr::new(198, 41, 200, 7),
                    Ipv4Addr::new(198, 41, 192, 167),
                ]
            )
        );
        append_quick_tunnel_mode_args(&mut quick, Path::new("runtime/quick.log"));
        assert_eq!(
            vec![
                "tunnel",
                "--edge",
                "198.41.192.167:7844",
                "--edge",
                "198.41.200.7:7844",
                "--url",
                "http://127.0.0.1:8917",
                "--no-chunked-encoding",
                "--protocol",
                "http2",
                "--edge-ip-version",
                "4",
                "--no-autoupdate",
                "--logfile",
                "runtime/quick.log",
                "--loglevel",
                "info",
                "--metrics",
                QUICK_TUNNEL_METRICS_BIND_ADDR,
            ],
            command_args(&quick)
        );

        let mut named = Command::new("cloudflared");
        assert_eq!(
            0,
            append_cloudflared_tunnel_prefix(
                &mut named,
                &[
                    Ipv4Addr::new(198, 41, 192, 167),
                    Ipv4Addr::new(198, 18, 0, 1),
                ]
            )
        );
        let token_file = Path::new("runtime/cloudflared_tunnel_token.txt");
        append_named_tunnel_run_args(&mut named, token_file);
        assert_eq!(
            vec![
                "tunnel",
                "run",
                "--token-file",
                "runtime/cloudflared_tunnel_token.txt",
            ],
            command_args(&named)
        );
        assert!(!command_args(&named)
            .iter()
            .any(|argument| argument == "secret"));
    }

    #[test]
    fn public_tunnel_probe_keeps_direct_first_and_vpn_proxy_fallback() {
        assert_eq!([false, true], public_tunnel_probe_proxy_modes());
    }

    #[test]
    fn public_health_recovers_only_after_a_successful_fallback_probe() {
        assert!(public_tunnel_health_with_fallback(
            || true,
            || panic!("unneeded fallback")
        ));
        assert!(public_tunnel_health_with_fallback(|| false, || true));
        assert!(!public_tunnel_health_with_fallback(|| false, || false));
    }

    #[test]
    fn public_dns_fallback_rejects_wrong_hosts_private_routes_and_mismatched_requests() {
        use ureq::Resolver;
        let hostname = "quiet-field.trycloudflare.com";
        assert_eq!(
            Some(hostname.into()),
            public_tunnel_dns_host(&format!("https://{hostname}"))
        );
        for invalid in [
            "http://quiet-field.trycloudflare.com",
            "https://quiet-field.trycloudflare.com.evil.test",
            "https://nested.quiet-field.trycloudflare.com",
            "https://quiet-field.trycloudflare.com:444",
            "https://user@quiet-field.trycloudflare.com",
            "https://quiet-field.trycloudflare.com/path",
            "https://127.0.0.1",
        ] {
            assert!(public_tunnel_dns_host(invalid).is_none(), "{invalid}");
        }
        assert_eq!(
            Some("ntfy.sh"),
            rendezvous_dns_host("https://ntfy.sh/test-topic")
        );
        for invalid in [
            "http://ntfy.sh/topic",
            "https://ntfy.sh.evil.test/topic",
            "https://ntfy.sh:444/topic",
            "https://user@ntfy.sh/topic",
            "https://127.0.0.1/topic",
        ] {
            assert!(rendezvous_dns_host(invalid).is_none(), "{invalid}");
        }
        assert!(resolve_public_dns("127.0.0.1").is_none());
        assert!(resolve_public_dns("unrelated.test").is_none());
        let approved = Ipv4Addr::new(104, 16, 230, 132);
        let mut inputs = vec![approved, approved];
        for invalid in [
            "0.0.0.0",
            "10.0.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "192.0.0.1",
            "198.18.0.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            inputs.push(invalid.parse().unwrap());
        }
        let (query, response) = sample_dns_a_response(hostname, 8, &inputs);
        let addresses =
            parse_dns_a_response(hostname, &query, &response, is_public_dns_address).unwrap();
        assert_eq!(vec![approved], addresses);
        assert!(parse_dns_a_response(
            "another.trycloudflare.com",
            &query,
            &response,
            is_public_dns_address
        )
        .is_err());
        let resolver = PublicTunnelResolver {
            authority: format!("{hostname}:443"),
            addresses: addresses
                .into_iter()
                .map(|ip| SocketAddr::from((ip, 443)))
                .collect(),
        };
        assert_eq!(
            vec![SocketAddr::from((approved, 443))],
            resolver.resolve(&format!("{hostname}:443")).unwrap()
        );
        assert!(resolver.resolve("another.trycloudflare.com:443").is_err());
        assert!(resolver.resolve(&format!("{hostname}:80")).is_err());
    }

    #[test]
    fn doh_content_type_is_exact_and_case_insensitive() {
        assert!(is_dns_message_content_type(Some("application/dns-message")));
        assert!(is_dns_message_content_type(Some(
            "Application/Dns-Message; charset=binary"
        )));
        assert!(!is_dns_message_content_type(Some("application/json")));
        assert!(!is_dns_message_content_type(None));
    }

    #[test]
    fn public_urls_must_use_https() {
        assert_eq!(
            Some("https://example.test".to_string()),
            normalize_public_url("https://example.test/")
        );
        assert_eq!(None, normalize_public_url("http://192.168.1.2:8917"));
        assert_eq!(None, normalize_public_url("https://user@example.test"));
        assert_eq!(
            None,
            normalize_public_url("https://example.test/?token=secret")
        );
        assert_eq!(None, normalize_public_url("https://example.test/a b"));
    }

    #[test]
    fn quick_tunnel_url_is_extracted_from_log() {
        let log = "INF (https://quiet-field.trycloudflare.com) | registered";
        assert_eq!(
            Some("https://quiet-field.trycloudflare.com".to_string()),
            extract_trycloudflare_url(log)
        );
        assert_eq!(
            None,
            extract_trycloudflare_url("https://quiet-field.trycloudflare.com.evil.test")
        );
        assert_eq!(
            None,
            extract_trycloudflare_url("https://quiet-field.trycloudflare.com/not-a-base-url")
        );
    }

    #[test]
    fn terminal_quick_tunnel_invalidation_requires_an_explicit_registration_denial() {
        assert!(contains_terminal_quick_tunnel_invalidation(
            r#"{"level":"error","error":"Unauthorized: Tunnel not found","message":"Register tunnel error from server side"}"#,
        ));
        assert!(contains_terminal_quick_tunnel_invalidation(
            r#"{"level":"error","error":" unauthorized: tunnel not found ","message":" register tunnel error from server side "}"#,
        ));
        assert!(!contains_terminal_quick_tunnel_invalidation(
            r#"{"level":"error","error":"Unauthorized: Tunnel not found","message":"failed to serve incoming request"}"#,
        ));
        assert!(!contains_terminal_quick_tunnel_invalidation(
            r#"{"level":"error","error":"Forbidden: invalid tunnel credentials","message":"Register tunnel error from server side"}"#,
        ));
        assert!(!contains_terminal_quick_tunnel_invalidation(
            r#"{"level":"error","error":"DialContext error: i/o timeout","message":"Unable to establish connection with Cloudflare edge"}"#,
        ));
        assert!(!contains_terminal_quick_tunnel_invalidation(
            r#"{"level":"error","error":"temporary internal server error","message":"Register tunnel error from server side"}"#,
        ));
    }

    #[test]
    fn incremental_quick_tunnel_log_reader_ignores_stale_terminal_lines() {
        let directory = unique_test_directory("quick-log-cursor");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("cloudflared_quick_tunnel.log");
        let terminal = r#"{"level":"error","error":"Unauthorized: Tunnel not found","message":"Register tunnel error from server side"}"#;
        fs::write(&path, format!("{terminal}\n")).unwrap();
        let mut offset = path.metadata().unwrap().len();

        append_test_log_line(
            &path,
            concat!(
                r#"{"level":"error","error":"dial tcp: i/o timeout","message":"Serve tunnel error"}"#,
                "\n"
            ),
        );
        let transient = read_new_complete_log_lines(&path, &mut offset).unwrap();
        assert!(!transient.contains("Tunnel not found"));
        assert!(!contains_terminal_quick_tunnel_invalidation(&transient));

        let before_partial = offset;
        append_test_log_line(&path, terminal);
        assert_eq!("", read_new_complete_log_lines(&path, &mut offset).unwrap());
        assert_eq!(before_partial, offset);

        append_test_log_line(&path, "\n");
        let invalidation = read_new_complete_log_lines(&path, &mut offset).unwrap();
        assert!(contains_terminal_quick_tunnel_invalidation(&invalidation));

        let offset_before_replacement = offset;
        fs::write(&path, "{}\n").unwrap();
        assert!(read_new_complete_log_lines(&path, &mut offset).is_err());
        assert_eq!(offset_before_replacement, offset);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rendezvous_generation_rejects_unsafe_or_malformed_storage() {
        let directory = unique_test_directory("rendezvous-generation-invalid");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(RENDEZVOUS_GENERATION_FILE_NAME);
        for invalid in ["", "01", "+1", "1\n", "184467440737095516150"] {
            fs::write(&path, invalid).unwrap();
            assert!(read_persisted_rendezvous_generation(&path).is_err());
        }
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(read_persisted_rendezvous_generation(&path).is_err());
        fs::remove_dir(&path).unwrap();
        let target = directory.join("generation-target.txt");
        let link = directory.join("generation-link.txt");
        fs::write(&target, "9").unwrap();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&target, &link).is_ok();
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&target, &link).is_ok();
        #[cfg(not(any(windows, unix)))]
        let linked = false;
        if linked {
            assert!(read_persisted_rendezvous_generation(&link).is_err());
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn quick_tunnel_metrics_address_comes_from_the_current_loopback_log_entry() {
        let log =
            r#"{"level":"info","message":"Starting metrics server on 127.0.0.1:49321/metrics"}"#;
        assert_eq!(
            Some("127.0.0.1:49321".parse::<SocketAddr>().unwrap()),
            extract_quick_tunnel_metrics_addr(log),
        );
        assert_eq!(
            None,
            extract_quick_tunnel_metrics_addr(
                r#"{"level":"info","message":"Starting metrics server on 0.0.0.0:49321/metrics"}"#,
            ),
        );
        assert_eq!(
            None,
            extract_quick_tunnel_metrics_addr(
                r#"{"level":"info","message":"Starting metrics server on 127.0.0.1:0/metrics"}"#,
            ),
        );
    }

    #[test]
    fn sync_server_resolution_uses_one_exact_file_and_rejects_tampering() {
        let directory = unique_test_directory("server-integrity");
        fs::create_dir_all(&directory).unwrap();
        let exact = directory.join("timer_sync_server.exe");
        let decoy = directory.join("grid_timer_sync_server_v99.99.99-newer.exe");
        write_minimal_pe_for_test(&exact);
        write_minimal_pe_for_test(&decoy);
        assert_eq!(exact, find_sync_server(&directory).unwrap());

        let (size, sha256) = sha256_file_hex(&exact).unwrap();
        let expectation = SyncServerExpectation {
            file_name: "timer_sync_server.exe",
            expected_size: Some(size),
            expected_sha256: Some(Box::leak(sha256.into_boxed_str())),
        };
        verify_sync_server_candidate(&exact, &directory, expectation).unwrap();
        let mut bytes = fs::read(&exact).unwrap();
        bytes[100] = 1;
        fs::write(&exact, bytes).unwrap();
        assert!(verify_sync_server_candidate(&exact, &directory, expectation).is_err());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn health_response_requires_valid_json_and_success_flag() {
        let trusted = serde_json::json!({
            "ok": true,
            "mode": "health",
            "productId": product_identity::PRODUCT.internal_id,
            "serviceRole": "sync_server",
            "syncProtocolVersion": product_identity::PRODUCT.sync_protocol_version,
            "serverBuildId": SYNC_SERVER_BUILD_ID,
            "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
            "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
            "serverProcessId": 42,
        });
        assert!(is_valid_health_response(&trusted.to_string()));
        let mut failed = trusted.clone();
        failed["ok"] = serde_json::json!(false);
        assert!(!is_valid_health_response(&failed.to_string()));
        let mut forged = trusted;
        forged["productId"] = serde_json::json!("another-product");
        assert!(!is_valid_health_response(&forged.to_string()));
        assert!(!is_valid_health_response(
            r#"{"message":"contains \"mode\":\"health\" only"}"#
        ));
    }

    #[test]
    fn managed_listener_owner_requires_the_exact_child_pid_without_image_fallback() {
        let image_check_called = std::cell::Cell::new(false);
        assert!(listener_owner_matches_expected(Some(42), Some(42), |_| {
            image_check_called.set(true);
            false
        },));
        assert!(!image_check_called.get());

        assert!(!listener_owner_matches_expected(Some(43), Some(42), |_| {
            image_check_called.set(true);
            true
        },));
        assert!(!image_check_called.get());
        assert!(!listener_owner_matches_expected(None, Some(42), |_| true));
    }

    #[test]
    fn unmanaged_same_server_executable_with_wrong_store_never_authorizes_tunnel() {
        let image_check_called = std::cell::Cell::new(false);
        let listener_authorized = listener_owner_matches_expected(Some(77), None, |pid| {
            image_check_called.set(true);
            // Model the exact same server executable running outside this
            // launcher's lifecycle, potentially against a different store.
            pid == 77
        });

        assert!(!listener_authorized);
        assert!(!image_check_called.get());
        let tunnel_may_launch = health_with_stable_listener_owner(
            LocalServerHealth::Healthy,
            listener_authorized.then_some(77),
            listener_authorized.then_some(77),
        )
        .is_ready();
        assert!(!tunnel_may_launch);
        assert!(!listener_owner_matches_expected(None, None, |_| true));
    }

    #[test]
    fn healthy_and_busy_responses_require_the_same_verified_listener_owner() {
        for health in [LocalServerHealth::Healthy, LocalServerHealth::Busy] {
            assert_eq!(
                health,
                health_with_stable_listener_owner(health, Some(91), Some(91))
            );
            assert_eq!(
                LocalServerHealth::Unavailable,
                health_with_stable_listener_owner(health, Some(91), Some(92))
            );
            assert_eq!(
                LocalServerHealth::Unavailable,
                health_with_stable_listener_owner(health, None, Some(91))
            );
            assert_eq!(
                LocalServerHealth::Unavailable,
                health_with_stable_listener_owner(health, Some(91), None)
            );
        }
    }

    #[test]
    fn local_health_must_match_the_selected_server_executable() {
        let expected = format!("grid_timer_sync_server_v{SYNC_SERVER_BUILD_ID}.exe");
        let healthy = serde_json::json!({
            "ok": true,
            "mode": "health",
            "productId": gridtimer_native::product_identity::PRODUCT.internal_id,
            "serviceRole": "sync_server",
            "syncProtocolVersion": gridtimer_native::product_identity::PRODUCT.sync_protocol_version,
            "serverBuildId": SYNC_SERVER_BUILD_ID,
            "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
            "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
            "serverProcessId": 42,
            "serverProcessName": expected,
        })
        .to_string();
        assert!(is_expected_health_response(&healthy, &expected, 42));
        let future_process = "grid_timer_sync_server_v2.22.21-next.exe";
        let future_health = serde_json::json!({
            "ok": true,
            "mode": "health",
            "productId": gridtimer_native::product_identity::PRODUCT.internal_id,
            "serviceRole": "sync_server",
            "syncProtocolVersion": gridtimer_native::product_identity::PRODUCT.sync_protocol_version,
            "serverBuildId": "2.22.21-next",
            "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
            "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
            "serverProcessId": 42,
            "serverProcessName": future_process,
        })
        .to_string();
        assert!(is_expected_health_response(
            &future_health,
            future_process,
            42
        ));
        let busy = serde_json::json!({
            "ok": false,
            "mode": "busy",
            "productId": gridtimer_native::product_identity::PRODUCT.internal_id,
            "serviceRole": "sync_server",
            "syncProtocolVersion": gridtimer_native::product_identity::PRODUCT.sync_protocol_version,
            "serverBuildId": SYNC_SERVER_BUILD_ID,
            "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
            "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
            "serverProcessId": 42,
            "serverProcessName": expected,
        })
        .to_string();
        assert!(is_expected_busy_response(&busy, &expected, 42));
        assert!(!is_expected_health_response(&busy, &expected, 42));
        let healthy_uppercase_process = serde_json::json!({
            "ok": true,
            "mode": "health",
            "productId": gridtimer_native::product_identity::PRODUCT.internal_id,
            "serviceRole": "sync_server",
            "syncProtocolVersion": gridtimer_native::product_identity::PRODUCT.sync_protocol_version,
            "serverBuildId": SYNC_SERVER_BUILD_ID,
            "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
            "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
            "serverProcessId": 42,
            "serverProcessName": expected.to_ascii_uppercase(),
        })
        .to_string();
        assert!(is_expected_health_response(
            &healthy_uppercase_process,
            &expected,
            42,
        ));
        assert!(!is_expected_health_response(
            r#"{"ok":true,"mode":"health"}"#,
            &expected,
            42,
        ));
        for invalid in [
            serde_json::json!({
                "ok": true,
                "mode": "health",
                "productId": gridtimer_native::product_identity::PRODUCT.internal_id,
                "serviceRole": "sync_server",
                "syncProtocolVersion": gridtimer_native::product_identity::PRODUCT.sync_protocol_version,
                "serverBuildId": SYNC_SERVER_BUILD_ID,
                "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
                "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
                "serverProcessId": 42,
                "serverProcessName": "grid_timer_sync_server_v0.0.0-stale.exe",
            }),
            serde_json::json!({
                "ok": false,
                "mode": "health",
                "productId": gridtimer_native::product_identity::PRODUCT.internal_id,
                "serviceRole": "sync_server",
                "syncProtocolVersion": gridtimer_native::product_identity::PRODUCT.sync_protocol_version,
                "serverBuildId": SYNC_SERVER_BUILD_ID,
                "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
                "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
                "serverProcessId": 42,
                "serverProcessName": expected,
            }),
            serde_json::json!({
                "ok": true,
                "mode": "health",
                "productId": gridtimer_native::product_identity::PRODUCT.internal_id,
                "serviceRole": "sync_server",
                "syncProtocolVersion": gridtimer_native::product_identity::PRODUCT.sync_protocol_version,
                "serverBuildId": "stale-build",
                "serverGitCommit": product_identity::BUILD_GIT_COMMIT,
                "serverSourceSnapshotSha256": product_identity::BUILD_SOURCE_SNAPSHOT_SHA256,
                "serverProcessId": 42,
                "serverProcessName": expected,
            }),
        ] {
            assert!(!is_expected_health_response(
                &invalid.to_string(),
                &expected,
                42,
            ));
        }
    }

    #[test]
    fn verified_quick_tunnel_url_survives_transient_health_failures() {
        let policy = PublicHealthFailurePolicy::RetainQuickTunnel;
        let mut state = PublicUrlPublicationState::default();

        assert_eq!(PublicUrlAction::Publish, state.observe(true, policy));
        state.mark_published();
        assert_eq!(PublicUrlAction::KeepPublished, state.observe(true, policy));
        assert_eq!(PublicUrlAction::KeepPublished, state.observe(false, policy));
        assert!(state.published);
        assert!(!policy.should_replace(state.consecutive_health_failures, true));

        for _ in 0..(MAX_CONSECUTIVE_HEALTH_FAILURES * 4) {
            assert_eq!(PublicUrlAction::KeepPublished, state.observe(false, policy));
            assert!(!policy.should_replace(state.consecutive_health_failures, true));
        }

        assert_eq!(PublicUrlAction::KeepPublished, state.observe(true, policy));
        assert_eq!(0, state.consecutive_health_failures);
        assert_eq!(PublicUrlAction::KeepPublished, state.observe(true, policy));
    }

    #[test]
    fn verified_quick_rotation_requires_both_failure_count_and_window() {
        let started = Instant::now();
        let before_window = started + VERIFIED_QUICK_TUNNEL_FAILURE_WINDOW - Duration::from_secs(1);
        let at_window = started + VERIFIED_QUICK_TUNNEL_FAILURE_WINDOW;
        let mut state = SustainedQuickTunnelFailureState::default();
        for index in 0..VERIFIED_QUICK_TUNNEL_FAILURE_THRESHOLD {
            let observed_at = if index == 0 { started } else { before_window };
            state.observe(false, observed_at);
        }
        assert!(!state.sustained_failure_reached(before_window));
        assert!(state.sustained_failure_reached(at_window));

        let mut below_threshold = SustainedQuickTunnelFailureState::default();
        for _ in 0..(VERIFIED_QUICK_TUNNEL_FAILURE_THRESHOLD - 1) {
            below_threshold.observe(false, started);
        }
        assert!(!below_threshold.sustained_failure_reached(at_window));

        state.observe(true, at_window);
        assert_eq!(0, state.consecutive_failures);
        assert_eq!(None, state.first_failure_at);
        assert!(
            !state.sustained_failure_reached(started + VERIFIED_QUICK_TUNNEL_FAILURE_WINDOW * 2)
        );
    }

    #[test]
    fn sustained_rotation_excludes_pending_named_and_external_tunnels() {
        let started = Instant::now();
        let decision_at = started + VERIFIED_QUICK_TUNNEL_FAILURE_WINDOW;
        let mut state = SustainedQuickTunnelFailureState::default();
        for _ in 0..VERIFIED_QUICK_TUNNEL_FAILURE_THRESHOLD {
            state.observe(false, started);
        }

        for (initially_verified, is_quick, owns_child) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
        ] {
            assert!(!should_rotate_managed_quick_tunnel(
                &state,
                decision_at,
                initially_verified,
                is_quick,
                owns_child,
            ));
        }
        assert!(should_rotate_managed_quick_tunnel(
            &state,
            decision_at,
            true,
            true,
            true,
        ));
    }

    #[test]
    fn pending_quick_tunnel_keeps_the_allocated_url_during_startup() {
        let started = Instant::now();
        let mut timed = SustainedQuickTunnelFailureState::default();
        for _ in 0..VERIFIED_QUICK_TUNNEL_FAILURE_THRESHOLD {
            timed.observe(false, started);
        }
        assert!(!should_rotate_managed_quick_tunnel(
            &timed,
            started + PENDING_QUICK_TUNNEL_FAILURE_WINDOW - Duration::from_secs(1),
            false,
            true,
            true
        ));
        assert!(should_rotate_managed_quick_tunnel(
            &timed,
            started + PENDING_QUICK_TUNNEL_FAILURE_WINDOW,
            false,
            true,
            true
        ));
        let policy = PublicHealthFailurePolicy::RetainQuickTunnel;
        let mut state = PublicUrlPublicationState::default();

        for _ in 0..(MAX_CONSECUTIVE_HEALTH_FAILURES - 1) {
            assert_eq!(PublicUrlAction::KeepWithdrawn, state.observe(false, policy));
            assert!(!policy.should_replace(state.consecutive_health_failures, false));
        }
        assert_eq!(PublicUrlAction::KeepWithdrawn, state.observe(false, policy));
        assert!(
            !policy.should_replace(state.consecutive_health_failures, false),
            "pending tunnel was discarded after only three health checks"
        );
        assert!(!policy.should_replace(state.consecutive_health_failures, true));
    }

    #[test]
    fn stable_tunnel_keeps_its_existing_health_failure_fallback_threshold() {
        let policy = PublicHealthFailurePolicy::ReplaceAfterThreshold;

        assert!(!policy.should_replace(MAX_CONSECUTIVE_HEALTH_FAILURES - 1, true));
        assert!(policy.should_replace(MAX_CONSECUTIVE_HEALTH_FAILURES, true));
        assert!(policy.should_replace(MAX_CONSECUTIVE_HEALTH_FAILURES + 1, true));
    }

    #[test]
    fn rendezvous_schedule_publishes_immediately_then_renews_and_backs_off() {
        let started = Instant::now();
        let first_url = "https://first.example.test";
        let second_url = "https://second.example.test";
        let mut schedule = RendezvousPublicationSchedule::default();

        assert!(schedule.should_publish(started, first_url, false));
        assert_eq!(
            RENDEZVOUS_RETRY_INITIAL,
            schedule.record_failure(started, first_url)
        );
        assert!(!schedule.should_publish(
            started + RENDEZVOUS_RETRY_INITIAL - Duration::from_secs(1),
            first_url,
            false,
        ));
        assert!(schedule.should_publish(started + RENDEZVOUS_RETRY_INITIAL, first_url, false,));
        assert!(schedule.should_publish(started, second_url, false));

        schedule.record_success(started, second_url);
        assert!(!schedule.should_publish(
            started + RENDEZVOUS_RENEW_INTERVAL - Duration::from_secs(1),
            second_url,
            false,
        ));
        assert!(schedule.should_publish(started + RENDEZVOUS_RENEW_INTERVAL, second_url, false,));
        assert!(schedule.should_publish(started, second_url, true));
    }

    #[test]
    fn rendezvous_retry_delay_is_exponential_and_capped() {
        assert_eq!(Duration::from_secs(30), rendezvous_retry_delay(1));
        assert_eq!(Duration::from_secs(60), rendezvous_retry_delay(2));
        assert_eq!(Duration::from_secs(120), rendezvous_retry_delay(3));
        assert_eq!(RENDEZVOUS_RETRY_MAX, rendezvous_retry_delay(32));
    }

    #[test]
    fn rendezvous_generation_survives_restart_and_clock_rollback() {
        let directory = unique_test_directory("rendezvous-generation-restart");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(RENDEZVOUS_GENERATION_FILE_NAME);

        assert_eq!(
            100,
            allocate_rendezvous_generation_in_file(&path, 100, 0).unwrap()
        );
        assert_eq!(
            101,
            allocate_rendezvous_generation_in_file(&path, 50, 0).unwrap()
        );
        assert_eq!(
            102,
            allocate_rendezvous_generation_in_file(&path, 40, 101).unwrap()
        );
        assert_eq!("102", fs::read_to_string(&path).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persistent_tunnel_configuration_precedes_and_absorbs_legacy_files() {
        let directory = unique_test_directory("persistent-tunnel-config");
        let local_dir = directory.join("local");
        let legacy_dir = directory.join("legacy");
        fs::create_dir_all(&local_dir).unwrap();
        fs::create_dir_all(&legacy_dir).unwrap();
        let local_url = local_dir.join(STABLE_PUBLIC_URL_FILE_NAME);
        let legacy_url = legacy_dir.join(STABLE_PUBLIC_URL_FILE_NAME);
        fs::write(&local_url, "https://local.example.test").unwrap();
        fs::write(&legacy_url, "https://legacy.example.test").unwrap();
        assert_eq!(
            Some("https://local.example.test".to_string()),
            configured_stable_public_url_from_files(&local_url, &legacy_url)
        );

        fs::remove_file(&local_url).unwrap();
        assert_eq!(
            Some("https://legacy.example.test".to_string()),
            configured_stable_public_url_from_files(&local_url, &legacy_url)
        );
        assert_eq!(
            "https://legacy.example.test",
            fs::read_to_string(&local_url).unwrap()
        );

        let local_token = local_dir.join(CLOUDFLARED_TOKEN_FILE_NAME);
        let legacy_token = legacy_dir.join(CLOUDFLARED_TOKEN_FILE_NAME);
        fs::write(&local_token, "local-token").unwrap();
        fs::write(&legacy_token, "legacy-token").unwrap();
        assert_eq!(
            Some(local_token.clone()),
            configured_cloudflared_tunnel_token_file_from_files(&local_token, &legacy_token)
        );

        fs::remove_file(&local_token).unwrap();
        assert_eq!(
            Some(local_token.clone()),
            configured_cloudflared_tunnel_token_file_from_files(&local_token, &legacy_token)
        );
        assert_eq!("legacy-token", fs::read_to_string(&local_token).unwrap());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn atomic_publication_replaces_existing_file() {
        let directory = unique_test_directory("atomic-publication");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sync_public_server_url.txt");
        fs::write(&path, "https://old.example.test").unwrap();
        atomic_write_text(&path, "https://new.example.test").unwrap();
        assert_eq!(
            "https://new.example.test",
            fs::read_to_string(&path).unwrap()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn launcher_logs_rotate_to_a_fixed_backup_count() {
        let directory = unique_test_directory("log-rotation");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("launcher.log");
        fs::write(&path, "12345678").unwrap();
        rotate_log_files_if_needed(&path, 8, 2, 1).unwrap();
        assert!(!path.exists());
        assert!(numbered_log_path(&path, 1).exists());

        fs::write(&path, "abcdefgh").unwrap();
        rotate_log_files_if_needed(&path, 8, 2, 1).unwrap();
        assert!(numbered_log_path(&path, 1).exists());
        assert!(numbered_log_path(&path, 2).exists());
        assert!(!numbered_log_path(&path, 3).exists());

        fs::write(&path, "oversized").unwrap();
        rotate_log_files_if_needed(&path, 8, 2, 1).unwrap();
        assert!(!path.exists());
        assert!(numbered_log_path(&path, 1).metadata().unwrap().len() <= 8);
        assert!(numbered_log_path(&path, 2).metadata().unwrap().len() <= 8);
        fs::remove_dir_all(directory).unwrap();
    }

    fn unique_test_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        env::temp_dir().join(format!(
            "grid-timer-launcher-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_minimal_pe_for_test(path: &Path) {
        let mut bytes = vec![0_u8; 128];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        fs::write(path, bytes).unwrap();
    }

    fn append_test_log_line(path: &Path, value: &str) {
        let mut file = OpenOptions::new().append(true).open(path).unwrap();
        file.write_all(value.as_bytes()).unwrap();
        file.flush().unwrap();
    }

    fn sample_dns_a_response(
        hostname: &str,
        transaction_id: u16,
        addresses: &[Ipv4Addr],
    ) -> (Vec<u8>, Vec<u8>) {
        let query = build_dns_a_query(hostname, transaction_id).unwrap();
        let mut response = Vec::new();
        response.extend_from_slice(&transaction_id.to_be_bytes());
        response.extend_from_slice(&0x8180_u16.to_be_bytes());
        response.extend_from_slice(&1_u16.to_be_bytes());
        response.extend_from_slice(&(addresses.len() as u16).to_be_bytes());
        response.extend_from_slice(&0_u16.to_be_bytes());
        response.extend_from_slice(&0_u16.to_be_bytes());
        response.extend_from_slice(&query[DNS_WIRE_MIN_BYTES..]);
        for address in addresses {
            response.extend_from_slice(&[0xc0, DNS_WIRE_MIN_BYTES as u8]);
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&1_u16.to_be_bytes());
            response.extend_from_slice(&60_u32.to_be_bytes());
            response.extend_from_slice(&4_u16.to_be_bytes());
            response.extend_from_slice(&address.octets());
        }
        (query, response)
    }

    fn command_args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn sample_dns_query() -> Vec<u8> {
        vec![
            0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, b'w',
            b'w', b'w', 0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'c', b'o', b'm',
            0x00, 0x00, 0x01, 0x00, 0x01,
        ]
    }
}
