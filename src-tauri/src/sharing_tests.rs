// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Unit tests for `sharing`, kept beside it rather than inline.
use super::*;
use axum::http::HeaderValue;
use std::{sync::mpsc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn activation_commits_a_script_free_strict_cookie_landing() {
    let uri: Uri = "/.kiwix-access/secret?path=%2FA%2F%22%3C%26"
        .parse()
        .unwrap();
    let target = activation_target(&uri).unwrap();
    let response = activation_page(&Localizer::new("en", ""), &target, 8080, "secret");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert!(!headers.contains_key("location"));
    assert!(!headers.contains_key("refresh"));
    assert_eq!(
        headers["set-cookie"],
        "kiwix_share_8080=secret; Path=/; HttpOnly; SameSite=Strict"
    );
    assert_eq!(headers["referrer-policy"], "no-referrer");
    assert_eq!(headers["cache-control"], "no-store");
    assert!(headers["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("default-src 'none'"));
    assert!(headers["permissions-policy"]
        .to_str()
        .unwrap()
        .contains("geolocation=()"));
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let html = std::str::from_utf8(&bytes).unwrap();
    assert!(html.contains("<a href=\"/A/&quot;&lt;&amp;\" rel=\"noreferrer\">"));
    assert!(!html.contains("<script"));
    assert!(!html.contains("http-equiv"));
    assert!(!html.contains("secret"));
    assert_eq!(
        activation_target(&"/.kiwix-access/secret".parse().unwrap()).unwrap(),
        "/"
    );
    for query in [
        "path=%2F%2Fevil.example",
        "path=%2F%5Cevil.example",
        "path=%2FA%3Fx",
        "path=%2FA%23x",
        "path=%2FA%0Ax",
        "path=https%3A%2F%2Fevil.example",
        "path=%2FA&path=%2FB",
        "other=x",
    ] {
        let uri = format!("/.kiwix-access/secret?{query}").parse().unwrap();
        assert!(activation_target(&uri).is_err(), "{query}");
    }
}

#[test]
fn revocation_drains_native_reads_and_rejects_queued_work() {
    let cancellation = SharingCancellation::new();
    let reading = cancellation.clone();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let native = std::thread::spawn(move || {
        reading
            .native(|| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .unwrap()
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let stopping = cancellation.clone();
    let (stopped_tx, stopped_rx) = mpsc::channel();
    let stop = std::thread::spawn(move || {
        stopping.cancel();
        stopped_tx.send(()).unwrap();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !cancellation.is_cancelled() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(matches!(
        stopped_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));
    release_tx.send(()).unwrap();
    stopped_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    native.join().unwrap();
    stop.join().unwrap();
    let result = cancellation.native(|| panic!("archive read must not start after stop"));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
}

#[tokio::test]
async fn cancellation_wakes_pending_socket_io_and_stays_cancelled() {
    let cancellation = SharingCancellation::new();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (socket, _) = listener.accept().await.unwrap();
    let permit = cancellation.0.connections.clone().try_acquire_owned().unwrap();
    let mut socket = SharingIo::new(socket, &cancellation, permit, SHARING_TIMEOUTS);
    let reading = tokio::spawn(async move {
        let mut byte = [0];
        assert_eq!(
            socket.read(&mut byte).await.unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
        // Do not poll an already-completed cancellation future again.
        assert_eq!(
            socket.write(b"late bytes").await.unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
    });
    tokio::task::yield_now().await;
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(2), reading)
        .await
        .unwrap()
        .unwrap();
    drop(client);
}

fn short_timeouts(request: u64, body: u64, idle: u64) -> SharingTimeouts {
    SharingTimeouts {
        request: Duration::from_millis(request),
        body: Duration::from_millis(body),
        idle: Duration::from_millis(idle),
    }
}

/// A loopback pair whose server side owns one admission permit.
async fn connected(
    cancellation: &SharingCancellation,
    timeouts: SharingTimeouts,
) -> (TcpStream, SharingIo) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
    let (socket, _) = listener.accept().await.unwrap();
    let permit = cancellation.0.connections.clone().try_acquire_owned().unwrap();
    (client, SharingIo::new(socket, cancellation, permit, timeouts))
}

#[test]
fn request_head_and_body_framing_is_conservative() {
    assert_eq!(request_head_end(b"GET / HTTP/1.1\r\n\r\n"), Some(18));
    assert_eq!(request_head_end(b"GET / HTTP/1.1\n\n"), Some(16));
    assert_eq!(request_head_end(b"GET / HTTP/1.1\r\nHost: x"), None);
    assert_eq!(declared_body(b"GET / HTTP/1.1\r\n\r\n"), DeclaredBody::None);
    assert_eq!(
        declared_body(b"POST / HTTP/1.1\r\nContent-Length: 12\r\n\r\n"),
        DeclaredBody::Length(12)
    );
    assert_eq!(
        declared_body(b"POST / HTTP/1.1\r\nContent-Length: 0\r\n\r\n"),
        DeclaredBody::None
    );
    assert_eq!(
        declared_body(b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"),
        DeclaredBody::Unaccounted
    );
    assert_eq!(
        declared_body(b"POST / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n"),
        DeclaredBody::Unaccounted,
        "a repeated length is Hyper's to reject"
    );
    assert_eq!(
        declared_body(b"POST / HTTP/1.1\r\nContent-Length: nope\r\n\r\n"),
        DeclaredBody::Unaccounted
    );
}

/// A chunked (unaccounted) body may still be draining after the response
/// starts, so its bytes must never be mistaken for a next request and must
/// never arm a request budget again.
#[tokio::test]
async fn sharing_chunked_body_never_reenters_a_request_phase() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(200, 200, 5000)).await;
    socket.received(
        b"POST /nearby/session HTTP/1.1\r\nHost: 127.0.0.1\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    assert!(matches!(socket.request, RequestState::Unaccounted));
    assert!(
        socket.request_deadline.is_none(),
        "an unaccounted body has no request budget"
    );
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();
    assert!(socket.response_idle.is_some(), "the response bound is armed");
    // Chunked framing and a following request head both stay unaccounted.
    socket.received(b"5\r\nhello\r\n0\r\n\r\n");
    socket.received(b"GET /secret/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    assert!(matches!(socket.request, RequestState::Unaccounted));
    assert!(socket.request_deadline.is_none());
    // Past the 200 ms request budget but inside the idle bound: reads still
    // succeed, proving no short request deadline was armed.
    tokio::time::sleep(Duration::from_millis(400)).await;
    client.write_all(b"next").await.unwrap();
    let mut body = [0; 16];
    assert_eq!(socket.read(&mut body).await.unwrap(), 4);
}

/// While the next request's head is incomplete, the previous response's idle
/// bound is still the governing deadline.
#[tokio::test]
async fn sharing_next_request_keeps_the_response_bound_until_its_head_completes() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(5000, 5000, 1000)).await;
    let head = b"GET /secret/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    client.write_all(head).await.unwrap();
    let mut received = vec![0; head.len()];
    socket.read_exact(&mut received).await.unwrap();
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();
    socket.flush().await.unwrap();
    client
        .write_all(b"GET /secret/book HTTP/1.1\r\nHost: 127.0.0.1")
        .await
        .unwrap();
    let mut body = [0; 64];
    assert!(socket.read(&mut body).await.unwrap() > 0);
    assert!(
        socket.request_deadline.is_some(),
        "a fresh head budget is armed for the next request"
    );
    // The 5 s head budget would hide a lost response bound; the 1 s idle
    // bound is what must close this connection.
    let error = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut body))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(error.to_string(), "Sharing connection is idle");
}

/// A head larger than the storage cap is still scanned for its terminator so a
/// completed head is not mistaken for a stalled one, but its framing is never
/// parsed and the connection never re-enters a request phase.
#[tokio::test]
async fn sharing_oversized_head_is_scanned_but_never_parsed() {
    let cancellation = SharingCancellation::new();
    let (_client, mut socket) = connected(&cancellation, short_timeouts(2000, 2000, 5000)).await;
    let filler = vec![b'x'; SHARING_HEAD_BYTES + 4096];
    socket.received(&filler);
    assert!(socket.head_truncated);
    assert!(matches!(socket.request, RequestState::Head));
    assert!(
        socket.request_deadline.is_some(),
        "an incomplete head keeps its budget past the storage cap"
    );
    socket.received(b"\r\n\r\n");
    assert!(matches!(socket.request, RequestState::Unaccounted));
    assert!(socket.request_deadline.is_none());
    socket.received(b"GET /secret/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
    assert!(
        matches!(socket.request, RequestState::Unaccounted),
        "an oversized head's connection never re-enters a request phase"
    );
}

/// A terminator spanning the storage boundary is still detected.
#[tokio::test]
async fn sharing_head_terminator_spanning_the_storage_cap_is_found() {
    let cancellation = SharingCancellation::new();
    let (_client, mut socket) = connected(&cancellation, short_timeouts(2000, 2000, 5000)).await;
    let mut first = vec![b'x'; SHARING_HEAD_BYTES - 1];
    first.extend_from_slice(b"\r\n");
    socket.received(&first);
    assert!(socket.head_truncated);
    socket.received(b"\r\n");
    assert!(matches!(socket.request, RequestState::Unaccounted));
}

/// An oversized head that never terminates still releases its permit through
/// the absolute head budget.
#[tokio::test]
async fn sharing_incomplete_oversized_head_still_times_out() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(1000, 1000, 5000)).await;
    let filler = vec![b'x'; SHARING_HEAD_BYTES + 4096];
    let writer = tokio::spawn(async move {
        client.write_all(&filler).await.unwrap();
        client
    });
    let mut buffer = vec![0; SHARING_HEAD_BYTES + 4096];
    socket.read_exact(&mut buffer).await.unwrap();
    let _client = writer.await.unwrap();
    assert!(socket.head_truncated);
    let error = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut buffer))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn sharing_socket_releases_capacity_after_head_timeout() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(1000, 1000, 1000)).await;
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: x")
        .await
        .unwrap();
    let mut body = [0; 64];
    assert!(socket.read(&mut body).await.unwrap() > 0);
    assert_eq!(
        cancellation.0.connections.available_permits(),
        SHARING_MAX_CONNECTIONS - 1
    );
    let error = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut body))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    drop(socket);
    assert_eq!(
        cancellation.0.connections.available_permits(),
        SHARING_MAX_CONNECTIONS
    );
}

