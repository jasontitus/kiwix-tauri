//! LocalSwarm v1 framing and hostile-input validation (see bundled NOTICE).
#![forbid(unsafe_code)]
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use unicode_normalization::UnicodeNormalization;

pub const MAX_FRAME: usize = 64 * 1024 * 1024;
pub const MAX_MANIFEST: usize = 32 * 1024 * 1024;
pub const MAX_CHUNKS: usize = 262_144;
pub const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;
pub const MAX_FILES: usize = 4096;
pub const MAX_PEER_ID: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwarmFile {
    pub path: String,
    pub size_bytes: u64,
    pub start_chunk_index: u32,
    pub end_chunk_index: u32,
}

/// Optional, advisory ZIM identity a sender may attach to a manifest
/// (PROTOCOL.md §5.1). It is excluded from the content address, so it is an
/// unauthenticated claim: it may only select or skip work, and any decision
/// that would refuse or reuse bytes must be confirmed from verified chunks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZimIdentity {
    /// Canonical lowercase hyphenated UUID from ZIM header bytes 8..24.
    pub uuid: String,
}

/// Unknown top-level keys are ignored (PROTOCOL.md §5) so senders can add
/// advisory metadata without breaking older receivers. Every key that
/// influences the content address or byte layout remains required and strict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Read from peers but never written: Kiwix does not emit the claim, and
    /// persisted job manifests stay readable by builds that reject the key.
    #[serde(default, skip_serializing)]
    pub zim: Option<ZimIdentity>,
    pub protocol_version: u32,
    pub swarm_id: String,
    pub name: String,
    pub total_bytes: u64,
    pub chunk_size_bytes: u32,
    pub chunk_hash_algo: String,
    #[serde(deserialize_with = "bounded_files")]
    pub files: Vec<SwarmFile>,
    #[serde(deserialize_with = "bounded_hashes")]
    pub chunk_hashes: Vec<String>,
}

fn bounded_sequence<'de, D, T, const CAP: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where D: serde::Deserializer<'de>, T: Deserialize<'de> {
    struct Visitor<T, const CAP: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const CAP: usize> serde::de::Visitor<'de> for Visitor<T, CAP> {
        type Value = Vec<T>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "an array with at most {CAP} elements")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Vec<T>, A::Error> {
            let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(CAP));
            while let Some(value) = sequence.next_element()? {
                if values.len() == CAP { return Err(serde::de::Error::custom("manifest array exceeds limit")); }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Visitor::<T, CAP>(std::marker::PhantomData))
}
fn bounded_files<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<SwarmFile>, D::Error> {
    bounded_sequence::<D, SwarmFile, MAX_FILES>(deserializer)
}
fn bounded_hashes<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    bounded_sequence::<D, String, MAX_CHUNKS>(deserializer)
}

pub fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn valid_peer_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_PEER_ID && !value.chars().any(char::is_control)
}

/// NFKC case-fold key for local collision and reserved-name checks. It is
/// never sent to peers or stored; the manifest keeps the original paths.
fn fold_key(text: &str) -> String {
    let text: String = text.nfkc().collect();
    crate::casefold::fold(&text).nfkc().collect()
}

pub fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 4096 || path.contains(['\\', ':']) {
        return Err("Unsafe manifest path".into());
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.len() > 255
            || part.ends_with(['.', ' ']) || part.chars().any(char::is_control)
        {
            return Err("Unsafe manifest path component".into());
        }
        let normalized = fold_key(part);
        if normalized.contains(['/', '\\', ':']) || normalized == "." || normalized == ".."
            || normalized.ends_with(['.', ' ']) || normalized.chars().any(char::is_control)
        {
            return Err("Unsafe normalized manifest path".into());
        }
        let base = normalized.split('.').next().unwrap_or("");
        let device = matches!(base, "con" | "prn" | "aux" | "nul" | "clock$" | "conin$" | "conout$")
            || (base.len() == 4 && (base.starts_with("com") || base.starts_with("lpt"))
                && matches!(base.as_bytes()[3], b'1'..=b'9'));
        if device || normalized.starts_with(".localswarm") || normalized.starts_with(".lsbits")
            || normalized.ends_with(".lsbits")
            || matches!(normalized.as_str(), "manifest.json" | "state.json" | "chunks.bitfield" | ".ds_store")
        {
            return Err("Reserved manifest path".into());
        }
    }
    Ok(())
}

