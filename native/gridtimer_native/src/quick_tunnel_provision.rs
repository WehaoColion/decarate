// v2.22.56 - Persist allocation pacing and provider cooldown before retrying.
// v2.22.45 - Honor the configured proxy and provider cooldown when creating a sync tunnel.
use rand::RngCore;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const API_URL: &str = "https://api.trycloudflare.com/tunnel";
const HEADER_LIMIT: usize = 8_192;
const RESPONSE_LIMIT: u64 = 65_536;

struct Reply {
    status: u16,
    body: Vec<u8>,
    retry_after: Option<u64>,
}

impl Reply {
    fn failure(status: u16) -> Self {
        Self {
            status,
            body: br#"{"success":false,"result":null,"errors":[{"code":10000,"message":"Tunnel provisioning request failed"}]}"#.to_vec(),
            retry_after: None,
        }
    }
}

pub(super) struct ProvisionRelay {
    pub url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Drop for ProvisionRelay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn cooldown_seconds() -> io::Result<u64> {
    super::tunnel_retry::remaining()
}

pub(super) fn start() -> io::Result<ProvisionRelay> {
    // cloudflared's quick-tunnel transport does not consult HTTP(S)_PROXY.
    // This private, short-lived relay accepts only its empty provisioning POST;
    // the destination is fixed and no account/document traffic passes through it.
    let proxy = [
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ]
    .iter()
    .find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
    .map(|value| {
        ureq::Proxy::new(value.trim()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Configured tunnel proxy is invalid",
            )
        })
    })
    .transpose()?;
    let agent = build_agent(proxy);
    start_with(move |user_agent| forward(&agent, user_agent))
}

fn build_agent(proxy: Option<ureq::Proxy>) -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .https_only(true)
        .redirects(0)
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(15));
    if let Some(proxy) = proxy {
        builder = builder.proxy(proxy);
    }
    builder.build()
}

fn start_with<F>(mut forward_request: F) -> io::Result<ProvisionRelay>
where
    F: FnMut(&str) -> Reply + Send + 'static,
{
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let mut nonce = [0_u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let nonce: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();
    let url = format!("http://{}/{nonce}", listener.local_addr()?);
    let expected_path = format!("/{nonce}/tunnel");
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let worker = thread::Builder::new()
        .name("sync-tunnel-provision".into())
        .spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let reply = match read_request(&mut stream, &expected_path) {
                            Ok(user_agent) => forward_request(&user_agent),
                            Err(()) => Reply::failure(400),
                        };
                        let _ = write_reply(&mut stream, reply);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        })?;
    Ok(ProvisionRelay {
        url,
        stop,
        worker: Some(worker),
    })
}

fn read_request(stream: &mut TcpStream, expected_path: &str) -> Result<String, ()> {
    let mut header = Vec::with_capacity(1_024);
    let started = Instant::now();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= HEADER_LIMIT || started.elapsed() > Duration::from_secs(2) {
            return Err(());
        }
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(|_| ())?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header).map_err(|_| ())?;
    let mut lines = header.split("\r\n");
    if lines.next() != Some(format!("POST {expected_path} HTTP/1.1").as_str()) {
        return Err(());
    }
    let mut content_length = None;
    let mut user_agent = String::from("GridTimerSync");
    for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(())?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("transfer-encoding") || name.eq_ignore_ascii_case("expect") {
            return Err(());
        }
        if name.eq_ignore_ascii_case("content-length") {
            if content_length.replace(value).is_some() || value != "0" {
                return Err(());
            }
        }
        if name.eq_ignore_ascii_case("user-agent") {
            if value.len() > 256 || !value.bytes().all(|b| (32..=126).contains(&b)) {
                return Err(());
            }
            user_agent = value.to_owned();
        }
    }
    if content_length != Some("0") {
        return Err(());
    }
    Ok(user_agent)
}

fn forward(agent: &ureq::Agent, user_agent: &str) -> Reply {
    match super::tunnel_retry::request(|| {
        let reply = forward_unchecked(agent, user_agent);
        let deadline = reply.retry_after.map(super::tunnel_retry::deadline_after);
        (reply, deadline)
    }) {
        Ok(Ok(reply)) => reply,
        Ok(Err(remaining)) => Reply {
            retry_after: Some(remaining),
            ..Reply::failure(429)
        },
        Err(error) => {
            super::write_log(&format!("could not preserve tunnel retry state: {error}"));
            Reply::failure(503)
        }
    }
}

fn forward_unchecked(agent: &ureq::Agent, user_agent: &str) -> Reply {
    let response = match agent
        .post(API_URL)
        .set("Content-Type", "application/json")
        .set("User-Agent", user_agent)
        .send_bytes(&[])
    {
        Ok(response) | Err(ureq::Error::Status(_, response)) => response,
        Err(ureq::Error::Transport(_)) => return Reply::failure(502),
    };
    let status = response.status();
    let retry_after = retry_seconds(status, response.header("Retry-After"));
    read_provider_reply(status, retry_after, response.into_reader())
}

