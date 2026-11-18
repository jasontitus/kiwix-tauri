// SPDX-License-Identifier: GPL-3.0-or-later
//! A receipt belongs to its selected interface and local address, not the
//! inventory of unrelated cellular, VPN, or other LAN interfaces on the device.
#![forbid(unsafe_code)]
use crate::transfer_policy::{self, LocalBinding};
use std::{net::IpAddr, sync::{Arc, Mutex}};
#[derive(Clone, Default)]
pub(crate) struct LibraryRouteAuthority(Arc<Mutex<Option<Receipt>>>);
struct Receipt {
    revision: Option<u64>,
    selected: SelectedInterface,
    binding: Option<(LocalBinding, IpAddr)>,
    native_index: Option<u32>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct SelectedInterface {
    name: String,
    index: Option<u32>,
    local_ip: IpAddr,
}
#[cfg(unix)]
fn interface_index(name: &str) -> Result<u32, String> {
    if name.as_bytes().contains(&0) { return Err("Invalid selected interface".into()); }
    nix::net::if_::if_nametoindex(name).map_err(|_| "The selected Nearby interface retired".to_string())
}
#[cfg(any(target_os = "ios", target_os = "macos"))]
fn native_name(index: u32) -> Result<String, String> {
    nix::net::if_::if_indextoname(index)
        .map_err(|_| "The selected native QUIC interface retired".to_string())?
        .into_string().map_err(|error| error.to_string())
}
impl SelectedInterface {
    fn from_local_ip(local_ip: IpAddr) -> Result<Self, String> {
        let interfaces = if_addrs::get_if_addrs().map_err(|e| e.to_string())?;
        let mut name: Option<String> = None;
        for interface in interfaces.iter().filter(|interface| interface.ip() == local_ip) {
            if name.as_ref().is_some_and(|name| name != &interface.name) {
                return Err("The selected local address belongs to multiple interfaces".into());
            }
            if name.is_none() { name = Some(interface.name.clone()); }
        }
        let name = name.ok_or("The bound QUIC local address has no live interface")?;
        #[cfg(unix)]
        let index = Some(interface_index(&name)?);
        #[cfg(not(unix))]
        let index = None;
        Ok(Self { name, index, local_ip })
    }
    #[cfg(any(not(any(target_os = "ios", target_os = "macos")), test))]
    fn available<'a>(&self, mut addresses: impl Iterator<Item = (&'a str, IpAddr)>) -> bool {
        addresses.any(|(name, ip)| name == self.name && ip == self.local_ip)
    }
    fn check(&self) -> Result<(), String> {
        #[cfg(unix)]
        if self.index != Some(interface_index(&self.name)?) {
            return Err("The selected Nearby interface was replaced".into());
        }
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        {
            let index = self.index.ok_or("The selected native interface has no index")?;
            if !super::apple_transport::interface_has_address(index, self.local_ip) {
                return Err("The selected Nearby local address retired; browse again".into());
            }
            Ok(())
        }
        #[cfg(not(any(target_os = "ios", target_os = "macos")))]
        {
            let interfaces = if_addrs::get_if_addrs().map_err(|e| e.to_string())?;
            if !self.available(interfaces.iter().map(|interface| (interface.name.as_str(), interface.ip()))) {
                return Err("The selected Nearby local address retired; browse again".into());
            }
            Ok(())
        }
    }
}
impl LibraryRouteAuthority {
    pub(crate) fn check(&self) -> Result<(), String> {
        let receipt = self.0.lock().map_err(|_| "Nearby route unavailable")?;
        if let Some(receipt) = receipt.as_ref() {
            if let Some(revision) = receipt.revision { transfer_policy::require_local_since(revision)?; }
            receipt.selected.check()?;
            if let Some((binding, peer)) = receipt.binding {
                if !binding.is_on_link_peer(peer)? { return Err("The selected Nearby route is no longer local".into()); }
            }
        }
        Ok(())
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    pub(super) fn native_interface(&self) -> Result<Option<u32>, String> {
        self.check()?;
        Ok(self.0.lock().map_err(|_| "Nearby route unavailable")?.as_ref().and_then(|receipt| receipt.native_index))
    }
    #[cfg(any(target_os = "ios", target_os = "macos"))]
    pub(super) fn retain_native(&self, index: u32, local_ip: IpAddr) -> Result<(), String> {
        if index == 0 { return Err("Invalid native QUIC interface receipt".into()); }
        let selected = SelectedInterface { name: native_name(index)?, index: Some(index), local_ip };
        selected.check()?;
        self.retain_inner(None, selected, None, Some(index))
    }
    pub(super) fn retain(&self, revision: u64, local_ip: IpAddr, binding: (LocalBinding, IpAddr)) -> Result<(), String> {
        self.retain_inner(Some(revision), SelectedInterface::from_local_ip(local_ip)?, Some(binding), None)
    }
    fn retain_inner(&self, revision: Option<u64>, selected: SelectedInterface, binding: Option<(LocalBinding, IpAddr)>, native_index: Option<u32>) -> Result<(), String> {
        self.check()?;
        let mut receipt = self.0.lock().map_err(|_| "Nearby route unavailable")?;
        if let Some(current) = receipt.as_ref() {
            if current.selected != selected || current.binding != binding || current.native_index != native_index {
                return Err("The selected Nearby route changed; browse again".into());
            }
        } else { *receipt = Some(Receipt { revision, selected, binding, native_index }); }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_route_ignores_unrelated_networks_and_extra_addresses() {
        let selected = SelectedInterface { name: "awdl0".into(), index: Some(7), local_ip: "fe80::1234".parse().unwrap() };
        let addresses = |values: &[(&str, &str)]| values.iter().map(|(name, ip)| ((*name).to_string(), ip.parse::<IpAddr>().unwrap())).collect::<Vec<_>>();
        for current in [
            addresses(&[("awdl0", "fe80::1234")]),
            addresses(&[("pdp_ip0", "10.0.0.9"), ("awdl0", "fe80::1234"), ("utun5", "fd00::2"), ("en0", "192.168.2.9")]),
            addresses(&[("awdl0", "fe80::1234"), ("awdl0", "fd00::99")]),
        ] {
            assert!(selected.available(current.iter().map(|(name, ip)| (name.as_str(), *ip))));
        }
        for current in [
            addresses(&[("awdl0", "fe80::5678")]),
            addresses(&[("utun5", "fe80::1234")]),
            addresses(&[]),
        ] {
            assert!(!selected.available(current.iter().map(|(name, ip)| (name.as_str(), *ip))));
        }
    }
}

#[cfg(all(test, any(target_os = "ios", target_os = "macos")))]
#[test]
#[ignore = "Requires an active native interface with an IPv6 link-local address"]
fn native_link_local_receipt_uses_unfiltered_os_addresses() {
    let selected = nix::ifaddrs::getifaddrs().unwrap().find_map(|interface| {
        if !interface.flags.contains(nix::net::if_::InterfaceFlags::IFF_UP) {
            return None;
        }
        let ip = interface.address?.as_sockaddr_in6()?.ip();
        if !ip.is_unicast_link_local() {
            return None;
        }
        // Darwin embeds the scope index into the second octet pair.
        let mut bytes = ip.octets();
        bytes[2..4].fill(0);
        let name = interface.interface_name;
        Some(SelectedInterface { index: Some(interface_index(&name).unwrap()), name,
            local_ip: std::net::Ipv6Addr::from(bytes).into() })
    });
    let selected = selected.expect("An active IPv6 link-local interface is required for this native regression");
    selected.check().expect("A native link-local receipt must not depend on if-addrs feature filtering");
    let mut missing = selected.clone();
    missing.index = Some(u32::MAX);
    assert!(missing.check().is_err());
}
