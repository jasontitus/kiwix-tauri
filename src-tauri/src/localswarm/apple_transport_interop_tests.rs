// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use crate::localswarm::{library, library_wire as wire, network::{self, RecvStream, SendStream}};

struct TestDirectory(std::path::PathBuf);
impl Drop for TestDirectory {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

async fn inventory_request(
    endpoint: &quinn::Endpoint, address: std::net::SocketAddr,
    peer: &library::LibraryPeerDescriptor, cancel: &CancellationToken,
) -> Result<(quinn::Connection, SendStream, RecvStream, library::LibraryInventory), String> {
    let connection = endpoint.connect(address, "localswarm.local").map_err(|error| error.to_string())?
        .await.map_err(|error| error.to_string())?;
    let (send, recv) = connection.open_bi().await.map_err(|error| error.to_string())?;
    let mut send = SendStream::Quinn(send); let mut recv = RecvStream::Quinn(recv);
    let request = wire::Request { version: wire::VERSION, session_id: peer.session_id.clone(),
        peer_id: peer.peer_id.clone(), book_id: None, range: None, if_range: None };
    wire::send(&mut send, &request, wire::CONTROL_LIMIT, cancel).await?;
    let reply: wire::Reply = wire::receive(&mut recv, wire::INVENTORY_LIMIT, cancel).await?;
    if reply.version != wire::VERSION || reply.status != 200 || reply.session_id != peer.session_id
        || reply.peer_id != peer.peer_id || reply.length != 0 || reply.total != 0 || reply.start != 0
        || reply.etag.is_some() || reply.error.is_some() {
        return Err("Native inventory reply did not match its exact request".into());
    }
    let inventory = reply.inventory.ok_or("Native inventory reply omitted the inventory")?;
    Ok((connection, send, recv, inventory))
}

async fn native_remote_close(connection: &quinn::Connection, recv: &mut RecvStream) -> Result<(), String> {
    let mut trailing = [0];
    let end = recv.read(&mut trailing).await;
    if matches!(end, Ok(Some(_))) { return Err("Native library sent trailing response payload".into()); }
    // Stream cancellation alone is not completion. Require one of the exact
    // remote no-error close frames production accepts after its application
    // ACK; EOF, other error codes and local closure cannot satisfy this.
    match connection.closed().await {
        quinn::ConnectionError::ApplicationClosed(close) if close.error_code == 0u32.into() => Ok(()),
        quinn::ConnectionError::ConnectionClosed(close) if close.error_code == quinn::TransportErrorCode::NO_ERROR => Ok(()),
        error => Err(format!("Expected native remote application-zero or transport-NO_ERROR close, received {error}")),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Exercises real Quinn clients and the native Apple library listener on an active physical LAN interface"]
async fn quinn_native_library_completion_retires_only_its_owned_transport() {
    let root = TestDirectory(std::env::temp_dir().canonicalize().unwrap()
        .join(format!("kiwix-quinn-native-library-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&root.0).unwrap();
    let path = root.0.join("library.zim");
    std::fs::copy(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/small.zim"), &path).unwrap();
    let book = crate::store::Book {
        metadata: crate::archive::Archive::open(&path).unwrap().metadata().unwrap(),
        installation_id: crate::store::new_installation_id(), path: path.to_string_lossy().into_owned(),
        source_reference: None, added_at: "2026-01-01T00:00:00Z".into(), available: true,
        managed: false, integrity: None,
    };
    let expected_book = library::LibraryBook { id: book.metadata.id.clone(), title: book.metadata.title.clone(),
        file_name: crate::sharing::download_name(&book.metadata.name, &book.metadata.id),
        size: book.metadata.size, archive_id: Some(book.metadata.id.clone()) };
    let store = Arc::new(crate::store::Store::new(root.0.join("profile")).unwrap());
    store.update(|state| { state.books.push(book); Ok(()) }).unwrap();
    let cancel = CancellationToken::new();
    let mut listener = tokio::time::timeout(Duration::from_secs(5), Listener::new_library(&cancel))
        .await.expect("Native listener setup exceeded five seconds").unwrap();
    let mut endpoint = None;
    let mut rejected_endpoint = None;
    let mut held = None;
    let mut rejected = None;
    let peer = library::LibraryPeerDescriptor { session_id: uuid::Uuid::new_v4().simple().to_string(),
        peer_id: uuid::Uuid::new_v4().to_string(), name: "Quinn native inventory test".into(),
        port: listener.port, addresses: Vec::new(), native_services: Vec::new(), direct: false,
        authority: Default::default(), route: None, cancel: cancel.clone() };
    let server_peer = peer.clone();
    let server_cancel = cancel.child_token(); let serving_cancel = server_cancel.clone();
    let (outcomes, mut completed) = mpsc::channel(16);
    let mut server = tokio::spawn(async move {
        let requests_cancel = serving_cancel.child_token();
        let mut requests = tokio::task::JoinSet::new();
        let result = loop {
            tokio::select! {
                _ = serving_cancel.cancelled() => break Ok::<(), String>(()),
                result = requests.join_next(), if !requests.is_empty() => {
                    let result = match result {
                        Some(Ok(result)) => result,
                        Some(Err(error)) => break Err(error.to_string()),
                        None => break Err("Native request supervisor lost a task".into()),
                    };
                    if outcomes.send(result).await.is_err() { break Err("Native request outcome receiver stopped".into()); }
                },
                incoming = listener.accept() => {
                    let Some((connection, ready)) = incoming else { break Err("Native library listener stopped unexpectedly".into()); };
                    let store = store.clone(); let peer = server_peer.clone();
                    let cancel = requests_cancel.child_token();
                    requests.spawn(async move {
                        let result = async {
                            network::guarded(&cancel, async {
                                ready.await.map_err(|_| "Native readiness callback lost".to_string())?
                            }).await?;
                            library::test_serve_native_request(store, peer.peer_id, peer.session_id, peer.name,
                                SendStream::Apple(connection.clone()), RecvStream::Apple(connection.clone()), cancel).await
                        }.await;
                        // Match production: the handler (including ACK and
                        // source work) drains before its owning transport closes.
                        connection.shutdown().await;
                        result
                    });
                },
            }
        };
        requests_cancel.cancel();
        listener.shutdown().await;
        while requests.join_next().await.is_some() {}
        result
    });
    let exchange = tokio::time::timeout(Duration::from_secs(30), async {
        let ip = network::reachable_addresses()?.into_iter().find(|ip| ip.is_ipv4())
            .ok_or("The native Quinn regression requires an active physical IPv4 LAN interface")?;
        let mut peer = peer; peer.addresses.push(ip.to_string());
        let address = std::net::SocketAddr::new(ip, peer.port);
        let mut client = quinn::Endpoint::client(std::net::SocketAddr::new(ip, 0)).map_err(|error| error.to_string())?;
        let client_config = network::client_config()?;
        client.set_default_client_config(client_config.clone());
        endpoint = Some(client);
        let endpoint = endpoint.as_ref().unwrap();
        let expected = library::LibraryInventory { version: wire::VERSION, session_id: peer.session_id.clone(),
            peer_id: peer.peer_id.clone(), name: peer.name.clone(), books: vec![expected_book] };
        let expected = serde_json::to_value(expected).map_err(|error| error.to_string())?;
        let (connection, send, recv, inventory) = tokio::time::timeout(Duration::from_secs(5),
            inventory_request(endpoint, address, &peer, &cancel)).await
            .map_err(|_| "Held Quinn/native inventory setup exceeded five seconds".to_string())??;
        held = Some((connection, send, recv));
        if serde_json::to_value(inventory).map_err(|error| error.to_string())? != expected {
            return Err("Held native inventory changed its installed archive or publication identity".into());
        }
        // Leave this independent transport waiting for ACK. Retire more than
        // the native listener's sixteen admission slots on other transports;
        // no local endpoint shutdown or retained response drop may fake success.
        for index in 0..18 {
            let inventory = tokio::time::timeout(Duration::from_secs(5), library::browse(&peer, cancel.child_token()))
                .await.map_err(|_| format!("Quinn native inventory {index} did not complete within five seconds"))??;
            if serde_json::to_value(inventory).map_err(|error| error.to_string())? != expected {
                return Err(format!("Quinn native inventory {index} changed its exact installed snapshot"));
            }
            match tokio::time::timeout(Duration::from_secs(5), completed.recv()).await
                .map_err(|_| format!("Native inventory handler {index} did not finish shutdown within five seconds"))? {
                Some(Ok(())) => {},
                result => return Err(format!("Native inventory handler did not consume its ACK successfully: {result:?}")),
            }
            if held.as_ref().unwrap().0.close_reason().is_some() {
                return Err("Retiring another library request closed the held client's transport".into());
            }
        }
        // Production gives each library request its own UDP socket/endpoint.
        // Do not ask the native listener to demultiplex another QUIC connection
        // on the held request's exact local/remote socket tuple.
        let mut invalid_client = quinn::Endpoint::client(std::net::SocketAddr::new(ip, 0)).map_err(|error| error.to_string())?;
        invalid_client.set_default_client_config(client_config);
        rejected_endpoint = Some(invalid_client);
        let (connection, mut send, recv, inventory) = tokio::time::timeout(Duration::from_secs(5),
            inventory_request(rejected_endpoint.as_ref().unwrap(), address, &peer, &cancel)).await
            .map_err(|_| "Invalid-ACK Quinn/native inventory setup exceeded five seconds".to_string())??;
        rejected = Some((connection, recv));
        if serde_json::to_value(inventory).map_err(|error| error.to_string())? != expected {
            return Err("Invalid-ACK inventory changed its exact installed snapshot".into());
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            send.write_all(&[2]).await?;
            if let SendStream::Quinn(send) = &mut send { let _ = send.finish(); }
            let (connection, recv) = rejected.as_mut().unwrap();
            native_remote_close(connection, recv).await
        }).await.map_err(|_| "Invalid-ACK native transport did not close within five seconds".to_string())??;
        match tokio::time::timeout(Duration::from_secs(5), completed.recv()).await
            .map_err(|_| "Invalid-ACK native handler did not finish shutdown within five seconds".to_string())? {
            Some(Err(error)) if error == "Invalid library completion acknowledgement" => {},
            result => return Err(format!("Native handler did not reject the exact invalid ACK: {result:?}")),
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            let (connection, send, recv) = held.as_mut().unwrap();
            if connection.close_reason().is_some() { return Err("Invalid ACK retired a different client's transport".into()); }
            send.write_all(&[1]).await?;
            if let SendStream::Quinn(send) = send { let _ = send.finish(); }
            native_remote_close(connection, recv).await
        }).await.map_err(|_| "Held native transport did not close after ACK within five seconds".to_string())??;
        match tokio::time::timeout(Duration::from_secs(5), completed.recv()).await
            .map_err(|_| "Held native handler did not finish shutdown within five seconds".to_string())? {
            Some(Ok(())) => Ok(()),
            result => Err(format!("Held native inventory handler did not consume its ACK: {result:?}")),
        }
    }).await;
    // No advertisements, devices, shared profiles or application state are
    // touched. Retire every owned socket and native task even on exchange error.
    cancel.cancel(); server_cancel.cancel();
    if let Some((connection, _, _)) = &held { connection.close(0u32.into(), b"test cleanup"); }
    if let Some((connection, _)) = &rejected { connection.close(0u32.into(), b"test cleanup"); }
    if let Some(endpoint) = &endpoint { endpoint.close(0u32.into(), b"test cleanup"); }
    if let Some(endpoint) = &rejected_endpoint { endpoint.close(0u32.into(), b"test cleanup"); }
    let server_cleanup = tokio::time::timeout(Duration::from_secs(5), &mut server).await;
    if server_cleanup.is_err() { server.abort(); }
    let client_cleanup = tokio::time::timeout(Duration::from_secs(5), async {
        if let Some(endpoint) = &endpoint { endpoint.wait_idle().await; }
        if let Some(endpoint) = &rejected_endpoint { endpoint.wait_idle().await; }
    }).await;
    drop(held); drop(rejected); drop(endpoint); drop(rejected_endpoint);
    // Cleanup evidence is not replaced by the primary exchange outcome.
    server_cleanup.expect("Native listener/request cleanup exceeded five seconds")
        .expect("Native request supervisor panicked").expect("Native request supervisor failed");
    client_cleanup.expect("Owned Quinn endpoint cleanup exceeded five seconds");
    exchange.expect("Quinn/native inventory exchange exceeded thirty seconds")
        .expect("Quinn/native production completion failed");
}
