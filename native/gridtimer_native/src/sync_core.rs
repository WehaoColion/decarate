use crate::app_data;
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{
    Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket,
};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_SYNC_PORT: u16 = 8917;
const MAX_REQUEST_BODY_BYTES: usize = 24 * 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 24 * 1024 * 1024;
const SYNC_CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
const SYNC_IO_TIMEOUT: Duration = Duration::from_secs(20);
const SYNC_DISCOVERY_WORKERS: usize = 32;
const SYNC_DISCOVERY_CONNECT_TIMEOUT: Duration = Duration::from_millis(220);
const SYNC_DISCOVERY_IO_TIMEOUT: Duration = Duration::from_millis(450);
const SYNC_DISCOVERY_TOTAL_TIMEOUT: Duration = Duration::from_secs(3);
const UPNP_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);
const UPNP_HTTP_TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncClientResult {
    pub ok: bool,
    pub message: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub app_data_json: Option<String>,
    #[serde(default)]
    pub server_updated_at_epoch_millis: i64,
    #[serde(default)]
    pub client_updated_at_epoch_millis: i64,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub resolved_server_url: String,
    #[serde(default)]
    pub public_server_url: String,
    #[serde(default)]
    pub public_access_message: String,
}

#[derive(Clone, Debug, Default)]
struct ServerRuntimeInfo {
    public_server_url: String,
    public_access_message: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterRequest {
    email: String,
    password: String,
    device_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    email: String,
    password: String,
    device_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSnapshotRequest {
    app_data_json: String,
    client_updated_at_epoch_millis: i64,
    device_name: String,
    #[serde(default)]
    force_upload: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerStore {
    users: Vec<ServerUser>,
}

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

enum SyncClientIoError {
    Connect(io::Error),
    Send(io::Error),
    Read(io::Error),
}

pub fn default_server_url() -> String {
    format!("http://127.0.0.1:{DEFAULT_SYNC_PORT}")
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
    encode_result(&sync_app_data_result(
        server_url,
        token,
        app_data_json,
        client_updated_at_epoch_millis,
        device_name,
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
    encode_result(&sync_app_data_result(
        server_url,
        token,
        app_data_json,
        client_updated_at_epoch_millis,
        device_name,
        true,
    ))
}

pub fn app_data_revision_millis(raw: &str, fallback: i64) -> i64 {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return fallback.max(0);
    };
    max_revision_in_value(&value).unwrap_or_else(|| {
        if has_meaningful_app_data(&value) {
            fallback.max(0)
        } else {
            0
        }
    })
}

pub fn sanitize_sync_app_data(raw: &str, now: i64) -> Option<String> {
    app_data::sanitize_app_data_json(raw, now)
}

pub fn run_sync_server(bind_addr: &str, store_path: &Path) -> io::Result<()> {
    let listener = TcpListener::bind(bind_addr)?;
    let store = Arc::new(Mutex::new(ServerStore::load(store_path)?));
    let store_path = Arc::new(store_path.to_path_buf());
    let configured_public_access = configured_public_runtime_info();
    let has_configured_public_access = !configured_public_access.public_server_url.is_empty();
    let runtime_info = Arc::new(Mutex::new(configured_public_access));
    println!("sync server listening on http://{bind_addr}");
    println!("sync store: {}", store_path.display());
    if has_configured_public_access {
        if let Ok(locked) = runtime_info.lock() {
            println!("sync public access: {}", locked.public_access_message);
        }
    } else if public_access_candidate_port(bind_addr).is_some() {
        let runtime_info_for_mapping = Arc::clone(&runtime_info);
        let bind_addr_for_mapping = bind_addr.to_string();
        thread::spawn(move || {
            let info = match public_access_candidate_port(&bind_addr_for_mapping) {
                Some(port) => configure_public_sync_access(port),
                None => ServerRuntimeInfo {
                    public_access_message: "Sync server is bound to a local-only address."
                        .to_string(),
                    ..ServerRuntimeInfo::default()
                },
            };
            println!("sync public access: {}", info.public_access_message);
            if let Ok(mut locked) = runtime_info_for_mapping.lock() {
                *locked = info;
            }
        });
    }

    for incoming in listener.incoming() {
        let stream = incoming?;
        let store = Arc::clone(&store);
        let store_path = Arc::clone(&store_path);
        let runtime_info = Arc::clone(&runtime_info);
        thread::spawn(move || {
            if let Err(error) = handle_connection(stream, store, store_path, runtime_info) {
                eprintln!("request failed: {error}");
            }
        });
    }
    Ok(())
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
        .or_else(|| configured_public_server_url_file().and_then(|path| {
            fs::read_to_string(path)
                .ok()
                .and_then(|value| normalized_public_server_url(&value))
        }))
        .unwrap_or_default();
    if public_server_url.is_empty() {
        ServerRuntimeInfo::default()
    } else {
        ServerRuntimeInfo {
            public_server_url: public_server_url.clone(),
            public_access_message: format!("Public tunnel URL: {public_server_url}"),
        }
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
    if let Ok(ip) = parsed.host.parse::<Ipv4Addr>() {
        if !is_public_sync_ipv4(ip) {
            return None;
        }
    }
    Some(trimmed)
}

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
    }
}

#[derive(Clone, Debug)]
struct UpnpGateway {
    control_url: String,
    service_type: String,
    control_host: String,
    control_port: u16,
}

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

fn find_wan_connection_service(description: &str, location: &str) -> Option<(String, String)> {
    for block in xml_blocks(description, "service") {
        let service_type = xml_tag_text(block, "serviceType")?;
        if !service_type.contains("WANIPConnection") && !service_type.contains("WANPPPConnection")
        {
            continue;
        }
        let control_path = xml_tag_text(block, "controlURL")?;
        let control_url = absolute_control_url(location, &control_path)?;
        return Some((service_type, control_url));
    }
    None
}

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

fn xml_tag_text(xml: &str, tag: &str) -> Option<String> {
    let lower = xml.to_ascii_lowercase();
    let tag_lower = tag.to_ascii_lowercase();
    let open = format!("<{tag_lower}");
    let close = format!("</{tag_lower}>");
    let open_start = lower.find(&open)?;
    let open_end = lower[open_start..].find('>')? + open_start;
    let content_start = open_end + 1;
    let close_start = lower[content_start..].find(&close)? + content_start;
    Some(xml_unescape(xml[content_start..close_start].trim()))
        .filter(|value| !value.is_empty())
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

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

fn upnp_external_ipv4(gateway: &UpnpGateway) -> Result<Ipv4Addr, String> {
    let response = upnp_soap_call(gateway, "GetExternalIPAddress", "")?;
    let raw_ip = xml_tag_text(&response, "NewExternalIPAddress")
        .ok_or_else(|| "router did not return NewExternalIPAddress".to_string())?;
    raw_ip
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("router returned invalid external address {raw_ip}"))
}

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
    let mapped_port = xml_tag_text(&response, "NewInternalPort")
        .and_then(|value| value.parse::<u16>().ok());
    Ok(mapped_ip == Some(local_ip) && mapped_port == Some(external_port))
}

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
    app_data_json: &str,
    client_updated_at_epoch_millis: i64,
    device_name: &str,
    force_upload: bool,
) -> SyncClientResult {
    if token.trim().is_empty() {
        return error_result("Not logged in.");
    }
    let now = now_millis();
    let sanitized = sanitize_sync_app_data(app_data_json, now)
        .unwrap_or_else(|| app_data_json.trim().to_string());
    let request = SyncSnapshotRequest {
        app_data_json: sanitized,
        client_updated_at_epoch_millis,
        device_name: normalized_device_name(device_name),
        force_upload,
    };
    post_json(
        server_url,
        if force_upload {
            "/v1/upload-local"
        } else {
            "/v1/sync"
        },
        Some(token),
        serde_json::to_value(request).unwrap_or_else(|_| json!({})),
    )
}

fn post_json(
    server_url: &str,
    endpoint: &str,
    bearer_token: Option<&str>,
    body: Value,
) -> SyncClientResult {
    let base = match parse_base_url(server_url) {
        Ok(value) => value,
        Err(error) => return error_result(&error),
    };
    let request_body = match serde_json::to_string(&body) {
        Ok(value) => value,
        Err(_) => return error_result("Could not encode sync request."),
    };

    match send_json_to_base(&base, endpoint, bearer_token, &request_body) {
        Ok(result) => result,
        Err(SyncClientIoError::Connect(error)) => {
            if let Some(discovered_base) = discover_sync_server_base(&base) {
                match send_json_to_base(&discovered_base, endpoint, bearer_token, &request_body) {
                    Ok(mut result) => {
                        if result.resolved_server_url.trim().is_empty() {
                            result.resolved_server_url = format_base_url(&discovered_base);
                        }
                        return result;
                    }
                    Err(SyncClientIoError::Connect(retry_error)) => {
                        return error_result(&format!(
                            "Could not connect to discovered sync server: {retry_error}"
                        ));
                    }
                    Err(SyncClientIoError::Send(retry_error)) => {
                        return error_result(&format!(
                            "Could not send sync request: {retry_error}"
                        ));
                    }
                    Err(SyncClientIoError::Read(retry_error)) => {
                        return error_result(&format!(
                            "Could not read sync response: {retry_error}"
                        ));
                    }
                }
            }
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
        return send_json_to_https_base(base, endpoint, bearer_token, request_body);
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

    let mut stream =
        connect_with_timeout(&base.host, base.port).map_err(SyncClientIoError::Connect)?;
    let _ = stream.set_read_timeout(Some(SYNC_IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SYNC_IO_TIMEOUT));
    if let Err(error) = stream.write_all(request.as_bytes()) {
        return Err(SyncClientIoError::Send(error));
    }
    let response_bytes = match read_http_response(&mut stream) {
        Ok(value) => value,
        Err(error) => return Err(SyncClientIoError::Read(error)),
    };
    Ok(parse_client_response(&response_bytes))
}

fn send_json_to_https_base(
    base: &ParsedBaseUrl,
    endpoint: &str,
    bearer_token: Option<&str>,
    request_body: &str,
) -> Result<SyncClientResult, SyncClientIoError> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(SYNC_CONNECT_TIMEOUT)
        .timeout_read(SYNC_IO_TIMEOUT)
        .build();
    let url = format_request_url(base, endpoint);
    let mut request = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json");
    if let Some(token) = bearer_token.map(str::trim).filter(|token| !token.is_empty()) {
        request = request.set("Authorization", &format!("Bearer {token}"));
    }
    match request.send_string(request_body) {
        Ok(response) => {
            let status = response.status();
            let raw = response.into_string().map_err(|error| {
                SyncClientIoError::Read(io::Error::new(io::ErrorKind::Other, error))
            })?;
            Ok(parse_sync_body_response(status, &raw))
        }
        Err(ureq::Error::Status(status, response)) => {
            let raw = response.into_string().map_err(|error| {
                SyncClientIoError::Read(io::Error::new(io::ErrorKind::Other, error))
            })?;
            Ok(parse_sync_body_response(status, &raw))
        }
        Err(error) => Err(SyncClientIoError::Connect(io::Error::new(
            io::ErrorKind::Other,
            error.to_string(),
        ))),
    }
}

fn connect_with_timeout(host: &str, port: u16) -> io::Result<TcpStream> {
    let address = format!("{host}:{port}");
    let mut resolved = address.to_socket_addrs()?;
    let Some(first_addr) = resolved.next() else {
        return Err(io::Error::new(
            io::ErrorKind::AddrNotAvailable,
            format!("no socket address resolved for {address}"),
        ));
    };

    let mut last_error: Option<io::Error> = None;
    for socket_addr in std::iter::once(first_addr).chain(resolved) {
        match TcpStream::connect_timeout(&socket_addr, SYNC_CONNECT_TIMEOUT) {
            Ok(stream) => return Ok(stream),
            Err(error) => last_error = Some(error),
        }
    }

    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("timed out while connecting to {address}"),
        )
    }))
}

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