fn read_provider_reply(status: u16, retry_after: Option<u64>, reader: impl Read) -> Reply {
    let mut body = Vec::new();
    if reader
        .take(RESPONSE_LIMIT + 1)
        .read_to_end(&mut body)
        .is_err()
        || body.len() as u64 > RESPONSE_LIMIT
    {
        return Reply {
            retry_after,
            ..Reply::failure(502)
        };
    }
    // Keep provider errors bounded and preserve their status. Credentials are
    // returned only to the local child, never logged or persisted by the relay.
    if status == 200 && serde_json::from_slice::<serde_json::Value>(&body).is_err() {
        return Reply::failure(502);
    }
    Reply {
        status,
        body,
        retry_after,
    }
}

fn retry_seconds(status: u16, header: Option<&str>) -> Option<u64> {
    if status != 429 {
        return None;
    }
    Some(
        header
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(60)
            .clamp(1, 86_400),
    )
}

fn write_reply(stream: &mut TcpStream, reply: Reply) -> io::Result<()> {
    write!(stream, "HTTP/1.1 {} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", reply.status, reply.body.len())?;
    if let Some(delay) = reply.retry_after {
        write!(stream, "Retry-After: {delay}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&reply.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn send(relay: &ProvisionRelay, request: &str) -> String {
        let url = url::Url::parse(&relay.url).unwrap();
        let mut stream = TcpStream::connect(("127.0.0.1", url.port().unwrap())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn the_configured_proxy_carries_https_provisioning() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let proxy = ureq::Proxy::new(format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let client = thread::spawn(move || {
            assert!(build_agent(Some(proxy))
                .post("https://upstream.invalid/tunnel")
                .send_bytes(&[])
                .is_err());
        });
        let start = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        && start.elapsed() < Duration::from_secs(3) =>
                {
                    thread::sleep(Duration::from_millis(20))
                }
                other => panic!("HTTPS request did not use the configured proxy: {other:?}"),
            }
        };
        // Windows may inherit the listener's nonblocking mode on accept.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") && header.len() < HEADER_LIMIT {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        assert!(String::from_utf8(header)
            .unwrap()
            .starts_with("CONNECT upstream.invalid:443 HTTP/1.1\r\n"));
        stream
            .write_all(b"HTTP/1.1 502 Test proxy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        drop(stream);
        client.join().unwrap();
    }

    #[test]
    fn only_the_private_empty_post_can_reach_the_provider() {
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let relay = start_with(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Reply::failure(503)
        })
        .unwrap();
        let path = format!("{}/tunnel", url::Url::parse(&relay.url).unwrap().path());
        for request in [
            format!("GET {path} HTTP/1.1\r\nContent-Length: 0\r\n\r\n"),
            "POST /unrelated/tunnel HTTP/1.1\r\nContent-Length: 0\r\n\r\n".to_string(),
            format!("POST {path} HTTP/1.1\r\nContent-Length: 1\r\n\r\n"),
            format!(
                "POST {path} HTTP/1.1\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            format!("POST {path} HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n"),
            format!("POST {path} HTTP/1.1\r\nHost: elsewhere\r\n\r\n"),
        ] {
            assert!(send(&relay, &request).starts_with("HTTP/1.1 400"));
        }
        assert_eq!(0, calls.load(Ordering::SeqCst));
    }

    #[test]
    fn a_valid_provision_request_preserves_the_response_and_user_agent() {
        let relay = start_with(|agent| {
            assert_eq!("cloudflared/test", agent);
            Reply {
                status: 200,
                body: br#"{"success":true,"result":{"id":"test"}}"#.to_vec(),
                retry_after: None,
            }
        })
        .unwrap();
        let url = url::Url::parse(&relay.url).unwrap();
        assert_eq!(Some("127.0.0.1"), url.host_str());
        let response = send(&relay, &format!("POST {}/tunnel HTTP/1.1\r\nContent-Length: 0\r\nUser-Agent: cloudflared/test\r\n\r\n", url.path()));
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.ends_with(r#"{"success":true,"result":{"id":"test"}}"#));
    }

    #[test]
    fn rate_limit_status_and_retry_after_reach_the_child() {
        let relay = start_with(|_| Reply {
            retry_after: Some(2504),
            ..Reply::failure(429)
        })
        .unwrap();
        let path = url::Url::parse(&relay.url).unwrap().path().to_string();
        let response = send(
            &relay,
            &format!("POST {path}/tunnel HTTP/1.1\r\nContent-Length: 0\r\n\r\n"),
        );
        assert!(response.starts_with("HTTP/1.1 429"));
        assert!(response.contains("Retry-After: 2504\r\n"));
        assert_eq!(Some(2504), retry_seconds(429, Some("2504")));
        assert_eq!(Some(60), retry_seconds(429, None));
        assert_eq!(Some(86400), retry_seconds(429, Some("999999999")));
        assert_eq!(None, retry_seconds(200, Some("2504")));
        let oversized = vec![b'x'; RESPONSE_LIMIT as usize + 1];
        let rejected = read_provider_reply(429, Some(2504), oversized.as_slice());
        assert_eq!(502, rejected.status);
        assert_eq!(Some(2504), rejected.retry_after);
    }

    #[test]
    fn dropping_the_relay_closes_its_listener() {
        let relay = start_with(|_| Reply::failure(503)).unwrap();
        let url = url::Url::parse(&relay.url).unwrap();
        let port = url.port().unwrap();
        drop(relay);
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
}
