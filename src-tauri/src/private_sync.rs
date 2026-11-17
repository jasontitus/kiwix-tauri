// SPDX-License-Identifier: GPL-3.0-or-later
//! Private reading-data protocol. This has no public Nearby route or archive handler.
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub(crate) const PROTOCOL: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const PROLOGUE: &[u8] = b"org.kiwix.private-reading-sync/1";
pub(crate) const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const CHUNK: usize = 60_000;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Settings {
    #[serde(default)]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub listen_port: u16,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Peer {
    pub id: String,
    pub identity: String,
    pub name: String,
    pub endpoints: Vec<SocketAddr>,
    pub enabled: bool,
    pub automatic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<String>,
    #[serde(default)]
    pub acknowledged_counter: u64,
}
impl Settings {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.peers.len() > 32 { return Err("Too many connected private devices".into()); }
        let mut seen = std::collections::HashSet::new();
        for peer in &self.peers {
            validate_identity(&peer.identity)?;
            if peer.id != peer.identity || !seen.insert(&peer.identity)
                || peer.name.is_empty() || peer.name.len() > 128 || peer.name.contains('\0')
                || peer.endpoints.is_empty() || peer.endpoints.len() > 16
                || peer.endpoints.iter().any(|address| address.port() == 0 || address.ip().is_unspecified() || address.ip().is_multicast()) {
                return Err("Invalid connected private device".into());
            }
        }
        Ok(())
    }
}
pub(crate) fn validate_identity(value: &str) -> Result<(), String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err("Invalid private device identity".into());
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Invitation {
    pub version: u32,
    pub identity: String,
    pub token: zeroize::Zeroizing<String>,
    pub endpoints: Vec<SocketAddr>,
    pub expires_at: String,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Message {
    Hello { purpose: String, token: Option<zeroize::Zeroizing<String>>, name: String, endpoints: Vec<SocketAddr>, categories: crate::reading_data::Categories },
    Confirm { identity: String },
    Ready,
    Snapshot { snapshot: crate::reading_data::WireSnapshot },
    Committed,
}
pub(crate) struct Channel {
    stream: TcpStream,
    noise: snow::TransportState,
    ciphertext: Vec<u8>,
    plaintext: zeroize::Zeroizing<Vec<u8>>,
    pub identity: String,
    pub verification: String,
}
impl Channel {
    pub(crate) async fn handshake(mut stream: TcpStream, identity: &crate::private_sync_identity::Identity, initiator: bool, pinned: Option<&str>) -> Result<Self, String> {
        stream.set_nodelay(true).map_err(|error| error.to_string())?;
        let params = PROTOCOL.parse().map_err(|_| "Invalid private Noise protocol")?;
        let builder = snow::Builder::new(params).prologue(PROLOGUE).map_err(|error| error.to_string())?
            .local_private_key(identity.secret_key()).map_err(|error| error.to_string())?;
        let mut handshake = if initiator { builder.build_initiator() } else { builder.build_responder() }
            .map_err(|error| error.to_string())?;
        let mut output = vec![0; 65_535];
        if initiator {
            let count = handshake.write_message(&[], &mut output).map_err(|_| "Private handshake failed")?;
            write_packet(&mut stream, &output[..count]).await?;
            let packet = read_packet(&mut stream).await?;
            handshake.read_message(&packet, &mut output).map_err(|_| "Private handshake authentication failed")?;
            if pinned.is_some_and(|key| handshake.get_remote_static().map(hex::encode).as_deref() != Some(key)) {
                return Err("The nearby device identity does not match the scanned invitation".into());
            }
            let count = handshake.write_message(&[], &mut output).map_err(|_| "Private handshake failed")?;
            write_packet(&mut stream, &output[..count]).await?;
        } else {
            let packet = read_packet(&mut stream).await?;
            handshake.read_message(&packet, &mut output).map_err(|_| "Private handshake failed")?;
            let count = handshake.write_message(&[], &mut output).map_err(|_| "Private handshake failed")?;
            write_packet(&mut stream, &output[..count]).await?;
            let packet = read_packet(&mut stream).await?;
            handshake.read_message(&packet, &mut output).map_err(|_| "Private handshake authentication failed")?;
        }
        let remote = handshake.get_remote_static().ok_or("Private device did not authenticate an identity")?;
        let remote = hex::encode(remote);
        if remote == identity.public_hex() || pinned.is_some_and(|key| key != remote) {
            return Err("Unexpected private device identity".into());
        }
        // Noise's final handshake hash commits the prologue, both ephemeral keys,
        // both static identities and every encrypted handshake message.
        let digest = hex::encode(handshake.get_handshake_hash());
        let verification = digest.as_bytes()[..24].chunks(4).map(|part| std::str::from_utf8(part).unwrap_or("")).collect::<Vec<_>>().join(" ");
        Ok(Self { stream, noise: handshake.into_transport_mode().map_err(|_| "Private handshake incomplete")?, ciphertext: Vec::with_capacity(65_535), plaintext: zeroize::Zeroizing::new(Vec::with_capacity(65_535)), identity: remote, verification })
    }
    async fn send_chunk(&mut self, plaintext: &[u8]) -> Result<(), String> {
        self.ciphertext.resize(plaintext.len() + 16, 0);
        let count = self.noise.write_message(plaintext, &mut self.ciphertext).map_err(|_| "Private encryption failed")?;
        write_packet(&mut self.stream, &self.ciphertext[..count]).await
    }
    async fn receive_chunk(&mut self) -> Result<&[u8], String> {
        let length = self.stream.read_u32().await.map_err(|error| error.to_string())? as usize;
        if length == 0 || length > 65_535 { return Err("Invalid private encrypted frame length".into()); }
        self.ciphertext.resize(length, 0);
        self.stream.read_exact(&mut self.ciphertext).await.map_err(|error| error.to_string())?;
        self.plaintext.resize(length, 0);
        let count = self.noise.read_message(&self.ciphertext, &mut self.plaintext).map_err(|_| "Private message authentication failed")?;
        self.plaintext.truncate(count);
        Ok(self.plaintext.as_slice())
    }
    pub(crate) async fn send(&mut self, message: &Message) -> Result<(), String> {
        let data = zeroize::Zeroizing::new(serde_json::to_vec(message).map_err(|error| error.to_string())?);
        if data.len() > MAX_MESSAGE { return Err("Private reading data exceeds the supported transfer bound. Remove unused connected devices or reduce the reading list before syncing".into()); }
        self.send_chunk(&(data.len() as u32).to_be_bytes()).await?;
        for chunk in data.chunks(CHUNK) { self.send_chunk(chunk).await?; }
        Ok(())
    }
    pub(crate) async fn receive(&mut self) -> Result<Message, String> {
        let header = self.receive_chunk().await?;
        let bytes: [u8; 4] = header.try_into().map_err(|_| "Invalid private message header")?;
        let length = u32::from_be_bytes(bytes) as usize;
        if length == 0 || length > MAX_MESSAGE { return Err("Private message exceeds the supported transfer bound".into()); }
        let mut data = zeroize::Zeroizing::new(Vec::with_capacity(length));
        while data.len() < length {
            let chunk = self.receive_chunk().await?;
            if chunk.is_empty() || chunk.len() > CHUNK || chunk.len() > length - data.len() { return Err("Invalid private message framing".into()); }
            data.extend_from_slice(chunk);
        }
        serde_json::from_slice(&data).map_err(|_| "Invalid private reading-data message".into())
    }
}
async fn write_packet(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > 65_535 { return Err("Invalid private encrypted frame".into()); }
    stream.write_u32(bytes.len() as u32).await.map_err(|error| error.to_string())?;
    stream.write_all(bytes).await.map_err(|error| error.to_string())
}
async fn read_packet(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let length = stream.read_u32().await.map_err(|error| error.to_string())? as usize;
    if length == 0 || length > 65_535 { return Err("Invalid private encrypted frame length".into()); }
    let mut packet = vec![0; length];
    stream.read_exact(&mut packet).await.map_err(|error| error.to_string())?;
    Ok(packet)
}

#[cfg(test)]
#[path = "private_sync_tests.rs"]
pub(crate) mod tests;