impl Manifest {
    pub fn content_address(&self) -> String {
        let mut hash = Sha256::new();
        hash.update(format!("v1:{}:", self.chunk_size_bytes).as_bytes());
        for file in &self.files {
            hash.update(file.path.as_bytes());
            hash.update(format!(":{};", file.size_bytes).as_bytes());
        }
        for value in &self.chunk_hashes { hash.update(value.as_bytes()); }
        hex::encode(hash.finalize())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.protocol_version != 1 || self.chunk_hash_algo != "sha256" {
            return Err("Unsupported LocalSwarm manifest version or hash algorithm".into());
        }
        if self.chunk_size_bytes == 0 || self.chunk_size_bytes > MAX_CHUNK_SIZE
            || self.files.is_empty() || self.files.len() > MAX_FILES
            || self.chunk_hashes.len() > MAX_CHUNKS || !valid_hash(&self.swarm_id)
            || self.name.is_empty() || self.name.len() > 1024 || self.name.chars().any(char::is_control)
        {
            return Err("Manifest exceeds supported bounds".into());
        }
        if let Some(zim) = &self.zim {
            if uuid::Uuid::try_parse(&zim.uuid).map(|id| id.to_string()).as_deref() != Ok(zim.uuid.as_str()) {
                return Err("Manifest ZIM identity is not a canonical UUID".into());
            }
        }
        let mut paths = BTreeSet::new();
        let mut total = 0u64;
        let mut cursor = 0u64;
        for file in &self.files {
            validate_path(&file.path)?;
            let normalized = fold_key(&file.path);
            if !paths.insert(normalized) { return Err("Colliding manifest paths".into()); }
            let count = file.size_bytes / u64::from(self.chunk_size_bytes)
                + u64::from(file.size_bytes % u64::from(self.chunk_size_bytes) != 0);
            let end = if count == 0 { cursor } else {
                cursor.checked_add(count - 1).ok_or("Manifest chunk range overflow")?
            };
            if u64::from(file.start_chunk_index) != cursor || u64::from(file.end_chunk_index) != end {
                return Err("Inconsistent manifest chunk ranges".into());
            }
            cursor = cursor.checked_add(count).ok_or("Manifest chunk count overflow")?;
            if cursor > MAX_CHUNKS as u64 { return Err("Too many manifest chunks".into()); }
            total = total.checked_add(file.size_bytes).ok_or("Manifest size overflow")?;
        }
        for path in &paths {
            for (index, _) in path.match_indices('/') {
                if paths.contains(&path[..index]) { return Err("Manifest file/directory collision".into()); }
            }
        }
        if total != self.total_bytes || cursor != self.chunk_hashes.len() as u64
            || self.chunk_hashes.iter().any(|value| !valid_hash(value))
            || self.content_address() != self.swarm_id
        {
            return Err("Manifest content address, totals or hashes are inconsistent".into());
        }
        Ok(())
    }

