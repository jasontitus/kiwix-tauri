// SPDX-License-Identifier: GPL-3.0-or-later
//! Library extension v1: bounded JSON controls, then length-framed byte payloads.
//! This is not HTTP. Status/header semantics are an internal acquisition adapter.
use serde::{Deserialize, Serialize};
use super::{library::LibraryInventory, network::{self, RecvStream, SendStream}};
use tokio_util::sync::CancellationToken;
pub(super) const VERSION: u8 = 1;
pub(super) const CONTROL_LIMIT: usize = 8192;
pub(super) const INVENTORY_LIMIT: usize = 4 * 1024 * 1024;
pub(super) const PIECE: usize = 256 * 1024;
pub(super) const MAX_BOOKS: usize = 10_000;
#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    pub version: u8, pub session_id: String, pub peer_id: String,
    pub book_id: Option<String>, pub range: Option<String>, pub if_range: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    pub version: u8, pub session_id: String, pub peer_id: String, pub status: u16,
    pub length: u64, pub total: u64, pub start: u64, pub etag: Option<String>,
    pub inventory: Option<LibraryInventory>, pub error: Option<String>,
}
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
impl Request {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != VERSION || !super::discovery::valid_library_session_id(&self.session_id)
            || !super::protocol::valid_peer_id(&self.peer_id)
            || self.book_id.as_ref().is_some_and(|id| !valid_id(id))
            || [&self.range, &self.if_range].iter().any(|v| v.as_ref().is_some_and(|s| s.len() > 256 || !s.is_ascii() || s.chars().any(char::is_control)))
            || (self.book_id.is_none() && (self.range.is_some() || self.if_range.is_some()))
        { return Err("Invalid library request".into()); }
        Ok(())
    }
}
pub(super) async fn send<T: Serialize>(stream: &mut SendStream, value: &T, limit: usize, cancel: &CancellationToken) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > limit { return Err("Library control exceeds its bound".into()); }
    network::guarded(cancel, async {
        stream.write_all(&(bytes.len() as u32).to_be_bytes()).await?;
        stream.write_all(&bytes).await
    }).await
}
pub(super) async fn receive<T: serde::de::DeserializeOwned>(stream: &mut RecvStream, limit: usize, cancel: &CancellationToken) -> Result<T, String> {
    let mut size = [0; 4];
    network::guarded(cancel, stream.read_exact(&mut size)).await?;
    let size = u32::from_be_bytes(size) as usize;
    if size == 0 || size > limit { return Err("Invalid library control length".into()); }
    let mut bytes = vec![0; size];
    network::guarded(cancel, stream.read_exact(&mut bytes)).await?;
    serde_json::from_slice(&bytes).map_err(|e| format!("Invalid library control: {e}"))
}
/// A single byte range, with checked arithmetic and no multipart allocation.
pub(super) fn range(value: &str, size: u64) -> Result<(u64, u64), String> {
    let rest = value.strip_prefix("bytes=").ok_or("Invalid library range")?;
    let (start, end) = rest.split_once('-').ok_or("Invalid library range")?;
    if size == 0 || rest.contains(',') { return Err("Unsatisfiable library range".into()); }
    if start.is_empty() {
        let suffix: u64 = end.parse().map_err(|_| "Invalid library range")?;
        if suffix == 0 { return Err("Unsatisfiable library range".into()); }
        let length = suffix.min(size); return Ok((size - length, length));
    }
    let start: u64 = start.parse().map_err(|_| "Invalid library range")?;
    let end: u64 = if end.is_empty() { size - 1 } else { end.parse().map_err(|_| "Invalid library range")? };
    if start >= size || end < start { return Err("Unsatisfiable library range".into()); }
    Ok((start, end.min(size - 1) - start + 1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_are_bounded_and_never_wrap() {
        assert_eq!(range("bytes=2-5", 10).unwrap(), (2, 4));
        assert_eq!(range("bytes=-100", 10).unwrap(), (0, 10));
        assert_eq!(range("bytes=9-18446744073709551615", 10).unwrap(), (9, 1));
        for bad in ["bytes=10-", "bytes=2-1", "bytes=-0", "bytes=0-1,3-4", "bytes=18446744073709551616-", "../book"] { assert!(range(bad, 10).is_err(), "{bad}"); }
    }
    #[test]
    fn untrusted_requests_cannot_select_paths_or_other_versions() {
        let mut request = Request { version: VERSION, session_id: "a".repeat(32), peer_id: "remote-peer".into(), book_id: Some("book-1".into()), range: None, if_range: None };
        request.validate().unwrap();
        for id in ["../archive.zim", "/etc/passwd", "a/b", "a%2fb", "", "a\n"] { request.book_id = Some(id.into()); assert!(request.validate().is_err()); }
        request.book_id = None; request.version = 2; assert!(request.validate().is_err());
        assert!(serde_json::from_str::<Request>(r#"{"version":1,"session_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","peer_id":"remote-peer","book_id":null,"range":null,"if_range":null,"path":"/tmp/file"}"#).is_err());
    }
}

#[cfg(test)]
#[tokio::test]
async fn encrypted_peer_cannot_allocate_an_unbounded_control_frame() {
    use std::sync::Arc;
    let server = quinn::Endpoint::server(network::server_config().unwrap(), "127.0.0.1:0".parse().unwrap()).unwrap();
    let address = server.local_addr().unwrap();
    let client = tokio::spawn(async move {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut endpoint = quinn::Endpoint::new(quinn::EndpointConfig::default(), None, socket, Arc::new(quinn::TokioRuntime)).unwrap();
        endpoint.set_default_client_config(network::client_config().unwrap());
        let connection = endpoint.connect(address, "localswarm.local").unwrap().await.unwrap();
        let (mut send, _recv) = connection.open_bi().await.unwrap();
        send.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
        send.finish().unwrap();
        let _ = connection.closed().await;
        endpoint.close(0u32.into(), b"test complete"); endpoint.wait_idle().await;
    });
    let connection = server.accept().await.unwrap().await.unwrap();
    let (_send, recv) = connection.accept_bi().await.unwrap();
    let mut recv = RecvStream::Quinn(recv);
    let result = receive::<Request>(&mut recv, CONTROL_LIMIT, &CancellationToken::new()).await;
    assert!(result.unwrap_err().contains("control length"));
    connection.close(0u32.into(), b"rejected control");
    server.close(0u32.into(), b"test complete");
    client.await.unwrap(); server.wait_idle().await;
}
