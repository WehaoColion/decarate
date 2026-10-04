// v0.0.1 - Cancel Windows HTTPS requests through asynchronous WinHTTP while retaining callback buffers.
use crate::runtime::cancellation;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::c_void;
use std::io;
use std::ptr;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

type Callback = unsafe extern "system" fn(*mut c_void, usize, u32, *mut c_void, u32);
#[link(name = "winhttp")]
unsafe extern "system" {
    fn WinHttpOpen(
        agent: *const u16,
        access: u32,
        proxy: *const u16,
        bypass: *const u16,
        flags: u32,
    ) -> *mut c_void;
    fn WinHttpConnect(
        session: *mut c_void,
        host: *const u16,
        port: u16,
        reserved: u32,
    ) -> *mut c_void;
    fn WinHttpOpenRequest(
        connection: *mut c_void,
        verb: *const u16,
        object: *const u16,
        version: *const u16,
        referer: *const u16,
        accept: *const *const u16,
        flags: u32,
    ) -> *mut c_void;
    fn WinHttpCloseHandle(handle: *mut c_void) -> i32;
    fn WinHttpSetOption(handle: *mut c_void, option: u32, data: *const c_void, size: u32) -> i32;
    fn WinHttpSetTimeouts(
        handle: *mut c_void,
        resolve: i32,
        connect: i32,
        send: i32,
        receive: i32,
    ) -> i32;
    fn WinHttpSetStatusCallback(
        handle: *mut c_void,
        callback: Callback,
        flags: u32,
        reserved: usize,
    ) -> usize;
    fn WinHttpSendRequest(
        handle: *mut c_void,
        headers: *const u16,
        header_length: u32,
        body: *const c_void,
        body_length: u32,
        total_length: u32,
        context: usize,
    ) -> i32;
    fn WinHttpReceiveResponse(handle: *mut c_void, reserved: *mut c_void) -> i32;
    fn WinHttpQueryHeaders(
        handle: *mut c_void,
        level: u32,
        name: *const u16,
        buffer: *mut c_void,
        size: *mut u32,
        index: *mut u32,
    ) -> i32;
    fn WinHttpReadData(
        handle: *mut c_void,
        buffer: *mut c_void,
        count: u32,
        received: *mut u32,
    ) -> i32;
}

const HANDLE_CLOSING: u32 = 0x800;
const HEADERS: u32 = 0x20000;
const READ: u32 = 0x80000;
const ERROR: u32 = 0x200000;
const SENT: u32 = 0x400000;
const READ_CAPACITY: usize = 64 * 1024;

struct Handle(usize);
impl Handle {
    fn new(raw: *mut c_void) -> io::Result<Self> {
        if raw.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(Self(raw as usize))
        }
    }
    fn raw(&self) -> *mut c_void {
        self.0 as *mut c_void
    }
    fn option<T>(&self, option: u32, value: &T) -> io::Result<()> {
        checked(unsafe {
            WinHttpSetOption(
                self.raw(),
                option,
                (value as *const T).cast(),
                std::mem::size_of::<T>() as u32,
            )
        })
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.raw());
        }
    }
}

fn checked(ok: i32) -> io::Result<()> {
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn win_error(code: u32) -> io::Error {
    match code {
        12002 => io::Error::new(io::ErrorKind::TimedOut, "sync request timed out"),
        12017 => cancellation::cancelled_error(),
        _ => io::Error::from_raw_os_error(code as i32),
    }
}

fn session(local: bool, secure: bool) -> io::Result<Arc<Handle>> {
    // WinHTTP sessions retain their connection/TLS pools across requests.
    static DIRECT: OnceLock<Result<Arc<Handle>, i32>> = OnceLock::new();
    static SYSTEM: OnceLock<Result<Arc<Handle>, i32>> = OnceLock::new();
    static LOCAL_HEALTH: OnceLock<Result<Arc<Handle>, i32>> = OnceLock::new();
    let slot = match (local, secure) {
        (true, false) => &LOCAL_HEALTH,
        (true, true) => &DIRECT,
        (false, true) => &SYSTEM,
        (false, false) => return Err(invalid("plaintext transport must stay local")),
    };
    match slot.get_or_init(|| {
        let agent = wide("TenRateSync/1");
        // Secure defaults require TLS 1.2+ and reject plaintext OpenRequest.
        // Keep the unauthenticated loopback health session separate from HTTPS.
        Handle::new(unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                if local { 1 } else { 4 },
                ptr::null(),
                ptr::null(),
                if secure { 0x30000000 } else { 0x10000000 },
            )
        })
        .map(Arc::new)
        .map_err(|e| e.raw_os_error().unwrap_or(1))
    }) {
        Ok(handle) => Ok(handle.clone()),
        Err(code) => Err(io::Error::from_raw_os_error(*code)),
    }
}