/// A slowloris must not extend its admission by trickling bytes: the head
/// budget is absolute, not an idle reset per byte.
#[tokio::test]
async fn sharing_head_budget_is_absolute_while_bytes_trickle() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(250, 250, 5000)).await;
    let started = tokio::time::Instant::now();
    let error = tokio::time::timeout(Duration::from_secs(2), async {
        let mut byte = [0];
        loop {
            client.write_all(b"X").await.unwrap();
            if let Err(error) = socket.read(&mut byte).await {
                break error;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .expect("the absolute head budget must close a trickled request");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "the head budget must not grow with each trickled byte"
    );
}

/// Handler work happens before the first response byte; it must never be
/// charged to the client's connection budget.
#[tokio::test]
async fn sharing_handler_time_is_not_charged_to_the_first_response() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(1000, 1000, 200)).await;
    let head = b"GET /secret/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    client.write_all(head).await.unwrap();
    let mut received = vec![0; head.len()];
    socket.read_exact(&mut received).await.unwrap();
    // Longer than every configured budget, standing in for archive work.
    tokio::time::sleep(Duration::from_millis(1400)).await;
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .expect("a slow handler still owns its first response");
    let mut byte = [0; 16];
    let error = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut, "the response idle bound still applies");
}

#[tokio::test]
async fn sharing_declared_body_has_its_own_absolute_deadline() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(2000, 1000, 5000)).await;
    let head = b"POST /nearby/session HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 10\r\n\r\n";
    client.write_all(head).await.unwrap();
    let mut received = vec![0; head.len()];
    socket.read_exact(&mut received).await.unwrap();
    client.write_all(b"pin=").await.unwrap();
    let mut body = [0; 4];
    socket.read_exact(&mut body).await.unwrap();
    assert_eq!(&body, b"pin=");
    let mut extra = [0; 16];
    let error = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut extra))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[tokio::test]