    pub fn chunk_len(&self, index: u32) -> Result<usize, String> {
        if index as usize >= self.chunk_hashes.len() { return Err("Chunk index out of range".into()); }
        // `validate` guarantees contiguous, ordered chunk ranges, so the owning
        // file is the last one whose start is at or before this chunk.
        let file = self.files
            .partition_point(|file| file.start_chunk_index <= index)
            .checked_sub(1)
            .and_then(|position| self.files.get(position))
            .filter(|file| file.size_bytes != 0 && index <= file.end_chunk_index)
            .ok_or("Chunk has no owning file")?;
        let offset = u64::from(index - file.start_chunk_index) * u64::from(self.chunk_size_bytes);
        let remaining = file.size_bytes.checked_sub(offset).ok_or("Invalid chunk offset")?;
        Ok(remaining.min(u64::from(self.chunk_size_bytes)) as usize)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_MANIFEST { return Err("Manifest JSON too large".into()); }
        let manifest: Self = serde_json::from_slice(bytes).map_err(|_| "Invalid manifest JSON")?;
        manifest.validate()?;
        Ok(manifest)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Handshake { peer_id: String, swarm_id: String },
    ManifestRequest { swarm_id: String },
    ManifestResponse(Vec<u8>),
    Bitfield { swarm_id: String, bits: Vec<bool> },
    Have { swarm_id: String, index: u32 },
    ChunkRequest { swarm_id: String, index: u32 },
    ChunkResponse { swarm_id: String, index: u32, data: Vec<u8> },
    Auth { swarm_id: String, token: String },
    /// Client challenge: a fresh nonce the seeder must answer with a PIN proof
    /// before this client discloses anything derived from the PIN.
    AuthChallenge { swarm_id: String, nonce: [u8; 32] },
    /// Seeder proof over the client nonce and a fresh server nonce.
    AuthProof { swarm_id: String, nonce: [u8; 32], proof: [u8; 32] },
    /// Client proof over both nonces. It is useless on another connection.
    AuthResponse { swarm_id: String, proof: [u8; 32] },
}

pub fn body_cap(kind: u8) -> Result<usize, String> {
    Ok(match kind {
        1 => 1 + 4 + MAX_PEER_ID + 4 + 64,
        2 => 69,
        3 => 5 + MAX_MANIFEST,
        4 => 77 + MAX_CHUNKS.div_ceil(8),
        5 | 6 => 73,
        7 => 77 + MAX_CHUNK_SIZE as usize,
        8 => 137,
        9 => 101,
        10 => 133,
        11 => 101,
        _ => return Err("Unknown LocalSwarm message type".into()),
    })
}

struct Cursor<'a> { bytes: &'a [u8], offset: usize }
impl<'a> Cursor<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self.offset.checked_add(count).ok_or("Frame length overflow")?;
        let value = self.bytes.get(self.offset..end).ok_or("Truncated frame")?;
        self.offset = end;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| "Truncated integer")?))
    }
    fn blob(&mut self, cap: usize) -> Result<&'a [u8], String> {
        let count = self.u32()? as usize;
        if count > cap { return Err("Frame field exceeds limit".into()); }
        self.take(count)
    }
    fn string(&mut self, cap: usize) -> Result<String, String> {
        String::from_utf8(self.blob(cap)?.to_vec()).map_err(|_| "Invalid UTF-8 in frame".into())
    }
    fn nonce(&mut self) -> Result<[u8; 32], String> {
        self.take(32)?.try_into().map_err(|_| "Truncated authentication nonce".into())
    }
}

impl Message {
    pub fn swarm_id(&self) -> Option<&str> {
        match self {
            Self::Handshake { swarm_id, .. } | Self::ManifestRequest { swarm_id }
            | Self::Bitfield { swarm_id, .. } | Self::Have { swarm_id, .. }
            | Self::ChunkRequest { swarm_id, .. } | Self::ChunkResponse { swarm_id, .. }
            | Self::Auth { swarm_id, .. } | Self::AuthChallenge { swarm_id, .. }
            | Self::AuthProof { swarm_id, .. } | Self::AuthResponse { swarm_id, .. } => Some(swarm_id),
            Self::ManifestResponse(_) => None,
        }
    }

