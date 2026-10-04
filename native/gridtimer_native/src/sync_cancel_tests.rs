// v0.0.1 - Require supervised cancellation to release blocked network workers.
use super::*;
use crate::runtime::task_supervisor::{TaskDurability, TaskKind, TaskSupervisor};
use std::sync::mpsc;

#[test]
fn sync_cancel_during_local_verification_sends_no_credentials() {
    fn cancel_verifier(_: &TcpStream) -> io::Result<std::fs::File> {
        crate::runtime::cancellation::current().unwrap().cancel();
        std::fs::File::open(std::env::current_exe()?)
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = parse_base_url(&format!(
        "http://127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    ))
    .unwrap();
    let server = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(3);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < until =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("credential cancellation fixture failed: {error}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let _ = socket.read_to_end(&mut bytes);
        bytes
    });
    let token = crate::runtime::cancellation::CancellationToken::new();
    let _scope = token.enter();
    let _verifier = LocalHttpPeerVerification::enforce(cancel_verifier);
    let result = send_http_json_with_timeout(
        &base,
        "v1/login",
        Some("synthetic-only"),
        "{\"password\":\"synthetic-only\"}",
        Duration::from_secs(3),
    );
    assert!(server.join().unwrap().is_empty());
    assert!(result.is_err());
}

#[test]
fn sync_cancel_shutdown_releases_stalled_local_request() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = parse_base_url(&format!(
        "http://127.0.0.1:{}",
        listener.local_addr().unwrap().port()
    ))
    .unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut socket = loop {
            match listener.accept() {
                Ok((socket, _)) => break socket,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("isolated cancellation fixture failed: {error}"),
            }
        };
        socket.set_nonblocking(false).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(4)))
            .unwrap();
        assert_eq!(
            read_http_request(&mut socket).unwrap().body,
            "{\"requestId\":\"cancel-only-once\"}"
        );
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_secs(4));
        drop(socket);
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
            "cancelled mutation was replayed"
        );
    });
    let mut supervisor = TaskSupervisor::default();
    supervisor
        .spawn(TaskKind::Sync, TaskDurability::CommitSensitive, move |_| {
            let _ = send_http_json_with_timeout(
                &base,
                "v1/sync",
                Some("synthetic-cancel-token"),
                "{\"requestId\":\"cancel-only-once\"}",
                Duration::from_secs(3),
            );
        })
        .unwrap();
    ready_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    supervisor.begin_shutdown();
    let finished = supervisor.drain_for(Duration::from_millis(800));
    let _ = release_tx.send(());
    server.join().unwrap();
    supervisor.drain_for(Duration::from_secs(4));
    assert_eq!(
        finished.len(),
        1,
        "cancelled network worker remained blocked after shutdown"
    );
    assert!(!finished[0].panicked);
}

#[test]
fn sync_cancel_local_upload_preserves_exact_bytes_and_stops_blocked_write() {
    use sha2::Digest as _;
    for cancel in [false, true] {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = parse_base_url(&format!(
            "http://127.0.0.1:{}",
            listener.local_addr().unwrap().port()
        ))
        .unwrap();
        let body: String = (0..16 * 1024 * 1024)
            .map(|i| char::from(33 + (i % 89) as u8))
            .collect();
        let expected_hash = sha2::Sha256::digest(body.as_bytes());
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let until = Instant::now() + Duration::from_secs(4);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < until =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!("upload fixture failed: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            if cancel {
                let mut prefix = [0; 1024];
                assert!(socket.read(&mut prefix).unwrap() > 0);
                ready_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(5));
            } else {
                ready_tx.send(()).unwrap();
                thread::sleep(Duration::from_millis(500));
                let request = read_http_request(&mut socket).unwrap();
                assert_eq!(request.body.len(), 16 * 1024 * 1024);
                assert_eq!(
                    sha2::Sha256::digest(request.body.as_bytes()),
                    expected_hash,
                    "partial writes changed the request body"
                );
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
            }
            drop(socket);
            assert!(
                listener.accept().is_err(),
                "upload was replayed on a second connection"
            );
        });
        let (tx, rx) = mpsc::channel();
        let mut supervisor = TaskSupervisor::default();
        supervisor
            .spawn(TaskKind::Sync, TaskDurability::CommitSensitive, move |_| {
                let _ = tx.send(send_http_json_with_timeout(
                    &base,
                    "v1/sync",
                    None,
                    &body,
                    Duration::from_secs(4),
                ));
            })
            .unwrap();
        ready_rx.recv_timeout(Duration::from_secs(4)).unwrap();
        let before = Instant::now();
        if cancel {
            supervisor.begin_shutdown();
        }
        let finished = supervisor.drain_for(if cancel {
            Duration::from_millis(800)
        } else {
            Duration::from_secs(6)
        });
        let elapsed = before.elapsed();
        let _ = release_tx.send(());
        server.join().unwrap();
        supervisor.drain_for(Duration::from_secs(5));
        assert_eq!(finished.len(), 1, "upload worker did not finish");
        assert!(!finished[0].panicked);
        let result = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        if cancel {
            assert!(result.is_err());
            assert!(
                elapsed < Duration::from_secs(1),
                "cancelling the upload blocked the caller"
            );
        } else {
            assert!(result.unwrap().ends_with(b"{}"));
        }
    }
}