async fn sharing_completed_body_returns_to_unbounded_handler_time() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(2000, 1000, 100)).await;
    let head = b"POST /nearby/session HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 4\r\n\r\n";
    client.write_all(head).await.unwrap();
    client.write_all(b"pin=").await.unwrap();
    let mut received = vec![0; head.len()];
    socket.read_exact(&mut received).await.unwrap();
    let mut body = [0; 4];
    socket.read_exact(&mut body).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1400)).await;
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .expect("a complete body disarms the body deadline");
}

#[tokio::test]
async fn sharing_keep_alive_request_gets_a_fresh_head_budget() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(1000, 1000, 5000)).await;
    let head = b"GET /secret/ HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    client.write_all(head).await.unwrap();
    let mut received = vec![0; head.len()];
    socket.read_exact(&mut received).await.unwrap();
    socket
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
        .await
        .unwrap();
    socket.flush().await.unwrap();
    client
        .write_all(b"GET /secret/book HTTP/1.1\r\nHost: 127.0.0.1")
        .await
        .unwrap();
    let mut body = [0; 64];
    assert!(socket.read(&mut body).await.unwrap() > 0);
    let error = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut body))
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

/// HTTP/2 multiplexes requests on one connection, so the HTTP/1 phase machine
/// must not apply; only the long idle bound does.
#[tokio::test]
async fn sharing_http2_connection_keeps_only_the_idle_bound() {
    let cancellation = SharingCancellation::new();
    let (mut client, mut socket) = connected(&cancellation, short_timeouts(1000, 1000, 5000)).await;
    let preface = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
    client.write_all(preface).await.unwrap();
    let mut frame = [0; 9];
    let mut received = vec![0; preface.len()];
    socket.read_exact(&mut received).await.unwrap();
    assert_eq!(&received[..], &preface[..]);
    assert!(socket.http2);
    // Past the request budget, but the multiplexed connection keeps its idle
    // bound and keeps carrying frames.
    tokio::time::sleep(Duration::from_millis(1400)).await;
    client.write_all(&[0; 9]).await.unwrap();
    socket.read_exact(&mut frame).await.unwrap();
    socket.write_all(&[0; 9]).await.unwrap();
}

