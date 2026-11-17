// SPDX-License-Identifier: GPL-3.0-or-later
//! Public connection metadata, not invitation authentication or network authority.
//! A selected Connect permits a join; reading/subscribing never does.
mod framing;
pub(crate) use framing::{Framing, Kind};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;
const MAX_MESSAGE: usize = 4096;
type Result<T, E = LinkError> = std::result::Result<T, E>;
#[derive(Debug)]
pub(crate) enum LinkError { Framing }
impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("Invalid public Nearby frame") }
}
impl std::error::Error for LinkError {}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PublicLibrary {
    pub version: u8,
    pub public_id: String,
    pub session_id: String,
    pub peer_id: String,
    pub name: String,
    pub port: u16,
    pub addresses: Vec<String>,
    pub join: Option<TemporaryNetwork>,
    pub can_join: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TemporaryNetwork {
    pub reservation_id: String,
    pub ssid: String,
    // Explicit serde implementation avoids requiring zeroize's serde feature.
    #[serde(with = "secret")]
    pub passphrase: Zeroizing<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct HostRequest {
    pub version: u8,
    pub public_id: String,
    pub address: String,
    pub join: TemporaryNetwork,
}
impl HostRequest {
    pub(crate) fn validate(&self, public: &PublicLibrary) -> std::result::Result<(),String> {
        let candidate = PublicLibrary { version:self.version,public_id:self.public_id.clone(),
            session_id:public.session_id.clone(),peer_id:public.peer_id.clone(),name:public.name.clone(),port:public.port,
            addresses:vec![self.address.clone()],join:Some(self.join.clone()),can_join:false };
        candidate.validate(&public.public_id)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReverseReply { pub library: Option<PublicLibrary>, pub error: Option<String> }
mod secret {
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;
    pub fn serialize<S: Serializer>(value: &Zeroizing<String>, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(value)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Zeroizing<String>, D::Error> {
        String::deserialize(deserializer).map(Zeroizing::new)
    }
}
// Numeric hints do not grant route authority. The selected QUIC connection
// must still prove an on-link route; scoped link-local IPv6 needs native Bonjour.
pub(crate) fn metadata_address(raw: &str) -> bool {
    match raw.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => (ip.is_private() || ip.is_link_local())
            && !ip.is_unspecified() && !ip.is_broadcast(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_unique_local(),
        Err(_) => false,
    }
}
impl PublicLibrary {
    pub(crate) fn validate(&self, public_id: &str) -> std::result::Result<(), String> {
        let uuid = |raw: &str| uuid::Uuid::parse_str(raw).is_ok_and(|id| id.to_string() == raw);
        if self.version != 1 || self.public_id != public_id || !uuid(&self.public_id)
            || self.session_id.is_empty() || self.session_id.len() > 128
            || self.peer_id.is_empty() || self.peer_id.len() > 128
            || self.name.is_empty() || self.name.len() > 256 || self.name.chars().any(char::is_control)
            || self.port == 0 || self.addresses.len() > 16
            || self.addresses.iter().any(|raw| !metadata_address(raw)) {
            return Err("The sender supplied invalid public Nearby connection information".into());
        }
        if let Some(join) = &self.join {
            if !uuid(&join.reservation_id) || join.ssid.is_empty() || join.ssid.len() > 32
                || join.ssid.chars().any(char::is_control) || !(8..=63).contains(&join.passphrase.len())
                || !join.passphrase.bytes().all(|byte| (32..=126).contains(&byte)) {
                return Err("The sender supplied an unsupported temporary Wi-Fi network".into());
            }
        }
        Ok(())
    }
    pub(crate) fn encode(&self) -> std::result::Result<Zeroizing<Vec<u8>>, String> {
        self.validate(&self.public_id)?;
        let bytes = Zeroizing::new(serde_json::to_vec(self).map_err(|error| error.to_string())?);
        if bytes.len() > MAX_MESSAGE { return Err("Nearby connection information is too large".into()); }
        Ok(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor() -> PublicLibrary {
        PublicLibrary { version:1,public_id:uuid::Uuid::new_v4().to_string(),session_id:"session".into(),
            peer_id:"peer".into(),name:"Public library".into(),port:1234,addresses:vec!["192.168.1.2".into()],join:None,can_join:false }
    }
    #[test]
    fn public_metadata_rejects_remote_routes_and_bad_credentials() {
        let mut row = descriptor();
        assert!(row.validate(&row.public_id).is_ok());
        assert!(row.validate(&uuid::Uuid::new_v4().to_string()).is_err());
        for address in ["8.8.8.8", "127.0.0.1", "0.0.0.0", "sender.local", "192.168.1.2:1234",
            "::", "::1", "fe80::1", "fe80::1%wlan0", "ff02::1", "::ffff:192.168.1.2", "2001:4860:4860::8888"] {
            row.addresses = vec![address.into()]; assert!(row.validate(&row.public_id).is_err());
        }
        row.addresses = vec!["192.168.1.2".into()];
        row.join = Some(TemporaryNetwork { reservation_id:uuid::Uuid::new_v4().to_string(),ssid:"temporary".into(),passphrase:Zeroizing::new("short".into()) });
        assert!(row.validate(&row.public_id).is_err());
    }
    #[test]
    fn dual_stack_library_can_publish_and_decode_private_ipv6_hints() {
        let mut row = descriptor();
        row.addresses.extend(["fd12:3456:789a:419c::10".into(),
            "fd12:3456:789a:419c::11".into()]);
        let bytes = row.encode().unwrap();
        let decoded: PublicLibrary = serde_json::from_slice(&bytes).unwrap();
        assert!(decoded.validate(&row.public_id).is_ok());
        assert_eq!(decoded.addresses, row.addresses);
    }
    #[test]
    fn metadata_is_repeatable_for_independent_receivers() {
        let row = descriptor(); let bytes = row.encode().unwrap();
        for _ in 0..32 {
            let mut tx = Framing::new(20).unwrap(); let mut rx = Framing::new(20).unwrap(); let mut result = None;
            for packet in tx.fragment(Kind::Metadata, &bytes).unwrap() { result = rx.receive(Kind::Metadata,&packet).unwrap(); }
            let decoded: PublicLibrary = serde_json::from_slice(&result.unwrap()).unwrap();
            assert!(decoded.validate(&row.public_id).is_ok());
        }
    }
    #[test]
    fn receiver_host_offer_stays_bound_to_selected_public_share() {
        let mut row = descriptor(); row.addresses.clear(); row.can_join = true;
        // No endpoint is invented before the sender has joined the receiver AP.
        assert!(row.validate(&row.public_id).is_ok());
        let mut request = HostRequest { version:1,public_id:row.public_id.clone(),address:"192.168.43.1".into(),
            join:TemporaryNetwork { reservation_id:uuid::Uuid::new_v4().to_string(),ssid:"App temporary AP".into(),
                passphrase:Zeroizing::new("temporary-password".into()) } };
        assert!(request.validate(&row).is_ok());
        request.public_id = uuid::Uuid::new_v4().to_string();
        assert!(request.validate(&row).is_err());
        request.public_id = row.public_id.clone(); request.address = "8.8.8.8".into();
        assert!(request.validate(&row).is_err());
        request.address = "192.168.43.1".into(); request.join.ssid = "invalid\nnetwork".into();
        assert!(request.validate(&row).is_err());
    }
    #[test]
    fn hosting_exchange_preserves_order_and_rejects_replayed_fragments() {
        let row = descriptor();
        let messages: [&[u8]; 3] = [row.public_id.as_bytes(),b"hosting",b"done"];
        let mut tx = Framing::new(20).unwrap(); let mut rx = Framing::new(20).unwrap();
        let mut first = None;
        for message in messages {
            let packets = tx.fragment(Kind::Request,message).unwrap();
            if first.is_none() { first = Some(packets[0].clone()); }
            let mut complete = None;
            for packet in packets { complete = rx.receive(Kind::Request,&packet).unwrap(); }
            assert_eq!(complete.unwrap().as_slice(),message);
        }
        assert!(rx.receive(Kind::Request,&first.unwrap()).is_err());
    }
}