fn is_discoverable_lan_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

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

fn handle_connection(
    mut stream: TcpStream,
    store: Arc<Mutex<ServerStore>>,
    store_path: Arc<PathBuf>,
    runtime_info: Arc<Mutex<ServerRuntimeInfo>>,
) -> io::Result<()> {
    let request = read_http_request(&mut stream)?;
    let route = request.path.split('?').next().unwrap_or(&request.path);
    let response = match (request.method.as_str(), route) {
        ("GET", "/health") => {
            let info = runtime_info.lock().map(|locked| locked.clone()).unwrap_or_default();
            (
                200,
                SyncClientResult {
                    ok: true,
                    message: "Sync server is running.".to_string(),
                    mode: "health".to_string(),
                    public_server_url: info.public_server_url,
                    public_access_message: info.public_access_message,
                    ..SyncClientResult::default()
                },
            )
        }
        ("POST", "/v1/register") => handle_register(&request.body, &store, &store_path),
        ("POST", "/v1/login") => handle_login(&request.body, &store, &store_path),
        ("POST", "/v1/sync") => handle_sync(&request, &store, &store_path),
        ("POST", "/v1/upload-local") => handle_sync_mode(&request, &store, &store_path, true),
        _ => (404, error_result("Endpoint not found.")),
    };
    write_json_response(&mut stream, response.0, &response.1)
}