#[tokio::test]
async fn dropping_server_closes_active_responses_and_every_listener() {
    let cancellation = SharingCancellation::new();
    let mut servers = Vec::new();
    let mut responses = Vec::new();
    let mut addresses = Vec::new();
    let root = std::env::temp_dir().join(format!("kiwix-sharing-drop-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for _ in 0..2 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        addresses.push(address);
        let listener = SharingListener {
            listener,
            cancellation: cancellation.clone(),
        };
        let router = Router::new().fallback(any(|| async {
            let chunks = stream::once(async { Ok::<_, io::Error>(vec![b'x'; 1024]) })
                .chain(stream::pending::<Result<Vec<u8>, io::Error>>());
            Body::from_stream(chunks)
        }));
        let stopped = cancellation.clone();
        servers.push(tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(stopped.cancelled())
                .await.map_err(|error| format!("Sharing listener failed: {error}"))
        }));
        let mut response = client
            .get(format!("http://{address}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.chunk().await.unwrap().unwrap().len(), 1024);
        responses.push(response);
    }
    let server = SharingServer {
        url: String::new(),
        urls: Vec::new(),
        port: 0,
        binding: SharingBinding::Loopback {},
        bindings: Vec::new(),
        books: Vec::new(),
        token: String::new(),
        book_ids: Vec::new(),
        scope: SharingScope::Selected,
        session_id: String::new(),
        locked: false,
        catalog_url: None,
        catalog_urls: Vec::new(),
        library_count: 0,
        cancellation,
        servers,
        archives: Vec::new(),
        library: None,
    };
    drop(server);
    for mut response in responses {
        let chunk = tokio::time::timeout(Duration::from_secs(2), response.chunk())
            .await
            .expect("active response must stop promptly");
        assert!(chunk.is_err() || chunk.unwrap().is_none());
    }
    for address in addresses {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(listener) = TcpListener::bind(address).await {
                    drop(listener);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("every listener must be released");
    }
    std::fs::remove_dir_all(root).unwrap();
}

fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("host", HeaderValue::from_str(host).unwrap());
    if let Some(origin) = origin {
        headers.insert("origin", HeaderValue::from_str(origin).unwrap());
    }
    headers
}

#[test]
fn ipv6_urls_and_authorities_are_bracketed_and_family_specific() {
    let v4: IpAddr = "192.0.2.5".parse().unwrap();
    let v6: IpAddr = "2001:db8::5".parse().unwrap();
    assert_eq!(
        access_url(v6, 8080, "/secret/"),
        "http://[2001:db8::5]:8080/secret/"
    );
    let accepted = authorities(&[v6], 8080);
    let uri: Uri = "/secret/".parse().unwrap();
    assert_eq!(
        accepted_host(
            &headers("[2001:db8::5]:8080", Some("http://[2001:db8::5]:8080")),
            &uri,
            &accepted
        ),
        Some(v6)
    );
    assert_eq!(
        accepted_host(&headers("192.0.2.5:8080", None), &uri, &accepted),
        None
    );
    assert_eq!(
        accepted_host(&headers("2001:db8::5:8080", None), &uri, &accepted),
        None
    );
    assert_eq!(
        accepted_host(&headers("[2001:db8::5]:8081", None), &uri, &accepted),
        None
    );
    let both = authorities(&[v4, v6], 8080);
    assert_eq!(
        accepted_host(&headers("192.0.2.5:8080", None), &uri, &both),
        Some(v4)
    );
    assert_eq!(
        accepted_host(&headers("[2001:db8::5]:8080", None), &uri, &both),
        Some(v6)
    );
}

#[test]
fn authority_rejects_rebinding_duplicates_and_cross_family_origins() {
    let accepted = authorities(
        &["127.0.0.1".parse().unwrap(), "::1".parse().unwrap()],
        8080,
    );
    let uri: Uri = "/secret/".parse().unwrap();
    for host in [
        "localhost:8080",
        "evil.example:8080",
        "127.0.0.1:80",
        "[::ffff:127.0.0.1]:8080",
        "127.0.0.1:8080.evil.example",
        "127.0.0.1",
    ] {
        assert_eq!(
            accepted_host(&headers(host, None), &uri, &accepted),
            None,
            "{host}"
        );
    }
    for origin in [
        "null",
        "https://127.0.0.1:8080",
        "http://[::1]:8080",
        "http://127.0.0.1:8080/",
        "http://127.0.0.1:8080.evil.example",
    ] {
        assert_eq!(
            accepted_host(&headers("127.0.0.1:8080", Some(origin)), &uri, &accepted),
            None,
            "{origin}"
        );
    }
    let mut duplicate = headers("127.0.0.1:8080", None);
    duplicate.append("host", HeaderValue::from_static("127.0.0.1:8080"));
    assert_eq!(accepted_host(&duplicate, &uri, &accepted), None);
    let mut duplicate = headers("127.0.0.1:8080", Some("http://127.0.0.1:8080"));
    duplicate.append("origin", HeaderValue::from_static("http://127.0.0.1:8080"));
    assert_eq!(accepted_host(&duplicate, &uri, &accepted), None);
    let foreign_uri: Uri = "http://evil.example:8080/secret/".parse().unwrap();
    assert_eq!(
        accepted_host(&headers("127.0.0.1:8080", None), &foreign_uri, &accepted),
        None
    );
    let same_uri: Uri = "http://127.0.0.1:8080/secret/".parse().unwrap();
    assert!(accepted_host(&headers("127.0.0.1:8080", None), &same_uri, &accepted).is_some());
}

#[test]
fn interface_candidates_exclude_unreachable_and_scoped_addresses() {
    for address in [
        "0.0.0.0",
        "0.1.2.3",
        "169.254.2.3",
        "255.255.255.255",
        "224.0.0.1",
        "::",
        "fe80::1",
        "febf::1",
        "ff02::1",
        "::ffff:192.0.2.1",
    ] {
        assert!(!safe_interface(address.parse().unwrap()), "{address}");
    }
    for address in [
        "127.0.0.1",
        "192.168.1.3",
        "10.0.0.2",
        "::1",
        "fd12::1",
        "2001:db8::1",
    ] {
        assert!(safe_interface(address.parse().unwrap()), "{address}");
    }
    assert!("fe80::1%en0".parse::<IpAddr>().is_err());
    assert!("localhost".parse::<IpAddr>().is_err());
}

#[test]
fn explicit_binding_modes_have_strict_wire_shapes() {
    for mode in ["loopback", "ipv4All", "ipv6All", "dualStackAll", "hotspot"] {
        let value = json!({"mode": mode});
        let binding: SharingBinding = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(binding).unwrap(), value);
    }
    assert!(serde_json::from_value::<SharingBinding>(
        json!({"mode":"loopback","address":"192.0.2.1"})
    )
    .is_err());
    assert!(serde_json::from_value::<SharingBinding>(json!({"mode":"interface"})).is_err());
    assert!(serde_json::from_value::<SharingBinding>(json!({"mode":"public"})).is_err());
    assert_eq!(
        binding_addresses(None, &SharingBinding::Loopback {}).unwrap(),
        (
            "127.0.0.1".parse().unwrap(),
            vec!["127.0.0.1".parse().unwrap()]
        )
    );
    // Hotspot binding is resolved from the reservation, so it can never
    // fall back to a guessed address when no service owns one.
    assert!(binding_addresses(None, &SharingBinding::Hotspot {}).is_err());
}