enum Event {
    Sent,
    Headers,
    Read(Vec<u8>),
    Error(u32),
}
struct State {
    events: Mutex<VecDeque<Event>>,
    ready: Condvar,
    // The registry reference belongs to the request handle. These buffers
    // remain allocated after cancellation until HANDLE_CLOSING, the last callback.
    body: Box<[u8]>,
    headers: Vec<u16>,
    read_buffer: Mutex<Box<[u8]>>,
    _connection: Handle,
    _session: Arc<Handle>,
}

fn contexts() -> &'static Mutex<BTreeMap<usize, Arc<State>>> {
    static CONTEXTS: OnceLock<Mutex<BTreeMap<usize, Arc<State>>>> = OnceLock::new();
    CONTEXTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn track_context(state: Arc<State>) -> io::Result<usize> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    let id = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| io::Error::other("request context identifiers exhausted"))?;
    contexts()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, state);
    Ok(id)
}

fn release_context(id: usize) {
    // Drop handles after releasing the registry lock, including on callbacks.
    let state = contexts()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    drop(state);
}
#[cfg(test)]
static LIVE_CONTEXTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
impl Drop for State {
    fn drop(&mut self) {
        LIVE_CONTEXTS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
#[cfg(test)]
pub(super) fn live_contexts() -> usize {
    LIVE_CONTEXTS.load(std::sync::atomic::Ordering::SeqCst)
}

#[repr(C)]
struct AsyncResult {
    result: usize,
    error: u32,
}

unsafe extern "system" fn callback(
    _: *mut c_void,
    context: usize,
    status: u32,
    info: *mut c_void,
    length: u32,
) {
    if context == 0 {
        return;
    }
    if status == HANDLE_CLOSING {
        release_context(context);
        return;
    }
    // An integer context never dereferences freed memory. Earlier callbacks
    // retain their own Arc if the final close notification overlaps them.
    let Some(state) = contexts()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&context)
        .cloned()
    else {
        return;
    };
    let event = match status {
        SENT => Event::Sent,
        HEADERS => Event::Headers,
        READ if length as usize <= READ_CAPACITY && (length == 0 || !info.is_null()) => {
            let bytes = if length == 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(info.cast::<u8>(), length as usize) }.to_vec()
            };
            Event::Read(bytes)
        }
        READ => Event::Error(13),
        ERROR if !info.is_null() && length as usize >= std::mem::size_of::<AsyncResult>() => {
            Event::Error(unsafe { ptr::read_unaligned(info.cast::<AsyncResult>()) }.error)
        }
        ERROR => Event::Error(13),
        _ => return,
    };
    let mut events = state.events.lock().unwrap_or_else(|e| e.into_inner());
    events.push_back(event);
    state.ready.notify_all();
}

impl State {
    fn wait(&self, deadline: Instant) -> io::Result<Event> {
        let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            cancellation::check_current()?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "sync request exceeded its total deadline",
                ));
            }
            if let Some(event) = events.pop_front() {
                return match event {
                    Event::Error(code) => Err(win_error(code)),
                    event => Ok(event),
                };
            }
            events = self
                .ready
                .wait_timeout(events, remaining)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

/// Windows-only bounded transport. Public account operations require HTTPS;
/// unauthenticated local GET is reserved for health checks.
pub(crate) fn request(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: &str,
    timeout: Duration,
    limit: usize,
) -> io::Result<(u16, String)> {
    cancellation::check_current()?;
    let base = url::Url::parse(url).map_err(|_| invalid("invalid sync request URL"))?;
    let local = is_local(&base);
    if base.scheme() != "https"
        && !(base.scheme() == "http"
            && local
            && method == "GET"
            && bearer.is_none()
            && body.is_empty())
    {
        return Err(invalid("account requests require HTTPS"));
    }
    run(method, &base, bearer, body, timeout, limit)
}