fn handle_register(
    body: &str,
    store: &Arc<Mutex<ServerStore>>,
    store_path: &Path,
) -> (u16, SyncClientResult) {
    let request = match serde_json::from_str::<RegisterRequest>(body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid registration request.")),
    };
    let email = normalized_email(&request.email);
    if !email.contains('@') {
        return (400, error_result("Email is invalid."));
    }
    if request.password.chars().count() < 6 {
        return (400, error_result("Password must be at least 6 characters."));
    }
    let now = now_millis();
    let mut locked = match store.lock() {
        Ok(value) => value,
        Err(_) => return (500, error_result("Sync store is unavailable.")),
    };
    if locked.users.iter().any(|user| user.email == email) {
        return (409, error_result("Account already exists."));
    }
    let salt = random_token(18);
    let token = random_token(32);
    let user_id = format!("user-{}", random_token(12));
    locked.users.push(ServerUser {
        id: user_id.clone(),
        email,
        password_salt: salt.clone(),
        password_hash: password_hash(&salt, &request.password),
        created_at_epoch_millis: now,
        updated_at_epoch_millis: now,
        app_data_json: String::new(),
        tokens: vec![ServerToken {
            token: token.clone(),
            device_name: normalized_device_name(&request.device_name),
            created_at_epoch_millis: now,
            last_seen_at_epoch_millis: now,
        }],
    });
    if let Err(error) = locked.save(store_path) {
        return (
            500,
            error_result(&format!("Could not save account: {error}")),
        );
    }
    (
        200,
        SyncClientResult {
            ok: true,
            message: "Account created.".to_string(),
            user_id,
            token,
            mode: "registered".to_string(),
            ..SyncClientResult::default()
        },
    )
}