#[test]
fn library_routes_are_an_explicit_closed_table() {
    assert_eq!(library_route(""), Some(LibraryRoute::Index));
    assert_eq!(library_route("catalog/v2/root.xml"), Some(LibraryRoute::Root));
    assert_eq!(
        library_route("catalog/v2/entries"),
        Some(LibraryRoute::Entries)
    );
    assert_eq!(
        library_route("catalog/v2/categories"),
        Some(LibraryRoute::Categories)
    );
    assert_eq!(
        library_route("catalog/v2/languages"),
        Some(LibraryRoute::Languages)
    );
    assert_eq!(
        library_route("download/abc-123"),
        Some(LibraryRoute::Download("abc-123"))
    );
    assert_eq!(
        library_route("open/abc-123"),
        Some(LibraryRoute::Open("abc-123"))
    );
    for rest in [
        "catalog/v2/entries/",
        "catalog/v2/root.xml/",
        "catalog/v2",
        "download/",
        "download/a/b",
        "download/a\\b",
        "download/a?b",
        "download/a#b",
        "open/",
        "open/a/b",
        "open/a\\b",
        "open/a?b",
        "../download/a",
        "A/index.html",
    ] {
        assert!(library_route(rest).is_none(), "{rest}");
    }
}

#[test]
fn catalog_queries_are_bounded_and_reject_duplicates() {
    let query = parse_entries_query(
        &"/x/catalog/v2/entries?lang=eng&count=2&start=1&q=wiki"
            .parse()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(query.lang, "eng");
    assert_eq!(query.count, 2);
    assert_eq!(query.start, 1);
    assert_eq!(query.q, "wiki");
    let defaults = parse_entries_query(&"/x/catalog/v2/entries".parse().unwrap()).unwrap();
    assert_eq!(defaults.count, OPDS_PAGE_DEFAULT);
    assert_eq!(defaults.start, 0);
    for query in [
        "q=a&q=b",
        "start=1&start=2",
        "count=0",
        "count=201",
        "count=x",
        "unknown=1",
        "q=%0A",
    ] {
        let uri: Uri = format!("/x/catalog/v2/entries?{query}").parse().unwrap();
        assert!(parse_entries_query(&uri).is_err(), "{query}");
    }
}

pub(super) fn feed_book() -> Book {
    serde_json::from_value(json!({
        "id": "11111111-2222-3333-4444-555555555555",
        "title": "Alpha & <Beta>",
        "description": "A description",
        "language": "eng,fra",
        "creator": "Creator",
        "publisher": "Publisher",
        "date": "2025-01-01",
        "name": "alpha",
        "tags": ["_category:wikipedia", "reference"],
        "size": 1234,
        "articleCount": 10,
        "mediaCount": 2,
        "hasFulltextIndex": true,
        "hasTitleIndex": true,
        "mainPath": "index.html",
        "installationId": "install",
        "path": "/tmp/alpha.zim",
        "addedAt": "2025-01-01T00:00:00Z",
        "available": true,
        "managed": false
    }))
    .unwrap()
}

#[test]
fn library_feed_entries_are_parseable_opds_acquisitions() {
    let book = feed_book();
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\" xmlns:dc=\"http://purl.org/dc/terms/\">{}</feed>",
        book_entry_xml(
            &book,
            "http://127.0.0.1:9/token/",
            "http://127.0.0.1:9/token"
        )
    );
    let document = roxmltree::Document::parse(&xml).unwrap();
    let entry = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name(("http://www.w3.org/2005/Atom", "entry")))
        .unwrap();
    let field = |name: &str| {
        entry
            .children()
            .find(|node| node.is_element() && node.tag_name().name() == name)
            .and_then(|node| node.text())
            .unwrap_or("")
            .to_string()
    };
    assert_eq!(field("title"), "Alpha & <Beta>");
    assert_eq!(field("name"), "alpha");
    assert_eq!(field("articleCount"), "10");
    assert_eq!(field("language"), "eng,fra");
    let link = entry
        .children()
        .find(|node| {
            node.has_tag_name(("http://www.w3.org/2005/Atom", "link"))
                && node.attribute("type") == Some("application/x-zim")
        })
        .unwrap();
    assert_eq!(link.attribute("length"), Some("1234"));
    assert_eq!(
        link.attribute("href"),
        Some("http://127.0.0.1:9/token/download/11111111-2222-3333-4444-555555555555")
    );
    assert!(entry
        .children()
        .filter(|node| node.has_tag_name(("http://www.w3.org/2005/Atom", "category")))
        .any(|node| node.attribute("term") == Some("_category:wikipedia")));
    assert_eq!(
        download_name("Alpha & <Beta>", "id"),
        "Alpha____Beta_.zim"
    );
    assert_eq!(download_name("", "id"), "id.zim");
}

