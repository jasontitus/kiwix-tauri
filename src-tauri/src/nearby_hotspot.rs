// SPDX-License-Identifier: GPL-3.0-or-later
//! Match the hotspot controller's interface receipt against current kernel state.
#![forbid(unsafe_code)]

use nix::ifaddrs::{getifaddrs, InterfaceAddress};
use nix::net::if_::{if_nametoindex, InterfaceFlags};
use std::{ffi::CString, net::Ipv4Addr};

use super::LOST;

pub(super) struct Receipt<'a> {
    pub(super) lease: &'a str,
    name: CString,
    index: u32,
    address: Ipv4Addr,
    mask: u32,
}

impl<'a> Receipt<'a> {
    pub(super) fn parse(
        reply: &'a serde_json::Value,
        peer: Option<Ipv4Addr>,
    ) -> Result<Self, String> {
        let field = |key| {
            reply
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or("Invalid native hotspot interface receipt")
        };
        let address: Ipv4Addr = field("address")?
            .parse()
            .map_err(|_| "Invalid native hotspot IPv4 address")?;
        if !(address.is_private() || address.is_link_local()) {
            return Err("Invalid native hotspot IPv4 address".into());
        }
        let lease = field("lease")?;
        uuid::Uuid::parse_str(lease).map_err(|_| "Invalid native hotspot lease")?;
        let name = field("interfaceName")?;
        if name.is_empty() || name.len() >= libc::IF_NAMESIZE || !name.is_ascii() {
            return Err("Invalid native hotspot interface name".into());
        }
        let name = CString::new(name).map_err(|_| "Invalid native hotspot interface name")?;
        let index = reply
            .get("interfaceIndex")
            .and_then(serde_json::Value::as_u64)
            .and_then(|i| u32::try_from(i).ok())
            .filter(|i| *i > 0)
            .ok_or("Invalid native hotspot interface index")?;
        let prefix = reply
            .get("prefixLength")
            .and_then(serde_json::Value::as_u64)
            .filter(|p| (1..=32).contains(p))
            .ok_or("Invalid native hotspot IPv4 prefix")?;
        let mask = u32::MAX << (32 - prefix);
        if let Some(peer) = peer {
            let number = u32::from(peer);
            let host = number & !mask;
            if peer == address
                || number & mask != u32::from(address) & mask
                || (prefix < 31 && (host == 0 || host == !mask))
            {
                return Err("The selected peer is not on the owned hotspot interface".into());
            }
        }
        Ok(Self {
            lease,
            name,
            index,
            address,
            mask,
        })
    }

    fn matches(&self, row: &InterfaceAddress) -> bool {
        row.interface_name.as_bytes() == self.name.as_bytes()
            && row
                .flags
                .contains(InterfaceFlags::IFF_UP | InterfaceFlags::IFF_RUNNING)
            && !row
                .flags
                .intersects(InterfaceFlags::IFF_LOOPBACK | InterfaceFlags::IFF_POINTOPOINT)
            && row
                .address
                .as_ref()
                .and_then(|a| a.as_sockaddr_in())
                .map(|a| a.ip())
                == Some(self.address)
            && row
                .netmask
                .as_ref()
                .and_then(|a| a.as_sockaddr_in())
                .map(|a| u32::from(a.ip()))
                == Some(self.mask)
    }

    fn check(&self) -> Result<(), String> {
        if if_nametoindex(self.name.as_c_str()) != Ok(self.index) {
            return Err(LOST.into());
        }
        let mut rows = getifaddrs().map_err(|error| error.to_string())?;
        if !rows.any(|row| self.matches(&row)) {
            return Err(LOST.into());
        }
        Ok(())
    }