    pub fn decode(body: &[u8]) -> Result<Self, String> {
        let kind = *body.first().ok_or("Empty frame")?;
        if body.len() > MAX_FRAME || body.len() > body_cap(kind)? { return Err("Frame exceeds type limit".into()); }
        let mut r = Cursor { bytes: body, offset: 1 };
        let message = match kind {
            1 => Self::Handshake { peer_id: r.string(MAX_PEER_ID)?, swarm_id: r.string(64)? },
            2 => Self::ManifestRequest { swarm_id: r.string(64)? },
            3 => Self::ManifestResponse(r.blob(MAX_MANIFEST)?.to_vec()),
            4 => {
                let swarm_id = r.string(64)?;
                let count = r.u32()? as usize;
                if count > MAX_CHUNKS { return Err("Bitfield too large".into()); }
                let packed = r.blob(MAX_CHUNKS.div_ceil(8))?;
                if packed.len() != count.div_ceil(8) { return Err("Inconsistent bitfield length".into()); }
                if !count.is_multiple_of(8) && packed.last().is_some_and(|byte| byte & ((1 << (8 - count % 8)) - 1) != 0) {
                    return Err("Nonzero bitfield padding".into());
                }
                let bits = (0..count).map(|i| packed[i / 8] & (1 << (7 - i % 8)) != 0).collect();
                Self::Bitfield { swarm_id, bits }
            }
            5 => Self::Have { swarm_id: r.string(64)?, index: r.u32()? },
            6 => Self::ChunkRequest { swarm_id: r.string(64)?, index: r.u32()? },
            7 => Self::ChunkResponse { swarm_id: r.string(64)?, index: r.u32()?, data: r.blob(MAX_CHUNK_SIZE as usize)?.to_vec() },
            8 => Self::Auth { swarm_id: r.string(64)?, token: r.string(64)? },
            9 => Self::AuthChallenge { swarm_id: r.string(64)?, nonce: r.nonce()? },
            10 => Self::AuthProof { swarm_id: r.string(64)?, nonce: r.nonce()?, proof: r.nonce()? },
            11 => Self::AuthResponse { swarm_id: r.string(64)?, proof: r.nonce()? },
            _ => return Err("Unknown frame type".into()),
        };
        if r.offset != body.len() { return Err("Trailing frame data".into()); }
        Ok(message)
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        fn number(out: &mut Vec<u8>, value: usize) -> Result<(), String> {
            out.extend(u32::try_from(value).map_err(|_| "Frame length overflow")?.to_be_bytes()); Ok(())
        }
        fn blob(out: &mut Vec<u8>, value: &[u8]) -> Result<(), String> {
            number(out, value.len())?; out.extend_from_slice(value); Ok(())
        }
        let mut out = vec![0; 4];
        match self {
            Self::Handshake { peer_id, swarm_id } => { out.push(1); blob(&mut out, peer_id.as_bytes())?; blob(&mut out, swarm_id.as_bytes())?; }
            Self::ManifestRequest { swarm_id } => { out.push(2); blob(&mut out, swarm_id.as_bytes())?; }
            Self::ManifestResponse(json) => { out.push(3); blob(&mut out, json)?; }
            Self::Bitfield { swarm_id, bits } => {
                out.push(4); blob(&mut out, swarm_id.as_bytes())?; number(&mut out, bits.len())?;
                let mut packed = vec![0; bits.len().div_ceil(8)];
                for (i, bit) in bits.iter().enumerate() { if *bit { packed[i / 8] |= 1 << (7 - i % 8); } }
                blob(&mut out, &packed)?;
            }
            Self::Have { swarm_id, index } | Self::ChunkRequest { swarm_id, index } => {
                out.push(if matches!(self, Self::Have { .. }) { 5 } else { 6 });
                blob(&mut out, swarm_id.as_bytes())?; out.extend(index.to_be_bytes());
            }
            Self::ChunkResponse { swarm_id, index, data } => {
                out.push(7); blob(&mut out, swarm_id.as_bytes())?; out.extend(index.to_be_bytes()); blob(&mut out, data)?;
            }
            Self::Auth { swarm_id, token } => { out.push(8); blob(&mut out, swarm_id.as_bytes())?; blob(&mut out, token.as_bytes())?; }
            Self::AuthChallenge { swarm_id, nonce } => { out.push(9); blob(&mut out, swarm_id.as_bytes())?; out.extend_from_slice(nonce); }
            Self::AuthProof { swarm_id, nonce, proof } => {
                out.push(10); blob(&mut out, swarm_id.as_bytes())?; out.extend_from_slice(nonce); out.extend_from_slice(proof);
            }
            Self::AuthResponse { swarm_id, proof } => { out.push(11); blob(&mut out, swarm_id.as_bytes())?; out.extend_from_slice(proof); }
        }
        if out.len() - 4 > body_cap(out[4])? { return Err("Outbound frame exceeds limit".into()); }
        let length = u32::try_from(out.len() - 4).map_err(|_| "Frame too large")?;
        out[..4].copy_from_slice(&length.to_be_bytes());
        Ok(out)
    }
}