fn is_local(base: &url::Url) -> bool {
    base.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

fn run(
    method: &str,
    base: &url::Url,
    bearer: Option<&str>,
    body: &str,
    timeout: Duration,
    limit: usize,
) -> io::Result<(u16, String)> {
    cancellation::check_current()?;
    if !matches!(method, "GET" | "POST")
        || !base.username().is_empty()
        || base.password().is_some()
        || base.fragment().is_some()
        || body.len() > super::MAX_REQUEST_BODY_BYTES
    {
        return Err(invalid("invalid sync request parameters"));
    }
    if bearer.is_some_and(|value| value.contains(['\r', '\n', '\0'])) {
        return Err(invalid("invalid bearer token"));
    }
    let deadline = Instant::now() + timeout;
    let host = wide(
        base.host_str()
            .ok_or_else(|| invalid("missing sync host"))?
            .trim_matches(['[', ']']),
    );
    let session = session(is_local(base), base.scheme() == "https")?;
    let connection = Handle::new(unsafe {
        WinHttpConnect(
            session.raw(),
            host.as_ptr(),
            base.port_or_known_default()
                .ok_or_else(|| invalid("missing sync port"))?,
            0,
        )
    })?;
    let verb = wide(method);
    let path = wide(&format!(
        "{}{}",
        base.path(),
        base.query().map(|q| format!("?{q}")).unwrap_or_default()
    ));
    let request = Handle::new(unsafe {
        WinHttpOpenRequest(
            connection.raw(),
            verb.as_ptr(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            if base.scheme() == "https" {
                0x800000
            } else {
                0
            },
        )
    })?;
    // No cookies, implicit Windows credentials, or redirects between origins.
    request.option(63, &7_u32)?;
    request.option(77, &2_u32)?;
    request.option(88, &0_u32)?;
    request.option(91, &(super::MAX_HEADER_BYTES as u32))?;
    let millis = timeout.as_millis().clamp(1, i32::MAX as u128) as i32;
    checked(unsafe {
        WinHttpSetTimeouts(
            request.raw(),
            millis.min(6000),
            millis.min(6000),
            millis.min(20000),
            millis.min(90000),
        )
    })?;
    let mut headers = "Accept: application/json\r\nContent-Type: application/json\r\n".to_string();
    if let Some(token) = bearer {
        headers.push_str(&format!("Authorization: Bearer {}\r\n", token.trim()));
    }
    #[cfg(test)]
    LIVE_CONTEXTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let state = Arc::new(State {
        events: Mutex::new(VecDeque::new()),
        ready: Condvar::new(),
        body: body.as_bytes().into(),
        headers: wide(&headers),
        read_buffer: Mutex::new(vec![0; READ_CAPACITY].into_boxed_slice()),
        _connection: connection,
        _session: session,
    });
    let context = track_context(state.clone())?;
    if let Err(error) = request.option(45, &context) {
        release_context(context);
        return Err(error);
    }
    if unsafe {
        WinHttpSetStatusCallback(
            request.raw(),
            callback,
            0xC00 | HEADERS | READ | ERROR | SENT,
            0,
        )
    } == usize::MAX
    {
        let error = io::Error::last_os_error();
        release_context(context);
        return Err(error);
    }
    let _wake = cancellation::current().map(|token| {
        let state = state.clone();
        token.on_cancel(move || {
            let _events = state.events.lock().unwrap_or_else(|e| e.into_inner());
            state.ready.notify_all();
        })
    });
    cancellation::check_current()?;
    let body_pointer = if state.body.is_empty() {
        ptr::null()
    } else {
        state.body.as_ptr().cast()
    };
    checked(unsafe {
        WinHttpSendRequest(
            request.raw(),
            state.headers.as_ptr(),
            (state.headers.len() - 1) as u32,
            body_pointer,
            state.body.len() as u32,
            state.body.len() as u32,
            context,
        )
    })?;
    if !matches!(state.wait(deadline)?, Event::Sent) {
        return Err(invalid("unexpected send completion"));
    }
    cancellation::check_current()?;
    checked(unsafe { WinHttpReceiveResponse(request.raw(), ptr::null_mut()) })?;
    if !matches!(state.wait(deadline)?, Event::Headers) {
        return Err(invalid("unexpected header completion"));
    }
    let mut status = 0_u32;
    let mut size = 4_u32;
    checked(unsafe {
        WinHttpQueryHeaders(
            request.raw(),
            19 | 0x20000000,
            ptr::null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            ptr::null_mut(),
        )
    })?;
    let status = u16::try_from(status).map_err(|_| invalid("invalid HTTP status"))?;
    let mut output = Vec::new();
    loop {
        cancellation::check_current()?;
        // Only one read is active; its allocation is never moved or read by
        // Rust while WinHTTP owns the buffer. The completion callback copies it.
        let buffer = state
            .read_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut_ptr();
        checked(unsafe {
            WinHttpReadData(
                request.raw(),
                buffer.cast(),
                READ_CAPACITY as u32,
                ptr::null_mut(),
            )
        })?;
        let Event::Read(bytes) = state.wait(deadline)? else {
            return Err(invalid("unexpected body completion"));
        };
        if bytes.is_empty() {
            break;
        }
        if bytes.len() > limit.saturating_sub(output.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sync response exceeds the configured body limit",
            ));
        }
        output.extend_from_slice(&bytes);
    }
    cancellation::check_current()?;
    let body =
        String::from_utf8(output).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((status, body))
    // Only this worker closes its request, after the last WinHTTP function has
    // returned. Closing while an async operation is pending requests cancellation.
    // The registry Arc, including body/read buffers and parents, lives until the
    // final HANDLE_CLOSING callback even if this worker has already returned.
}

#[cfg(test)]
pub(super) fn fixture_request(
    method: &str,
    url: &str,
    bearer: Option<&str>,
    body: &str,
    timeout: Duration,
    limit: usize,
) -> io::Result<(u16, String)> {
    let base = url::Url::parse(url).unwrap();
    assert!(is_local(&base));
    run(method, &base, bearer, body, timeout, limit)
}

#[cfg(test)]
#[path = "sync_windows_http_tests.rs"]
mod tests;