fn handle_login(
    body: &str,
    store: &Arc<Mutex<ServerStore>>,
    store_path: &Path,
) -> (u16, SyncClientResult) {
    let request = match serde_json::from_str::<LoginRequest>(body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid login request.")),
    };
    let email = normalized_email(&request.email);
    let now = now_millis();
    let mut locked = match store.lock() {
        Ok(value) => value,
        Err(_) => return (500, error_result("Sync store is unavailable.")),
    };
    let Some(user) = locked.users.iter_mut().find(|user| user.email == email) else {
        return (401, error_result("Account or password is wrong."));
    };
    if password_hash(&user.password_salt, &request.password) != user.password_hash {
        return (401, error_result("Account or password is wrong."));
    }
    let token = random_token(32);
    user.tokens.push(ServerToken {
        token: token.clone(),
        device_name: normalized_device_name(&request.device_name),
        created_at_epoch_millis: now,
        last_seen_at_epoch_millis: now,
    });
    let user_id = user.id.clone();
    if let Err(error) = locked.save(store_path) {
        return (500, error_result(&format!("Could not save login: {error}")));
    }
    (
        200,
        SyncClientResult {
            ok: true,
            message: "Logged in.".to_string(),
            user_id,
            token,
            mode: "logged_in".to_string(),
            ..SyncClientResult::default()
        },
    )
}

