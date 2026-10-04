// v0.0.1 - Authenticate the actual loopback peer before sending desktop credentials.

fn desktop_sync_peer_guard() -> sync_core::LocalHttpPeerVerification {
    sync_core::LocalHttpPeerVerification::enforce(verify_desktop_sync_peer)
}

fn verify_desktop_sync_peer(stream: &std::net::TcpStream) -> io::Result<File> {
    let size = option_env!("GRIDTIMER_EXPECTED_SYNC_SERVER_SIZE")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "当前客户端缺少同步服务完整性信息，未发送凭据",
            )
        })?;
    let sha = option_env!("GRIDTIMER_EXPECTED_SYNC_SERVER_SHA256").unwrap_or("");
    let marker = executable_integrity_binding_marker("sync_server", size, sha);
    if !valid_sha256_hex(sha)
        || option_env!("GRIDTIMER_EXPECTED_SYNC_SERVER_BINDING_V1") != Some(marker.as_str())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "同步服务完整性绑定无效，未发送凭据",
        ));
    }
    let executable = std::env::current_exe()?;
    let directory = executable
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "客户端安装目录无效"))?;
    verify_loopback_peer_image(
        stream,
        &directory.join(product_identity::SYNC_SERVER_FILE_NAME),
        size,
        sha,
    )
}

#[cfg(target_os = "windows")]
fn verify_loopback_peer_image(
    stream: &std::net::TcpStream,
    expected: &Path,
    expected_size: u64,
    expected_sha: &str,
) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    let local = stream.local_addr()?;
    let remote = stream.peer_addr()?;
    let denied = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "本机同步连接未通过服务身份核验，未发送凭据",
        )
    };
    if !local.ip().is_loopback()
        || !remote.ip().is_loopback()
        || !valid_sha256_hex(expected_sha)
        || expected_size == 0
        || expected_size > 256 * 1024 * 1024
    {
        return Err(denied());
    }
    let mut owner = None;
    for _ in 0..3 {
        owner = windows_tcp_connected_peer_pid(local, remote);
        if owner.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let owner = owner.ok_or_else(denied)?;
    let metadata = fs::symlink_metadata(expected)?;
    if !metadata.is_file() || namespace_metadata_is_reparse_point(&metadata) {
        return Err(denied());
    }
    let expected = fs::canonicalize(expected)?;
    let actual = fs::canonicalize(query_process_image_path(owner)?)?;
    if windows_path_key(&actual) != windows_path_key(&expected) {
        return Err(denied());
    }
    // Deny write and delete sharing while this request is in flight. A path
    // match or self-reported health document alone is not server authentication.
    let mut image = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&expected)?;
    if image.metadata()?.len() != expected_size {
        return Err(denied());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        gridtimer_native::runtime::cancellation::check_current()?;
        let count = image.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if format!("{:x}", digest.finalize()) != expected_sha.to_ascii_lowercase() {
        return Err(denied());
    }
    // Recheck the connection after hashing, before any request is transmitted.
    if windows_tcp_connected_peer_pid(local, remote) != Some(owner) {
        return Err(denied());
    }
    Ok(image)
}

#[cfg(not(target_os = "windows"))]
fn verify_loopback_peer_image(
    _: &std::net::TcpStream,
    _: &Path,
    _: u64,
    _: &str,
) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "本机同步身份校验需要 Windows",
    ))
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy)]
struct DesktopTcp6OwnerRow {
    local_address: [u8; 16],
    local_scope: u32,
    local_port: u32,
    remote_address: [u8; 16],
    remote_scope: u32,
    remote_port: u32,
    state: u32,
    owner: u32,
}

#[cfg(target_os = "windows")]
fn windows_tcp_connected_peer_pid(
    local: std::net::SocketAddr,
    remote: std::net::SocketAddr,
) -> Option<u32> {
    let family = match (local, remote) {
        (std::net::SocketAddr::V4(_), std::net::SocketAddr::V4(_)) => 2,
        (std::net::SocketAddr::V6(_), std::net::SocketAddr::V6(_)) => 23,
        _ => return None,
    };
    let mut required = 0_u32;
    if unsafe { GetExtendedTcpTable(std::ptr::null_mut(), &mut required, 0, family, 4, 0) } != 122 {
        return None;
    }
    for _ in 0..3 {
        if !(4..=16 * 1024 * 1024).contains(&required) {
            return None;
        }
        let mut table = vec![0_u32; (required as usize + 3) / 4];
        let mut actual = required;
        let status =
            unsafe { GetExtendedTcpTable(table.as_mut_ptr().cast(), &mut actual, 0, family, 4, 0) };
        if status == 122 {
            required = actual;
            continue;
        }
        if status != 0 || actual < 4 || actual as usize > table.len() * 4 {
            return None;
        }
        let count = table[0] as usize;
        let row_size = if family == 2 {
            std::mem::size_of::<MibTcpRowOwnerPid>()
        } else {
            std::mem::size_of::<DesktopTcp6OwnerRow>()
        };
        if 4_usize.checked_add(count.checked_mul(row_size)?)? > actual as usize {
            return None;
        }
        let bytes = table.as_ptr().cast::<u8>();
        let mut owner = None;
        for index in 0..count {
            let address = unsafe { bytes.add(4 + index * row_size) };
            let candidate = match (local, remote) {
                (std::net::SocketAddr::V4(local), std::net::SocketAddr::V4(remote)) => {
                    let row =
                        unsafe { std::ptr::read_unaligned(address.cast::<MibTcpRowOwnerPid>()) };
                    (row.state == 5
                        && row.local_addr.to_ne_bytes() == remote.ip().octets()
                        && row.remote_addr.to_ne_bytes() == local.ip().octets()
                        && u16::from_be(row.local_port as u16) == remote.port()
                        && u16::from_be(row.remote_port as u16) == local.port())
                    .then_some(row.owning_pid)
                }
                (std::net::SocketAddr::V6(local), std::net::SocketAddr::V6(remote)) => {
                    let row =
                        unsafe { std::ptr::read_unaligned(address.cast::<DesktopTcp6OwnerRow>()) };
                    (row.state == 5
                        && row.local_address == remote.ip().octets()
                        && row.remote_address == local.ip().octets()
                        && row.local_scope == remote.scope_id()
                        && row.remote_scope == local.scope_id()
                        && u16::from_be(row.local_port as u16) == remote.port()
                        && u16::from_be(row.remote_port as u16) == local.port())
                    .then_some(row.owner)
                }
                _ => None,
            };
            if let Some(pid) = candidate.filter(|pid| *pid != 0) {
                if owner.is_some_and(|previous| previous != pid) {
                    return None;
                }
                owner = Some(pid);
            }
        }
        return owner;
    }
    None
}