#[test]
fn download_names_preserve_the_suffix_at_the_length_boundary() {
    for length in [115, 116, 117, 119, 120, 121, 240, 1024] {
        let stem = "a".repeat(length);
        for name in [stem.clone(), format!("{stem}.zim"), format!("{stem}.ZIM")] {
            let filename = download_name(&name, "id");
            assert_eq!(filename, format!("{}.zim", "a".repeat(length.min(116))));
            assert!(filename.len() <= 120);
            crate::acquisition::shared_file_name(&filename).unwrap();
        }
    }
}

#[test]
fn download_names_use_the_acquisition_validator_for_reserved_metadata() {
    for name in ["CON", "con.zim", "CON.notes", "PRN", "AUX", "NUL", "COM1", "com9.zim", "LPT1", "LPT9"] {
        let filename = download_name(name, "id");
        assert!(filename.starts_with('_'), "{name}: {filename}");
        assert!(filename.len() <= 120);
        crate::acquisition::shared_file_name(&filename).unwrap();
    }
    assert_eq!(download_name("COM0", "id"), "COM0.zim");
    assert_eq!(download_name("LPT10", "id"), "LPT10.zim");
    for (name, id, expected) in [
        ("", "CON", "_CON.zim"),
        ("...", "../bad/name", ".._bad_name.zim"),
        ("--__", "\u{0}/\\:*?\"<>|", "archive.zim"),
        ("日本語", "", "archive.zim"),
        ("", ".zim", "archive.zim"),
    ] {
        let filename = download_name(name, id);
        assert_eq!(filename, expected);
        crate::acquisition::shared_file_name(&filename).unwrap();
    }
    let filename = download_name(&format!("CON.{}", "a".repeat(200)), "id");
    assert_eq!(filename.len(), 120);
    crate::acquisition::shared_file_name(&filename).unwrap();
}

