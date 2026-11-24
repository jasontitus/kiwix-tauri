// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
use super::*;
use std::{path::PathBuf, sync::Arc};

struct Profile(PathBuf);
impl Profile {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("kiwix-private-channel-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        // Darwin's standard temp root may traverse /var -> /private/var.
        // Give the no-follow identity API the real owned fixture directory;
        // production admission remains unchanged and still rejects symlinks.
        let path = std::fs::canonicalize(path).unwrap();
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
    fn identity(&self) -> Arc<crate::private_sync_identity::Identity> {
        Arc::new(crate::private_sync_identity::load_or_create(&self.0).unwrap())
    }
}
impl Drop for Profile { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
async fn connected(a: Arc<crate::private_sync_identity::Identity>, b: Arc<crate::private_sync_identity::Identity>) -> (Channel, Channel) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let pin = b.public_hex();
    let host = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        Channel::handshake(stream, &b, false, None).await.unwrap()
    });
    let stream = TcpStream::connect(address).await.unwrap();
    let client = Channel::handshake(stream, &a, true, Some(&pin)).await.unwrap();
    (client, host.await.unwrap())
}
#[tokio::test]
async fn noise_mutually_authenticates_pinned_keys_and_full_transcript() {
    let a = Profile::new(); let b = Profile::new();
    let ai = a.identity(); let bi = b.identity();
    let (mut client, mut host) = connected(ai.clone(), bi.clone()).await;
    assert_eq!(client.identity, bi.public_hex());
    assert_eq!(host.identity, ai.public_hex());
    assert_eq!(client.verification, host.verification);
    client.send(&Message::Ready).await.unwrap();
    assert!(matches!(host.receive().await.unwrap(), Message::Ready));
    host.send(&Message::Committed).await.unwrap();
    assert!(matches!(client.receive().await.unwrap(), Message::Committed));
    let (second, _) = connected(ai, bi).await;
    assert_ne!(client.verification, second.verification, "fresh ephemeral transcript must change comparison code");
}
#[tokio::test]
async fn invitation_identity_substitution_fails_before_application_message() {
    let a = Profile::new(); let b = Profile::new(); let unrelated = Profile::new();
    let ai = a.identity(); let bi = b.identity();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        Channel::handshake(stream, &bi, false, None).await.is_err()
    });
    let stream = TcpStream::connect(address).await.unwrap();
    assert!(Channel::handshake(stream, &ai, true, Some(&unrelated.identity().public_hex())).await.is_err());
    assert!(server.await.unwrap());
}
#[tokio::test]
async fn replayed_encrypted_frame_is_rejected_not_reapplied() {
    let a = Profile::new(); let b = Profile::new();
    let (mut client, mut host) = connected(a.identity(), b.identity()).await;
    let mut ciphertext = vec![0u8; 20];
    let size = client.noise.write_message(&4u32.to_be_bytes(), &mut ciphertext).unwrap();
    write_packet(&mut client.stream, &ciphertext[..size]).await.unwrap();
    assert_eq!(host.receive_chunk().await.unwrap(), &4u32.to_be_bytes());
    write_packet(&mut client.stream, &ciphertext[..size]).await.unwrap();
    assert!(host.receive_chunk().await.is_err());
}
#[tokio::test]
async fn oversized_raw_or_authenticated_lengths_fail_before_payload_allocation() {
    let a = Profile::new(); let b = Profile::new();
    let (mut client, mut host) = connected(a.identity(), b.identity()).await;
    client.send_chunk(&((MAX_MESSAGE + 1) as u32).to_be_bytes()).await.unwrap();
    assert!(host.receive().await.is_err());
    client.stream.write_u32(65_536).await.unwrap();
    assert!(host.receive_chunk().await.is_err());
}

#[tokio::test]
async fn control_messages_cannot_claim_the_snapshot_budget() {
    let a = Profile::new();
    let b = Profile::new();
    let (mut client, mut host) = connected(a.identity(), b.identity()).await;
    client.send_chunk(&((MAX_CONTROL_MESSAGE + 1) as u32).to_be_bytes()).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), host.receive_control())
        .await.expect("oversized control headers are rejected without waiting for the body");
    assert!(matches!(result, Err(error) if error.contains("transfer bound")));
}
#[tokio::test]
async fn public_http_client_cannot_enter_noise_endpoint() {
    let profile = Profile::new(); let identity = profile.identity();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        Channel::handshake(stream, &identity, false, None).await.is_err()
    });
    let mut public_client = TcpStream::connect(address).await.unwrap();
    public_client.write_all(b"GET /bookmarks HTTP/1.1\r\nHost: local\r\n\r\n").await.unwrap();
    assert!(server.await.unwrap());
    let mut response = Vec::new();
    match public_client.read_to_end(&mut response).await {
        Ok(0) => {},
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {},
        result => panic!("Expected rejection by EOF or connection reset without application bytes, got {result:?}"),
    }
    assert!(response.is_empty());
}

pub(crate) async fn send_fragmented_confirmation(channel: &mut Channel, identity: String, header_sent: tokio::sync::oneshot::Sender<()>, release: tokio::sync::oneshot::Receiver<()>) {
    let bytes = serde_json::to_vec(&Message::Confirm { identity }).unwrap();
    channel.send_chunk(&(bytes.len() as u32).to_be_bytes()).await.unwrap();
    let _ = header_sent.send(());
    release.await.unwrap();
    for chunk in bytes.chunks(CHUNK) { channel.send_chunk(chunk).await.unwrap(); }
}
