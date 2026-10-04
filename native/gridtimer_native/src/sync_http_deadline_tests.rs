// v0.0.1 - Bound slow local peers without replaying writes or leaking expired requests.
use super::*;

fn local_listener() -> (TcpListener, ParsedBaseUrl) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = ParsedBaseUrl {
        scheme: SyncUrlScheme::Http,
        host: "localhost".to_string(),
        port: listener.local_addr().unwrap().port(),
        base_path: String::new(),
    };
    (listener, base)
}

fn accept_fixture(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted Windows sockets can inherit the listener's mode.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(4)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(1)))
                    .unwrap();
                stream.set_nodelay(true).unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("isolated peer did not accept a connection: {error}"),
        }
    }
}

fn timeout_kind(error: &SyncClientIoError) -> io::ErrorKind {
    match error {
        SyncClientIoError::Connect(error)
        | SyncClientIoError::Send(error)
        | SyncClientIoError::Read(error) => error.kind(),
    }
}

#[test]
fn http_deadline_stops_trickled_headers_and_bodies_without_replaying_post() {
    for response_mode in ["header", "content_length", "until_eof"] {
        let (listener, base) = local_listener();
        let worker = thread::spawn(move || {
            let mut stream = accept_fixture(&listener);
            let request = read_http_request(&mut stream).unwrap();
            assert_eq!("/v1/sync", request.path);
            assert_eq!("{\"requestId\":\"synthetic-once\"}", request.body);
            let body = format!("{{\"ok\":true}}{}", " ".repeat(200));
            let header = if response_mode == "until_eof" {
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_string()
            } else {
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len())
            };
            let bytes = if response_mode == "header" {
                format!("{header}{body}").into_bytes()
            } else {
                stream.write_all(header.as_bytes()).unwrap();
                body.into_bytes()
            };
            for byte in bytes {
                if stream.write_all(&[byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(15));
            }
            drop(stream);
            assert!(
                matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
                "a timed-out POST was replayed"
            );
        });
        let started = Instant::now();
        let result = send_http_json_with_timeout(
            &base,
            "v1/sync",
            Some("synthetic-token"),
            "{\"requestId\":\"synthetic-once\"}",
            Duration::from_millis(300),
        );
        let elapsed = started.elapsed();
        worker.join().unwrap();
        let error = result.expect_err("slow response escaped the total deadline");
        assert!(matches!(error, SyncClientIoError::Read(_)), "{error:?}");
        assert!(
            matches!(
                timeout_kind(&error),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ),
            "{error:?}"
        );
        assert!(
            elapsed < Duration::from_millis(1200),
            "slow response escaped the total deadline: {response_mode} {elapsed:?}"
        );
    }
}

#[test]
fn http_deadline_bounds_upload_request_when_peer_does_not_read() {
    let (listener, base) = local_listener();
    let worker = thread::spawn(move || {
        let stream = accept_fixture(&listener);
        thread::sleep(Duration::from_millis(1600));
        drop(stream);
    });
    let payload = "x".repeat(16 * 1024 * 1024);
    let started = Instant::now();
    let result = send_http_json_with_timeout(
        &base,
        "v1/sync",
        Some("synthetic-token"),
        &payload,
        Duration::from_millis(300),
    );
    let elapsed = started.elapsed();
    worker.join().unwrap();
    let error = result.expect_err("blocked upload must stop");
    // Windows may buffer the entire upload before the remote application reads.
    // The shared deadline must end either the write or the subsequent wait.
    assert!(
        matches!(
            error,
            SyncClientIoError::Send(_) | SyncClientIoError::Read(_)
        ),
        "{error:?}"
    );
    assert!(
        matches!(
            timeout_kind(&error),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ),
        "{error:?}"
    );
    assert!(
        elapsed < Duration::from_millis(1200),
        "blocked upload exceeded its budget: {elapsed:?}"
    );
}

#[test]
fn http_deadline_expired_during_verification_sends_no_credentials() {
    fn slow_verifier(_: &TcpStream) -> io::Result<std::fs::File> {
        thread::sleep(Duration::from_millis(250));
        std::fs::File::open(std::env::current_exe()?)
    }
    let (listener, base) = local_listener();
    let worker = thread::spawn(move || {
        let mut stream = accept_fixture(&listener);
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let _guard = LocalHttpPeerVerification::enforce(slow_verifier);
    let result = send_http_json_with_timeout(
        &base,
        "v1/login",
        Some("synthetic-token"),
        "{\"password\":\"synthetic\"}",
        Duration::from_millis(100),
    );
    let sent = worker.join().unwrap();
    assert!(
        sent.is_empty(),
        "expired verification transmitted credentials"
    );
    assert_eq!(io::ErrorKind::TimedOut, timeout_kind(&result.unwrap_err()));
}

#[test]
fn http_deadline_logout_accepts_revocation_and_bounds_stalled_peer() {
    let (listener, base) = local_listener();
    let worker = thread::spawn(move || {
        let mut stream = accept_fixture(&listener);
        assert_eq!("/v1/logout", read_http_request(&mut stream).unwrap().path);
        stream
            .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\n\r\n{}")
            .unwrap();
    });
    assert!(revoke_loopback_token_once(
        &format_base_url(&base),
        "synthetic-token"
    ));
    worker.join().unwrap();

    let (listener, base) = local_listener();
    let worker = thread::spawn(move || {
        let mut stream = accept_fixture(&listener);
        assert_eq!("/v1/logout", read_http_request(&mut stream).unwrap().path);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 400\r\n\r\n")
            .unwrap();
        for _ in 0..260 {
            if stream.write_all(b" ").is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
    let started = Instant::now();
    let revoked = revoke_loopback_token_once(&format_base_url(&base), "synthetic-token");
    let elapsed = started.elapsed();
    worker.join().unwrap();
    assert!(!revoked, "unfinished response must not acknowledge logout");
    assert!(
        elapsed < Duration::from_millis(11500),
        "logout did not use its shorter deadline: {elapsed:?}"
    );
}