/// Legacy deterministic share credential: `SHA-256(swarm_id || ':' || pin)`.
///
/// It is retained only so this build keeps authenticating clients that still
/// send `Message::Auth`, and because the wire format is shared with the
/// reference Swift/Go engines. A client MUST NOT send it any more: it is
/// offline-verifiable by whichever peer receives it. New clients use
/// [`pin_proof`] over per-connection nonces instead.
pub fn auth_token(swarm_id: &str, pin: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(swarm_id.as_bytes()); hash.update(b":"); hash.update(pin.as_bytes());
    hex::encode(hash.finalize())
}

/// HMAC-SHA-256 (RFC 2104). The crate set has no `hmac` dependency and this is
/// small enough to keep the protocol self-contained.
fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > block.len() { block[..32].copy_from_slice(&Sha256::digest(key)); }
    else { block[..key.len()].copy_from_slice(key); }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    for part in parts { inner.update(part); }
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

/// Fresh 256-bit nonce for one PIN authentication exchange.
pub fn auth_nonce() -> Result<[u8; 32], String> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce).map_err(|_| "Cannot create a LocalSwarm authentication nonce".to_string())?;
    Ok(nonce)
}

/// Role-separated proof that one side knows the share PIN, bound to both
/// per-connection nonces and the swarm identity. Neither side can replay it on
/// another connection, and the seeder proves possession before the client sends
/// anything derived from the PIN.
pub fn pin_proof(pin: &str, role: &str, swarm_id: &str, client_nonce: &[u8; 32], server_nonce: &[u8; 32]) -> [u8; 32] {
    let parts: [&[u8]; 6] = [b"localswarm-pin-v1:", role.as_bytes(), b":", swarm_id.as_bytes(), client_nonce, server_nonce];
    hmac_sha256(pin.as_bytes(), &parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normative_frames_and_auth() {
        let sid = "cd".to_string();
        let vectors = [
            (Message::Handshake { peer_id: "ab".into(), swarm_id: sid.clone() }, "0000000d01000000026162000000026364"),
            (Message::ManifestRequest { swarm_id: sid.clone() }, "0000000702000000026364"),
            (Message::Bitfield { swarm_id: sid.clone(), bits: vec![true,false,true,true] }, "00000010040000000263640000000400000001b0"),
            (Message::Have { swarm_id: sid.clone(), index: 5 }, "0000000b0500000002636400000005"),
            (Message::ChunkRequest { swarm_id: sid.clone(), index: 5 }, "0000000b0600000002636400000005"),
            (Message::ChunkResponse { swarm_id: sid.clone(), index: 5, data: vec![0,1,2] }, "00000012070000000263640000000500000003000102"),
            (Message::Auth { swarm_id: sid, token: "ef".into() }, "0000000d08000000026364000000026566"),
        ];
        for (message, expected) in vectors {
            let encoded = message.encode().unwrap();
            assert_eq!(hex::encode(&encoded), expected);
            assert_eq!(Message::decode(&encoded[4..]).unwrap(), message);
            let mut trailing = encoded[4..].to_vec(); trailing.push(0);
            assert!(Message::decode(&trailing).is_err());
        }
    }
    #[test]
    fn pin_exchange_frames_round_trip_and_proofs_are_connection_and_role_bound() {
        let sid = "cd".to_string();
        let client = [7u8; 32];
        let server = [9u8; 32];
        for message in [
            Message::AuthChallenge { swarm_id: sid.clone(), nonce: client },
            Message::AuthProof { swarm_id: sid.clone(), nonce: server, proof: [1u8; 32] },
            Message::AuthResponse { swarm_id: sid.clone(), proof: [2u8; 32] },
        ] {
            let encoded = message.encode().unwrap();
            assert_eq!(Message::decode(&encoded[4..]).unwrap(), message);
        }
        // RFC 4231 test case 2.
        assert_eq!(hex::encode(hmac_sha256(b"Jefe", &[&b"what do ya want for nothing?"[..]])),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        let seeder = pin_proof("493781", "server", &sid, &client, &server);
        let client_proof = pin_proof("493781", "client", &sid, &client, &server);
        assert_ne!(seeder, client_proof);
        assert_ne!(seeder, pin_proof("493782", "server", &sid, &client, &server));
        assert_ne!(seeder, pin_proof("493781", "server", &sid, &client, &[8u8; 32]));
        assert_ne!(seeder, pin_proof("493781", "server", "ce", &client, &server));
        // Nothing sent is the deterministic legacy credential, so a captured
        // frame cannot be replayed or matched against a PIN guess.
        assert_ne!(hex::encode(seeder), auth_token(&sid, "493781"));
        assert_eq!(auth_nonce().unwrap().len(), 32);
    }
    #[test]
    fn rejects_hostile_paths_and_bitfields() {
        for path in ["../a", "/a", "a//b", "a/./b", "a\\b", "a:b", "a.", "NUL.zim", "com1", ".localswarm-state", "a\0b", "a/ＣＯＮ.txt"] {
            assert!(validate_path(path).is_err(), "{path}");
        }
        let mut frame = Message::Bitfield { swarm_id: "cd".into(), bits: vec![true] }.encode().unwrap();
        *frame.last_mut().unwrap() |= 1;
        assert!(Message::decode(&frame[4..]).is_err());
    }
    #[test]
    fn independent_file_tails_and_manifest_identity() {
        let mut manifest = Manifest { zim: None, protocol_version: 1, swarm_id: String::new(), name: "test".into(), total_bytes: 6,
            chunk_size_bytes: 4, chunk_hash_algo: "sha256".into(), files: vec![
                SwarmFile { path: "a.zim".into(), size_bytes: 3, start_chunk_index: 0, end_chunk_index: 0 },
                SwarmFile { path: "b.zim".into(), size_bytes: 3, start_chunk_index: 1, end_chunk_index: 1 }],
            chunk_hashes: vec!["a".repeat(64), "b".repeat(64)] };
        manifest.swarm_id = manifest.content_address();
        manifest.validate().unwrap();
        assert_eq!(manifest.chunk_len(0).unwrap(), 3);
        assert_eq!(manifest.chunk_len(1).unwrap(), 3);
        assert!(manifest.chunk_len(2).is_err());
        manifest.files[1].path = "A.ZIM/child".into();
        manifest.swarm_id = manifest.content_address();
        assert!(manifest.validate().is_err());
    }
    #[test]
    fn unicode_case_pairs_collide() {
        // Georgian Mtavruli U+1C90 (Unicode 11) folds to Mkhedruli U+10D0.
        let mut manifest = Manifest { zim: None, protocol_version: 1, swarm_id: String::new(), name: "test".into(), total_bytes: 6,
            chunk_size_bytes: 4, chunk_hash_algo: "sha256".into(), files: vec![
                SwarmFile { path: "\u{1C90}.zim".into(), size_bytes: 3, start_chunk_index: 0, end_chunk_index: 0 },
                SwarmFile { path: "\u{10D0}.zim".into(), size_bytes: 3, start_chunk_index: 1, end_chunk_index: 1 }],
            chunk_hashes: vec!["a".repeat(64), "b".repeat(64)] };
        manifest.swarm_id = manifest.content_address();
        assert_eq!(manifest.validate().unwrap_err(), "Colliding manifest paths");
        manifest.files[1].path = "\u{10D1}.zim".into();
        manifest.swarm_id = manifest.content_address();
        manifest.validate().unwrap();
    }
    #[test]
    fn rejects_manifest_array_bombs_before_semantic_validation() {
        let file = serde_json::json!({"path":"x.zim","size_bytes":0,"start_chunk_index":0,"end_chunk_index":0});
        let value = serde_json::json!({
            "protocol_version":1, "swarm_id":"a".repeat(64), "name":"x",
            "total_bytes":0, "chunk_size_bytes":1048576, "chunk_hash_algo":"sha256",
            "files":vec![file; MAX_FILES + 1], "chunk_hashes":[]
        });
        assert!(serde_json::from_value::<Manifest>(value).is_err());
    }
    #[test]
    fn advisory_zim_identity_is_optional_validated_and_outside_the_address() {
        let base = serde_json::json!({
            "protocol_version":1, "swarm_id":"a".repeat(64), "name":"x",
            "total_bytes":6, "chunk_size_bytes":4, "chunk_hash_algo":"sha256",
            "files":[{"path":"x.zim","size_bytes":6,"start_chunk_index":0,"end_chunk_index":1}],
            "chunk_hashes":["b".repeat(64), "c".repeat(64)]
        });
        let plain: Manifest = serde_json::from_value(base.clone()).unwrap();
        assert!(plain.zim.is_none());
        assert!(!serde_json::to_string(&plain).unwrap().contains("\"zim\":"));
        let mut claimed = base.clone();
        claimed["zim"] = serde_json::json!({"uuid":"0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0", "future":1});
        claimed["future_advisory_key"] = serde_json::json!({"ignored": true});
        let claimed: Manifest = serde_json::from_value(claimed).unwrap();
        assert_eq!(claimed.zim.as_ref().unwrap().uuid, "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0");
        assert_eq!(claimed.content_address(), plain.content_address());
        assert!(!serde_json::to_string(&claimed).unwrap().contains("\"zim\":"), "the claim must not be persisted or re-sent");
        let mut fixed = claimed.clone();
        fixed.swarm_id = fixed.content_address();
        fixed.validate().unwrap();
        for bad in ["0F1E2D3C-4B5A-6978-8796-A5B4C3D2E1F0", "0f1e2d3c4b5a69788796a5b4c3d2e1f0", "x"] {
            fixed.zim = Some(ZimIdentity { uuid: bad.into() });
            assert!(fixed.validate().is_err(), "{bad}");
        }
        // Content-bearing keys stay strict.
        let mut missing = base;
        missing.as_object_mut().unwrap().remove("chunk_hash_algo");
        assert!(serde_json::from_value::<Manifest>(missing).is_err());
    }
    #[test]
    fn refuses_unknown_types_invalid_utf8_and_packed_count_mismatch() {
        assert!(Message::decode(&[9]).is_err());
        assert!(Message::decode(&[2, 0, 0, 0, 1, 255]).is_err());
        let mut bytes = hex::decode("04000000026364000000090000000180").unwrap();
        assert!(Message::decode(&bytes).is_err());
        bytes.clear();
        assert!(Message::decode(&bytes).is_err());
    }
}