fn handle_sync(
    request: &HttpRequest,
    store: &Arc<Mutex<ServerStore>>,
    store_path: &Path,
) -> (u16, SyncClientResult) {
    handle_sync_mode(request, store, store_path, false)
}

fn handle_sync_mode(
    request: &HttpRequest,
    store: &Arc<Mutex<ServerStore>>,
    store_path: &Path,
    force_upload: bool,
) -> (u16, SyncClientResult) {
    let token = match bearer_token(request) {
        Some(value) => value,
        None => return (401, error_result("Missing sync token.")),
    };
    let snapshot = match serde_json::from_str::<SyncSnapshotRequest>(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid sync request.")),
    };
    let now = now_millis();
    let sanitized_client_json = match sanitize_sync_app_data(&snapshot.app_data_json, now) {
        Some(value) => value,
        None => return (400, error_result("App data is invalid.")),
    };
    let client_revision = app_data_revision_millis(
        &sanitized_client_json,
        snapshot.client_updated_at_epoch_millis,
    );

    let mut locked = match store.lock() {
        Ok(value) => value,
        Err(_) => return (500, error_result("Sync store is unavailable.")),
    };
    let Some(user) = locked.user_by_token_mut(token) else {
        return (401, error_result("Login expired."));
    };
    if let Some(record) = user.tokens.iter_mut().find(|record| record.token == token) {
        record.last_seen_at_epoch_millis = now;
        record.device_name = normalized_device_name(&snapshot.device_name);
    }
    let server_revision = if user.app_data_json.trim().is_empty() {
        0
    } else {
        app_data_revision_millis(&user.app_data_json, user.updated_at_epoch_millis)
    };
    let force_upload = force_upload || snapshot.force_upload;

    let (mode, response_json, response_revision) = if force_upload {
        user.app_data_json = sanitized_client_json.clone();
        user.updated_at_epoch_millis = client_revision.max(now);
        (
            "uploaded_local",
            sanitized_client_json,
            user.updated_at_epoch_millis,
        )
    } else if user.app_data_json.trim().is_empty() || client_revision >= server_revision {
        user.app_data_json = sanitized_client_json.clone();
        user.updated_at_epoch_millis = client_revision.max(now);
        (
            "uploaded",
            sanitized_client_json,
            user.updated_at_epoch_millis,
        )
    } else {
        (
            "downloaded",
            user.app_data_json.clone(),
            server_revision.max(user.updated_at_epoch_millis),
        )
    };
    let user_id = user.id.clone();
    if let Err(error) = locked.save(store_path) {
        return (
            500,
            error_result(&format!("Could not save sync data: {error}")),
        );
    }
    (
        200,
        SyncClientResult {
            ok: true,
            message: match mode {
                "uploaded_local" => "本地数据已同步到账户。".to_string(),
                "uploaded" => "Synced to server.".to_string(),
                "downloaded" => "Pulled newer server data.".to_string(),
                _ => "Synced.".to_string(),
            },
            user_id,
            app_data_json: Some(response_json),
            server_updated_at_epoch_millis: response_revision,
            client_updated_at_epoch_millis: client_revision,
            mode: mode.to_string(),
            ..SyncClientResult::default()
        },
    )
}