pub(super) fn library_store(
    fixture: &std::path::Path,
    directory: &std::path::Path,
) -> (Arc<Store>, Book) {
    // Production imports record canonical locations. macOS's temporary
    // directory may otherwise contain a /var symlink ancestor, which the
    // retained no-follow source capability deliberately rejects.
    let directory = directory.canonicalize().unwrap();
    let archive = Archive::open(fixture).unwrap();
    let metadata = archive.metadata().unwrap();
    let path = directory.join("library.zim");
    std::fs::copy(fixture, &path).unwrap();
    let book = Book {
        metadata,
        installation_id: crate::store::new_installation_id(),
        path: path.to_string_lossy().into_owned(),
        source_reference: None,
        added_at: "2025-01-01T00:00:00Z".into(),
        available: true,
        managed: false,
        integrity: None,
    };
    let store = Arc::new(Store::new(directory.join("profile")).unwrap());
    store
        .update(|state| {
            state.books.push(book.clone());
            Ok(())
        })
        .unwrap();
    (store, book)
}

#[tokio::test]
async fn library_archive_identity_is_rechecked_for_every_request() {
    let directory = std::env::temp_dir().join(format!("kiwix-library-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let (store, book) = library_store(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
        &directory,
    );
    let state = LibraryBook {
        id: book.metadata.id.clone(),
        installation_id: book.installation_id.clone(),
        path: book.path.clone(),
        store: store.clone(),
        archives: Arc::new(LibraryArchives::default()),
    };
    let first = library_archive(&state).unwrap();
    let second = library_archive(&state).unwrap();
    assert!(Arc::ptr_eq(&first, &second), "unchanged archive stays cached");
    assert_eq!(state.archives.opens(), 1);

    // Windows refuses to delete an archive that an open reader still leases,
    // so a cached archive can only go stale there by being relinked. Prove the
    // live record is checked before the cache while the entry is still cached;
    // then release the handles, as the service does before it removes a book.
    #[cfg(windows)]
    {
        store
            .update(|state| {
                state.books[0].path = "C:\\elsewhere.zim".into();
                Ok(())
            })
            .unwrap();
        let relinked = library_archive(&state).unwrap_err();
        assert_eq!(relinked.status, StatusCode::GONE);
        assert!(relinked.message.contains("relinked"), "{}", relinked.message);
        store
            .update(|state| {
                state.books[0].path = book.path.clone();
                Ok(())
            })
            .unwrap();
        let cached = library_archive(&state).unwrap();
        assert!(Arc::ptr_eq(&first, &cached), "the cached entry survives");
        assert_eq!(state.archives.opens(), 1);
        drop((first, second, cached));
        state.archives.cached.lock().unwrap().clear();
    }
    std::fs::remove_file(&book.path).unwrap();
    let missing = library_archive(&state).unwrap_err();
    assert_eq!(missing.status, StatusCode::GONE);
    assert!(missing.message.contains("missing"), "{}", missing.message);

    let replacement = directory.join("replacement.zim");
    std::fs::copy(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
        &replacement,
    )
    .unwrap();
    store
        .update(|state| {
            state.books[0].path = replacement.to_string_lossy().into_owned();
            Ok(())
        })
        .unwrap();
    let relinked = library_archive(&state).unwrap_err();
    assert_eq!(relinked.status, StatusCode::GONE);
    assert!(relinked.message.contains("relinked"), "{}", relinked.message);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn ranged_downloads_read_the_admitted_descriptors() {
    let directory = std::env::temp_dir().join(format!("kiwix-download-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let (_store, book) = library_store(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim"),
        &directory,
    );
    let expected = std::fs::read(&book.path).unwrap();
    for offset in [0, 1, expected.len() / 2, expected.len() - 1] {
        let (mut file, size, name) = open_download(book.clone()).unwrap();
        assert_eq!(file.read(offset as u64, 1).unwrap(), expected[offset..offset + 1]);
        assert_eq!(size, book.metadata.size);
        assert!(name.ends_with(".zim"));
    }
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn archive_csp_forbids_framing_without_changing_existing_directives() {
    assert_eq!(
        ARCHIVE_CSP,
        "default-src 'self' blob: data:; script-src 'self' 'unsafe-inline' 'unsafe-eval' 'wasm-unsafe-eval' blob:; worker-src 'self' blob:; style-src 'self' 'unsafe-inline'; connect-src 'self'; frame-src 'none'; object-src 'none'; form-action 'self'; base-uri 'self'; frame-ancestors 'none'"
    );
    assert!(ARCHIVE_CSP.ends_with("; frame-ancestors 'none'"));
    assert!(ARCHIVE_CSP.contains("frame-src 'none'"));
    assert!(ARCHIVE_CSP.contains("object-src 'none'"));
}