    #[cfg(any(target_os = "android", target_os = "linux"))]
    pub(super) fn bind_socket(&self) -> Result<std::net::UdpSocket, String> {
        use socket2::{Domain, Protocol, Socket, Type};
        self.check()?;
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
            .map_err(|e| e.to_string())?;
        if socket.r#type().ok() != Some(Type::DGRAM) {
            return Err("The owned hotspot requires a UDP socket".into());
        }
        socket
            .bind_device(Some(self.name.as_bytes_with_nul()))
            .map_err(|e| {
                format!("Android cannot bind QUIC to the exact temporary-hotspot interface: {e}")
            })?;
        if socket
            .device()
            .map_err(|_| "Android did not retain the exact hotspot interface binding")?
            .as_deref()
            != Some(self.name.as_bytes())
        {
            return Err("Android did not retain the exact hotspot interface binding".into());
        }
        socket
            .bind(&std::net::SocketAddrV4::new(self.address, 0).into())
            .map_err(|e| e.to_string())?;
        self.check()?;
        Ok(socket.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddrV4;

    fn reply() -> serde_json::Value {
        serde_json::json!({"address":"192.168.23.1","interfaceName":"ap0","interfaceIndex":4,"prefixLength":24,
            "lease":"fcfaa0d2-e067-4bb7-9beb-9f7a87173c6f"})
    }

    #[test]
    fn route_requires_same_active_interface_address_and_mask() {
        let json = reply();
        let receipt = Receipt::parse(&json, Some("192.168.23.2".parse().unwrap())).unwrap();
        assert_eq!(receipt.lease, json["lease"].as_str().unwrap());
        let row = InterfaceAddress {
            interface_name: "ap0".to_string(),
            flags: InterfaceFlags::IFF_UP | InterfaceFlags::IFF_RUNNING,
            address: Some(SocketAddrV4::new(receipt.address, 0).into()),
            netmask: Some(SocketAddrV4::new(Ipv4Addr::from(receipt.mask), 0).into()),
            broadcast: None,
            destination: None,
        };
        assert!(receipt.matches(&row));
        for flags in [
            InterfaceFlags::IFF_UP,
            InterfaceFlags::IFF_RUNNING,
            row.flags | InterfaceFlags::IFF_LOOPBACK,
            row.flags | InterfaceFlags::IFF_POINTOPOINT,
        ] {
            assert!(!receipt.matches(&InterfaceAddress {
                flags,
                ..row.clone()
            }));
        }
        assert!(!receipt.matches(&InterfaceAddress {
            interface_name: "ap1".into(),
            ..row.clone()
        }));
        assert!(!receipt.matches(&InterfaceAddress {
            address: None,
            ..row.clone()
        }));
        assert!(!receipt.matches(&InterfaceAddress {
            netmask: None,
            ..row.clone()
        }));
        assert!(!receipt.matches(&InterfaceAddress {
            netmask: row.address,
            ..row.clone()
        }));
        for ip in [
            "192.168.23.1",
            "192.168.24.2",
            "192.168.23.0",
            "192.168.23.255",
        ] {
            assert!(Receipt::parse(&json, Some(ip.parse().unwrap())).is_err());
        }
        let wrong_address = SocketAddrV4::new("192.168.23.2".parse().unwrap(), 0);
        assert!(!receipt.matches(&InterfaceAddress {
            address: Some(wrong_address.into()),
            ..row.clone()
        }));
        let ipv6 = std::net::SocketAddrV6::new(std::net::Ipv6Addr::LOCALHOST, 0, 0, 0);
        assert!(!receipt.matches(&InterfaceAddress {
            netmask: Some(ipv6.into()),
            ..row.clone()
        }));
        assert!(!receipt.matches(&InterfaceAddress {
            address: Some(ipv6.into()),
            ..row.clone()
        }));
        let mut point_to_point = json.clone();
        point_to_point["prefixLength"] = 31.into();
        assert!(Receipt::parse(&point_to_point, Some("192.168.23.0".parse().unwrap())).is_ok());
        point_to_point["prefixLength"] = 32.into();
        assert!(Receipt::parse(&point_to_point, None).is_ok());
        assert!(Receipt::parse(&point_to_point, Some("192.168.23.0".parse().unwrap())).is_err());
        let mut invalid = json;
        invalid["interfaceName"] = "ap0\0other".into();
        assert!(Receipt::parse(&invalid, None).is_err());
    }

    #[test]
    fn retired_interface_index_cannot_be_used() {
        let mut json = reply();
        json["interfaceIndex"] = u32::MAX.into();
        let receipt = Receipt::parse(&json, None).unwrap();
        assert!(receipt.check().is_err());
        #[cfg(any(target_os = "android", target_os = "linux"))]
        assert!(receipt.bind_socket().is_err());
    }
}