#[derive(Default)]
struct HttpRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

fn read_http_request(stream: &mut TcpStream) -> io::Result<HttpRequest> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut buffer = Vec::<u8>::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before headers",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > MAX_REQUEST_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request too large",
            ));
        }
        if let Some(index) = find_header_end(&buffer) {
            break index;
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
    while buffer.len().saturating_sub(body_start) < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len().saturating_sub(body_start) > MAX_REQUEST_BODY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request body too large",
            ));
        }
    }
    let body_bytes =
        &buffer[body_start..body_start + content_length.min(buffer.len() - body_start)];
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
    let body = encode_result(result);
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
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

impl ServerStore {
    fn load(path: &Path) -> io::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str::<Self>(&raw).unwrap_or_default())
    }

    fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = path.with_extension("tmp");
        let encoded = serde_json::to_string_pretty(self)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(&temp_path, encoded)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        fs::rename(temp_path, path)?;
        Ok(())
    }

    fn user_by_token_mut(&mut self, token: &str) -> Option<&mut ServerUser> {
        self.users
            .iter_mut()
            .find(|user| user.tokens.iter().any(|record| record.token == token))
    }
}

fn parse_base_url(server_url: &str) -> Result<ParsedBaseUrl, String> {
    let trimmed = server_url.trim().trim_end_matches('/');
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
    let (host, port) = if let Some((host, port)) = host_port.rsplit_once(':') {
        let parsed_port = port
            .parse::<u16>()
            .map_err(|_| "Sync server port is invalid.".to_string())?;
        (host.trim().to_string(), parsed_port)
    } else {
        (host_port.trim().to_string(), scheme.default_port())
    };
    if host.trim().is_empty() {
        return Err("Sync server host is empty.".to_string());
    }
    Ok(ParsedBaseUrl {
        scheme,
        host,
        port,
        base_path: path,
    })
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
    if base.port == base.scheme.default_port() {
        base.host.clone()
    } else {
        format!("{}:{}", base.host, base.port)
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
    let headers = head
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_string(), value.trim().to_string()))
        })
        .collect::<Vec<_>>();
    let Some(value) = header_value(&headers, "content-length") else {
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

fn normalized_device_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "device".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn password_hash(salt: &str, password: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    hex_bytes(&hasher.finalize())
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
            let ignore_blank_slot_timestamp =
                key == "updatedAt" && is_blank_timer_slot_object(value, map);
            let own = if is_revision_key(key) && !ignore_blank_slot_timestamp {
                value.as_i64().filter(|candidate| *candidate >= 0)
            } else {
                None
            };
            max_option(best, max_option(own, max_revision_in_value(value)))
        }),
        Value::Array(values) => values.iter().fold(None, |best, value| {
            max_option(best, max_revision_in_value(value))
        }),
        _ => None,
    }
}

