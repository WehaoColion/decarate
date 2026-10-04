// v0.0.1 - Exercise native request cancellation, response bounds and one-shot mutations.
use super::*;
use crate::runtime::task_supervisor::{TaskDurability, TaskKind, TaskSupervisor};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

fn fixture() -> (TcpListener, String) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    (listener, url)
}
fn accept(listener: &TcpListener) -> TcpStream {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                return socket;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < until => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("native fixture accept failed: {error}"),
        }
    }
}
fn all_contexts_released() {
    let until = Instant::now() + Duration::from_secs(4);
    while live_contexts() != 0 && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        live_contexts(),
        0,
        "native callback context or its buffers remained retained"
    );
}

#[test]
fn sync_cancel_native_transport_contract() {
    // Before transmission, cancellation prevents any connection/credential disclosure.
    let (listener, url) = fixture();
    let token = cancellation::CancellationToken::new();
    token.cancel();
    {
        let _scope = token.enter();
        assert_eq!(
            fixture_request(
                "POST",
                &url,
                Some("synthetic-only"),
                "{}",
                Duration::from_secs(3),
                64
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::ConnectionAborted
        );
    }
    assert!(listener.accept().is_err());
    drop(listener);

    for phase in ["headers", "health", "body", "upload", "tls"] {
        let (listener, mut url) = fixture();
        if phase == "tls" {
            url = url.replacen("http://", "https://", 1);
        }
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut socket = accept(&listener);
            if matches!(phase, "headers" | "health" | "body") {
                let request = super::super::read_http_request(&mut socket).unwrap();
                if phase == "health" {
                    assert_eq!(request.path, "/health");
                    assert!(request.body.is_empty());
                } else {
                    assert_eq!(request.body, "{\"id\":\"one-shot\"}");
                }
                if phase == "body" {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n{").unwrap();
                }
            } else {
                let mut first = [0; 1024];
                let count = socket.read(&mut first).unwrap();
                assert!(count > 0);
                if phase == "tls" {
                    assert_eq!(
                        first[0], 22,
                        "expected a TLS handshake, never plaintext credentials"
                    );
                }
            }
            ready_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(5));
            drop(socket);
            assert!(
                matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
                "native transport replayed a cancelled POST"
            );
        });
        let mut supervisor = TaskSupervisor::default();
        let (result_tx, result_rx) = mpsc::channel();
        supervisor
            .spawn(TaskKind::Sync, TaskDurability::CommitSensitive, move |_| {
                let body = if phase == "upload" {
                    "x".repeat(16 * 1024 * 1024)
                } else {
                    "{\"id\":\"one-shot\"}".to_string()
                };
                let result = if phase == "health" {
                    super::super::windows_health_response(&url, Duration::from_secs(4), 8192)
                } else {
                    fixture_request(
                        "POST",
                        &url,
                        Some("synthetic-only"),
                        &body,
                        Duration::from_secs(4),
                        8192,
                    )
                };
                let _ = result_tx.send(result);
            })
            .unwrap();
        let ready = ready_rx.recv_timeout(Duration::from_secs(5));
        supervisor.cancel_kinds(&[TaskKind::Sync]);
        let completed = supervisor.drain_for(Duration::from_millis(800));
        let _ = release_tx.send(());
        let server_result = server.join();
        supervisor.drain_for(Duration::from_secs(5));
        assert!(
            ready.is_ok(),
            "native phase {phase} never reached network I/O; result={:?}",
            result_rx.try_recv()
        );
        server_result.unwrap();
        assert_eq!(completed.len(), 1, "cancel failed during {phase}");
        assert!(!completed[0].panicked);
        assert_eq!(
            result_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::ConnectionAborted,
            "phase {phase}"
        );
        all_contexts_released();
    }

    // A cancelled request must not poison the reusable session. Cover length,
    // chunking, invalid text and non-success statuses on fresh connections.
    for (response, limit, expected) in [
        (b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".as_slice(), 2, Some((200, "{}"))),
        (b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\n{}\r\n0\r\n\r\n".as_slice(), 2, Some((200, "{}"))),
        (b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".as_slice(), 2, Some((401, "{}"))),
        (b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nxxx".as_slice(), 2, None),
        (b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n\xff".as_slice(), 8, None),
    ] {
        let (listener, url) = fixture();
        let server = thread::spawn(move || {
            let mut socket = accept(&listener);
            super::super::read_http_request(&mut socket).unwrap();
            socket.write_all(response).unwrap();
        });
        let result = fixture_request("POST", &url, None, "{}", Duration::from_secs(3), limit);
        server.join().unwrap();
        match expected {
            Some((status, body)) => assert_eq!(result.unwrap(), (status, body.to_string())),
            None => assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData),
        }
        all_contexts_released();
    }
    for redirect in [false, true] {
        let (listener, url) = fixture();
        let location = url.clone();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut socket = accept(&listener);
            super::super::read_http_request(&mut socket).unwrap();
            if redirect {
                write!(socket, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
            drop(socket);
            let _ = release_rx.recv_timeout(Duration::from_secs(4));
            assert!(
                listener.accept().is_err(),
                "ambiguous response or redirect replayed the mutation"
            );
        });
        let result = fixture_request(
            "POST",
            &url,
            Some("synthetic-only"),
            "{}",
            Duration::from_secs(2),
            32,
        );
        let _ = release_tx.send(());
        server.join().unwrap();
        if redirect {
            assert_eq!(result.unwrap().0, 307);
        } else {
            assert!(result.is_err());
        }
        all_contexts_released();
    }
    // A total deadline terminates a healthy TCP connection whose server never replies.
    let (listener, url) = fixture();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut socket = accept(&listener);
        super::super::read_http_request(&mut socket).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(3));
    });
    let before = Instant::now();
    let result = fixture_request("POST", &url, None, "{}", Duration::from_millis(300), 32);
    let elapsed = before.elapsed();
    let _ = release_tx.send(());
    server.join().unwrap();
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert!(elapsed < Duration::from_secs(2));
    all_contexts_released();

    // An untrusted certificate must fail before any account HTTP bytes are sent.
    // The synthetic key is only a test fixture and is never installed in Windows.
    let (listener, url) = fixture();
    let server = thread::spawn(move || {
        use ureq::rustls::{
            self,
            pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
        };
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(
                include_bytes!("testdata/sync_tls/untrusted_cert.der").to_vec(),
            )],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                include_bytes!("testdata/sync_tls/synthetic_key.der").to_vec(),
            )),
        )
        .unwrap();
        let socket = accept(&listener);
        let connection = rustls::ServerConnection::new(Arc::new(config)).unwrap();
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut application = [0; 128];
        let result = stream.read(&mut application);
        assert!(
            !matches!(result, Ok(count) if count > 0),
            "credentials crossed an untrusted TLS connection"
        );
    });
    let result = request(
        "POST",
        &url.replacen("http://", "https://", 1),
        Some("synthetic-only"),
        "{}",
        Duration::from_secs(4),
        32,
    );
    server.join().unwrap();
    assert!(result.is_err());
    all_contexts_released();

    // Optional read-only check of the already running real public service.
    if let Ok(url) = std::env::var("TENRATE_NATIVE_HEALTH_URL") {
        let (status, body) = request(
            "GET",
            &format!("{}/health", url.trim_end_matches('/')),
            None,
            "",
            Duration::from_secs(15),
            65536,
        )
        .unwrap();
        let health: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status, 200);
        assert_eq!(health["ok"], true);
        assert_eq!(health["productId"], "gridtimer");
        assert_eq!(health["serviceRole"], "sync_server");
        all_contexts_released();
    }
}