fn is_revision_key(key: &str) -> bool {
    matches!(
        key,
        "updatedAt"
            | "updatedAtEpochMillis"
            | "endedAtEpochMillis"
            | "archivedAtEpochMillis"
            | "createdAtEpochMillis"
            | "capturedAtEpochMillis"
            | "lastSyncAtEpochMillis"
    )
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
                        && (!note
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

fn encode_result(result: &SyncClientResult) -> String {
    serde_json::to_string(result).unwrap_or_else(|_| {
        "{\"ok\":false,\"message\":\"Could not encode sync result.\"}".to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct ScriptedReader {
        responses: VecDeque<io::Result<Vec<u8>>>,
    }

    impl ScriptedReader {
        fn new(responses: Vec<io::Result<Vec<u8>>>) -> Self {
            Self {
                responses: responses.into(),
            }
        }
    }

    impl Read for ScriptedReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(next) = self.responses.pop_front() else {
                return Ok(0);
            };
            match next {
                Ok(bytes) => {
                    let len = bytes.len().min(buffer.len());
                    buffer[..len].copy_from_slice(&bytes[..len]);
                    Ok(len)
                }
                Err(error) => Err(error),
            }
        }
    }

    #[test]
    fn revision_scans_nested_app_data_timestamps() {
        let raw = r#"{"slots":[{"updatedAt":10}],"notes":[{"updatedAtEpochMillis":25}]}"#;
        assert_eq!(25, app_data_revision_millis(raw, 1));
    }

    #[test]
    fn password_hash_uses_salt() {
        assert_ne!(password_hash("a", "secret"), password_hash("b", "secret"));
        assert_eq!(password_hash("a", "secret"), password_hash("a", "secret"));
    }

    #[test]
    fn base_url_accepts_http_and_https() {
        let local = parse_base_url("http://127.0.0.1:8917").expect("http URL should parse");
        assert_eq!(SyncUrlScheme::Http, local.scheme);
        assert_eq!(8917, local.port);

        let public = parse_base_url("https://sync.example.com/api")
            .expect("https URL should parse");
        assert_eq!(SyncUrlScheme::Https, public.scheme);
        assert_eq!(443, public.port);
        assert_eq!("https://sync.example.com/api", format_base_url(&public));
    }

    #[test]
    fn response_reader_finishes_once_content_length_is_satisfied() {
        let body = "{\"ok\":true,\"message\":\"Logged in.\",\"mode\":\"ok\"}";
        let response = format!(
            concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: application/json\r\n",
                "Content-Length: {}\r\n",
                "Connection: close\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        )
        .into_bytes();
        let split_index = 58;
        let mut reader = ScriptedReader::new(vec![
            Ok(response[..split_index].to_vec()),
            Ok(response[split_index..].to_vec()),
            Err(io::Error::new(io::ErrorKind::TimedOut, "late eof timeout")),
        ]);

        let payload = read_http_response(&mut reader).expect("response should be readable");
        assert_eq!(payload, response);

        let parsed = parse_client_response(&payload);
        assert!(parsed.ok);
        assert_eq!("Logged in.", parsed.message);
    }

    #[test]
    fn lan_discovery_candidates_stay_in_same_subnet() {
        let candidates = sibling_ipv4_candidates(Ipv4Addr::new(192, 168, 1, 23));

        assert_eq!(253, candidates.len());
        assert!(candidates.contains(&Ipv4Addr::new(192, 168, 1, 134)));
        assert!(!candidates.contains(&Ipv4Addr::new(192, 168, 1, 23)));
        assert!(!candidates.contains(&Ipv4Addr::new(192, 168, 2, 134)));
    }

    #[test]
    fn public_hosts_are_not_scanned() {
        assert!(!is_discoverable_lan_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(is_discoverable_lan_ipv4(Ipv4Addr::new(192, 168, 1, 23)));
    }

    #[test]
    fn resolved_server_url_serializes_as_camel_case() {
        let encoded = encode_result(&SyncClientResult {
            ok: true,
            message: "ok".to_string(),
            resolved_server_url: "http://192.168.1.134:8917".to_string(),
            public_server_url: "http://8.8.8.8:8917".to_string(),
            ..SyncClientResult::default()
        });

        assert!(encoded.contains("\"resolvedServerUrl\":\"http://192.168.1.134:8917\""));
        assert!(encoded.contains("\"publicServerUrl\":\"http://8.8.8.8:8917\""));
    }

    #[test]
    fn public_sync_ipv4_rejects_private_and_cgnat_ranges() {
        assert!(is_public_sync_ipv4(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(!is_public_sync_ipv4(Ipv4Addr::new(192, 168, 1, 134)));
        assert!(!is_public_sync_ipv4(Ipv4Addr::new(100, 64, 1, 2)));
    }
}
