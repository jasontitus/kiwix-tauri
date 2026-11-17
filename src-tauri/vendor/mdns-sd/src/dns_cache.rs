//! A cache for DNS records.
//!
//! This is an internal implementation, not visible to the public API.

#[cfg(feature = "logging")]
use crate::log::{debug, trace};
use crate::{
    dns_parser::{DnsAddress, DnsPointer, DnsRecordBox, DnsSrv, DnsTxt, InterfaceId, RRType},
    service_info::{split_sub_domain, MyIntf},
    ScopedIp,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    time::SystemTime,
};

// Admission limits apply to the whole cache, not individual RR types or packets.
const MAX_RECORDS: usize = 1024;
const MAX_KEYS: usize = 1024;
const MAX_RECORDS_PER_OWNER: usize = 64;
const MAX_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

/// Associate a DnsRecord with the interface it was received on.
pub(crate) struct DnsRecordIntf {
    pub(crate) record: DnsRecordBox,
    pub(crate) src_intf: InterfaceId,
    cache_id: u64,
    payload_bytes: usize,
}

/// A cache for all types of DNS records.
pub(crate) struct DnsCache {
    /// DnsPointer records indexed by ty_domain
    ptr: HashMap<String, Vec<DnsRecordIntf>>,

    /// DnsSrv records indexed by the fullname of an instance
    srv: HashMap<String, Vec<DnsRecordIntf>>,

    /// DnsTxt records indexed by the fullname of an instance
    txt: HashMap<String, Vec<DnsRecordIntf>>,

    /// DnsAddr records indexed by the hostname in lowercase.
    addr: HashMap<String, Vec<DnsRecordIntf>>,

    /// Reverse index of live subtype PTR record IDs, grouped by instance fullname.
    subtype: HashMap<String, BTreeSet<u64>>,

    /// Negative responses:
    /// A map from "instance fullname" to DnsNSec.
    nsec: HashMap<String, Vec<DnsRecordIntf>>,

    record_count: usize,
    payload_bytes: usize,
    next_id: u64,
    // Only live records occur here. Refreshes never append historical owners.
    admission_order: BTreeMap<u64, (RRType, String)>,
    // Reverse record IDs let pressure invalidation follow only actual owners.
    ptr_owners: HashMap<String, BTreeSet<u64>>,
    host_owners: HashMap<String, BTreeSet<u64>>,
    pressure_evictions: HashMap<String, HashSet<String>>,
    pressure_eviction_count: usize,
    pressure_eviction_bytes: usize,
}

impl DnsCache {
    pub(crate) fn new() -> Self {
        Self {
            ptr: HashMap::new(),
            srv: HashMap::new(),
            txt: HashMap::new(),
            addr: HashMap::new(),
            subtype: HashMap::new(),
            nsec: HashMap::new(),
            record_count: 0,
            payload_bytes: 0,
            next_id: 0,
            admission_order: BTreeMap::new(),
            ptr_owners: HashMap::new(),
            host_owners: HashMap::new(),
            pressure_evictions: HashMap::new(),
            pressure_eviction_count: 0,
            pressure_eviction_bytes: 0,
        }
    }

    pub(crate) fn all_ptr(&self) -> &HashMap<String, Vec<DnsRecordIntf>> {
        &self.ptr
    }

    /// Count all PTR records in the cache.
    pub(crate) fn ptr_count(&self) -> usize {
        self.ptr.values().map(|v| v.len()).sum()
    }

    pub(crate) fn srv_count(&self) -> usize {
        self.srv.values().map(|v| v.len()).sum()
    }

    pub(crate) fn txt_count(&self) -> usize {
        self.txt.values().map(|v| v.len()).sum()
    }

    pub(crate) fn addr_count(&self) -> usize {
        self.addr.values().map(|v| v.len()).sum()
    }

    pub(crate) fn nsec_count(&self) -> usize {
        self.nsec.values().map(|v| v.len()).sum()
    }

    pub(crate) fn subtype_count(&self) -> usize {
        self.subtype.len()
    }

    pub(crate) fn get_ptr(&self, ty_domain: &str) -> Option<&Vec<DnsRecordIntf>> {
        self.ptr.get(ty_domain)
    }

    pub(crate) fn get_srv(&self, fullname: &str) -> Option<&Vec<DnsRecordIntf>> {
        self.srv.get(fullname)
    }

    pub(crate) fn get_txt(&self, fullname: &str) -> Option<&Vec<DnsRecordIntf>> {
        self.txt.get(fullname)
    }

    pub(crate) fn get_addr(&self, hostname: &str) -> Option<&Vec<DnsRecordIntf>> {
        self.addr.get(&hostname.to_lowercase())
    }

    /// A reverse lookup table from "instance fullname" to "subtype PTR name"
    pub(crate) fn get_subtype(&self, fullname: &str) -> Option<&String> {
        let id = self.subtype.get(fullname)?.first()?;
        self.admission_order.get(id).map(|(_, owner)| owner)
    }

    fn records(&self, ty: RRType) -> Option<&HashMap<String, Vec<DnsRecordIntf>>> {
        match ty {
            RRType::PTR => Some(&self.ptr),
            RRType::SRV => Some(&self.srv),
            RRType::TXT => Some(&self.txt),
            RRType::A | RRType::AAAA => Some(&self.addr),
            RRType::NSEC => Some(&self.nsec),
            _ => None,
        }
    }

    fn records_mut(&mut self, ty: RRType) -> &mut HashMap<String, Vec<DnsRecordIntf>> {
        match ty {
            RRType::PTR => &mut self.ptr,
            RRType::SRV => &mut self.srv,
            RRType::TXT => &mut self.txt,
            RRType::A | RRType::AAAA => &mut self.addr,
            RRType::NSEC => &mut self.nsec,
            _ => unreachable!("unsupported cache record"),
        }
    }

    fn key_count(&self) -> usize {
        self.ptr.len() + self.srv.len() + self.txt.len() + self.addr.len()
            + self.nsec.len() + self.subtype.len()
    }

    // Charge duplicate map/index names per record (even when shared). NSEC's
    // private rdata has no borrowed accessor: a terminating decoded name cannot
    // exceed a UDP datagram's 65535 bytes, and its bitmap length is one octet.
    // The factor of two also covers geometric String/Vec spare capacity in the
    // parser. Fixed record/container overhead is separately bounded by counts.
    fn payload_charge(record: &DnsRecordBox) -> Option<usize> {
        let data = match record.get_type() {
            RRType::PTR => record.any().downcast_ref::<DnsPointer>()?.alias().len().checked_mul(2)?,
            RRType::SRV => record.any().downcast_ref::<DnsSrv>()?.host().len().checked_mul(2)?,
            RRType::TXT => record.any().downcast_ref::<DnsTxt>()?.text().len(),
            RRType::A | RRType::AAAA => {
                record.any().downcast_ref::<DnsAddress>()?;
                0
            }
            RRType::NSEC => 65535 + 255,
            _ => return None,
        };
        let subtype = if record.get_type() == RRType::PTR {
            split_sub_domain(record.get_name()).1.map_or(0, |s| s.len() + data)
        } else {
            0
        };
        record.get_name().len().checked_mul(3)?
            .checked_add(data)?.checked_add(subtype)?.checked_mul(2)
    }

    fn add_index(index: &mut HashMap<String, BTreeSet<u64>>, key: &str, id: u64) {
        if let Some(ids) = index.get_mut(key) {
            ids.insert(id);
        } else {
            index.insert(key.to_string(), BTreeSet::from([id]));
        }
    }

    fn add_record_indexes(
        ptr_owners: &mut HashMap<String, BTreeSet<u64>>,
        host_owners: &mut HashMap<String, BTreeSet<u64>>,
        subtypes: &mut HashMap<String, BTreeSet<u64>>,
        record: &DnsRecordBox,
        id: u64,
    ) {
        if let Some(ptr) = record.any().downcast_ref::<DnsPointer>() {
            Self::add_index(ptr_owners, ptr.alias(), id);
            if split_sub_domain(record.get_name()).1.is_some() {
                Self::add_index(subtypes, ptr.alias(), id);
            }
        } else if let Some(srv) = record.any().downcast_ref::<DnsSrv>() {
            Self::add_index(host_owners, &srv.host().to_lowercase(), id);
        }
    }

    fn evict_record(&mut self, id: u64) {
        let Some((ty, key)) = self.admission_order.remove(&id) else {
            return;
        };
        let records = self.records_mut(ty);
        let values = records.get_mut(&key).expect("indexed cache owner");
        let index = values.iter().position(|r| r.cache_id == id).expect("indexed cache record");
        let removed = values.remove(index);
        if values.is_empty() {
            records.remove(&key);
        }
        self.record_count -= 1;
        self.payload_bytes -= removed.payload_bytes;
        Self::forget_record_indexes(
            &mut self.ptr_owners, &mut self.host_owners, &mut self.subtype, &removed.record, id,
        );
    }

    fn forget_index(index: &mut HashMap<String, BTreeSet<u64>>, key: &str, id: u64) {
        if let Some(ids) = index.get_mut(key) {
            ids.remove(&id);
            if ids.is_empty() {
                index.remove(key);
            }
        }
    }

    fn forget_record_indexes(
        ptr_owners: &mut HashMap<String, BTreeSet<u64>>,
        host_owners: &mut HashMap<String, BTreeSet<u64>>,
        subtypes: &mut HashMap<String, BTreeSet<u64>>,
        record: &DnsRecordBox,
        id: u64,
    ) {
        if let Some(ptr) = record.any().downcast_ref::<DnsPointer>() {
            Self::forget_index(ptr_owners, ptr.alias(), id);
            if split_sub_domain(record.get_name()).1.is_some() {
                Self::forget_index(subtypes, ptr.alias(), id);
            }
        } else if let Some(srv) = record.any().downcast_ref::<DnsSrv>() {
            Self::forget_index(host_owners, &srv.host().to_lowercase(), id);
        }
    }

    /// Drained after each admission by the daemon. Undrained notifications
    /// backpressure further pressure admissions rather than losing removals.
    pub(crate) fn take_pressure_evictions(&mut self) -> HashMap<String, HashSet<String>> {
        self.pressure_eviction_count = 0;
        self.pressure_eviction_bytes = 0;
        std::mem::take(&mut self.pressure_evictions)
    }

    fn invalidate_instance<'a>(&'a self, instance: &str, invalidated: &mut HashSet<(&'a str, &'a str)>) {
        if let Some((instance, ptrs)) = self.ptr_owners.get_key_value(instance) {
            for ptr_id in ptrs {
                if let Some((_, domain)) = self.admission_order.get(ptr_id) {
                    invalidated.insert((domain.as_str(), instance.as_str()));
                }
            }
        }
    }

    fn pressure_evict_record(&mut self, id: u64) -> bool {
        let Some((ty, owner)) = self.admission_order.get(&id) else {
            return true;
        };
        let mut invalidated = HashSet::new();
        match ty {
            RRType::PTR => {
                let record = self.ptr[owner].iter().find(|r| r.cache_id == id).unwrap();
                if let Some(ptr) = record.record.any().downcast_ref::<DnsPointer>() {
                    invalidated.insert((owner.as_str(), ptr.alias()));
                }
            }
            RRType::SRV | RRType::TXT => self.invalidate_instance(owner, &mut invalidated),
            RRType::A | RRType::AAAA => {
                if let Some(srvs) = self.host_owners.get(owner) {
                    for srv_id in srvs {
                        if let Some((_, instance)) = self.admission_order.get(srv_id) {
                            self.invalidate_instance(instance, &mut invalidated);
                        }
                    }
                }
            }
            _ => {}
        }
        invalidated.retain(|(domain, instance)| {
            !self.pressure_evictions.get(*domain).is_some_and(|names| names.contains(*instance))
        });
        let bytes: usize = invalidated.iter().map(|(domain, instance)| 2 * (domain.len() + instance.len())).sum();
        if self.pressure_eviction_count + invalidated.len() > MAX_RECORDS
            || self.pressure_eviction_bytes + bytes > MAX_PAYLOAD_BYTES
        {
            return false;
        }
        let additions: Vec<_> = invalidated.into_iter()
            .map(|(domain, instance)| (domain.to_string(), instance.to_string())).collect();
        self.pressure_eviction_count += additions.len();
        self.pressure_eviction_bytes += bytes;
        for (domain, instance) in additions {
            self.pressure_evictions.entry(domain).or_default().insert(instance);
        }
        self.evict_record(id);
        true
    }

    fn retain_records(&mut self, ty: RRType, mut keep: impl FnMut(&DnsRecordIntf) -> bool) {
        let Self { ptr, srv, txt, addr, nsec, record_count, payload_bytes,
            admission_order, ptr_owners, host_owners, subtype, .. } = self;
        let records = match ty {
            RRType::PTR => ptr,
            RRType::SRV => srv,
            RRType::TXT => txt,
            RRType::A | RRType::AAAA => addr,
            RRType::NSEC => nsec,
            _ => unreachable!("unsupported cache record"),
        };
        records.retain(|_, values| {
            values.retain(|value| {
                if keep(value) {
                    return true;
                }
                *record_count -= 1;
                *payload_bytes -= value.payload_bytes;
                admission_order.remove(&value.cache_id);
                Self::forget_record_indexes(
                    ptr_owners, host_owners, subtype,
                    &value.record, value.cache_id,
                );
                false
            });
            !values.is_empty()
        });
    }

    // Service/interface removal already traverses the cache. Reconcile their
    // bulk removals without allocating a second copy of the owner index.
    fn reconcile(&mut self) {
        self.record_count = 0;
        self.payload_bytes = 0;
        for records in [&mut self.ptr, &mut self.srv, &mut self.txt, &mut self.addr, &mut self.nsec] {
            records.retain(|_, values| !values.is_empty());
            for value in records.values().flatten() {
                self.record_count += 1;
                self.payload_bytes += value.payload_bytes;
            }
        }
        let maps = [&self.ptr, &self.srv, &self.txt, &self.addr, &self.nsec];
        self.admission_order.retain(|id, (ty, key)| {
            let map = match ty {
                RRType::PTR => maps[0],
                RRType::SRV => maps[1],
                RRType::TXT => maps[2],
                RRType::A | RRType::AAAA => maps[3],
                RRType::NSEC => maps[4],
                _ => return false,
            };
            map.get(key).is_some_and(|values| values.iter().any(|value| value.cache_id == *id))
        });
        let admission_order = &self.admission_order;
        for index in [&mut self.subtype, &mut self.ptr_owners, &mut self.host_owners] {
            index.retain(|_, ids| {
                ids.retain(|id| admission_order.contains_key(id));
                !ids.is_empty()
            });
        }
    }

    pub(crate) fn contains_instance(&self, fullname: &str) -> bool {
        let now = current_time_millis();
        self.srv.get(fullname).into_iter().flatten()
            .chain(self.txt.get(fullname).into_iter().flatten())
            .any(|r| !r.record.get_record().is_expired(now))
            || self.ptr_owners.get(fullname).into_iter().flatten().any(|id| {
                self.admission_order.get(id)
                    .and_then(|(_, owner)| self.ptr.get(owner))
                    .is_some_and(|records| records.iter()
                        .any(|r| r.cache_id == *id && !r.record.get_record().is_expired(now)))
            })
    }

    #[cfg(test)]
    pub(crate) fn instance_names(&self) -> HashSet<String> {
        let now = current_time_millis();
        let mut names: HashSet<String> = self.srv.iter().chain(self.txt.iter())
            .filter(|(_, records)| records.iter().any(|r| !r.record.get_record().is_expired(now)))
            .map(|(name, _)| name.clone()).collect();
        for r in self.ptr.values().flatten().filter(|r| !r.record.get_record().is_expired(now)) {
            if let Some(ptr) = r.record.any().downcast_ref::<DnsPointer>() {
                names.insert(ptr.alias().to_string());
            }
        }
        names
    }

    pub(crate) fn next_deadline(&self, now: u64) -> Option<u64> {
        [&self.ptr, &self.srv, &self.txt, &self.addr, &self.nsec].iter()
            .flat_map(|map| map.values()).flatten()
            .map(|r| {
                let record = r.record.get_record();
                let refresh = record.get_refresh_time();
                if refresh > now {
                    record.get_expire_time().min(refresh)
                } else {
                    record.get_expire_time()
                }
            }).min()
    }

    /// Returns the list of instances that has `host` as its hostname.
    pub(crate) fn get_instances_on_host(&self, host: &str) -> Vec<String> {
        self.srv
            .iter()
            .filter_map(|(instance, srv_list)| {
                if let Some(item) = srv_list.first() {
                    if let Some(dns_srv) = item.record.any().downcast_ref::<DnsSrv>() {
                        if dns_srv.host() == host {
                            return Some(instance.clone());
                        }
                    }
                }
                None
            })
            .collect()
    }

    /// Returns a hashmap of hostnames and their addresses for a given `host`.
    ///
    /// Note that the keys in the returned HashMap are the same hostname, with different cases
    /// of letters (e.g. "example.local.", "Example.local.", "EXAMPLE.local.").
    pub(crate) fn get_addresses_for_host(&self, host: &str) -> HashMap<String, HashSet<ScopedIp>> {
        let hostname_lower = host.to_lowercase();
        let mut result = HashMap::new();

        if let Some(records) = self.addr.get(&hostname_lower) {
            for record in records {
                if let Some(dns_addr) = record.record.any().downcast_ref::<DnsAddress>() {
                    let record_name = record.record.get_name().to_string();
                    let address = dns_addr.address();

                    // Use the entry API to insert or update the HashSet for the record_name
                    result
                        .entry(record_name)
                        .or_insert_with(HashSet::new)
                        .insert(address);
                }
            }
        }

        result
    }

    /// Returns a list of resource records (name, rr_type) that need to be queried in order to
    /// verify the `instance`.
    ///
    /// If `expire_at` is not None, the resource records' expire time will be updated.
    pub(crate) fn service_verify_queries(
        &mut self,
        instance: &str,
        expire_at: Option<u64>,
    ) -> Vec<(String, RRType)> {
        let Some(srv_vec) = self.srv.get_mut(instance) else {
            return Vec::new();
        };

        let mut query_vec = vec![(instance.to_string(), RRType::SRV)];

        for srv in srv_vec {
            if let Some(new_expire) = expire_at {
                srv.record.set_expire_sooner(new_expire);
            }

            let Some(srv_record) = srv.record.any().downcast_ref::<DnsSrv>() else {
                continue;
            };

            // Will verify addresses for the hostname.
            query_vec.push((srv_record.host().to_string(), RRType::A));
            query_vec.push((srv_record.host().to_string(), RRType::AAAA));

            if let Some(new_expire) = expire_at {
                if let Some(addrs) = self.addr.get_mut(srv_record.host()) {
                    for addr in addrs {
                        addr.record.set_expire_sooner(new_expire);
                    }
                }
            }
        }

        query_vec
    }

    /// Update a DNSRecord TTL if already exists, otherwise insert a new record.
    ///
    /// Returns `None` if `incoming` is invalid / unrecognized, otherwise returns
    /// (a new record, true) or (existing record with TTL updated, false).
    ///
    /// If you need to add new timers for related records, push into `timers`.
    pub(crate) fn add_or_update(
        &mut self,
        intf: &MyIntf,
        incoming: DnsRecordBox,
        timers: &mut Vec<u64>,
        is_for_us: bool,
    ) -> Option<(&DnsRecordIntf, bool)> {
        let ty = incoming.get_type();
        let key = if matches!(ty, RRType::A | RRType::AAAA) {
            incoming.get_name().to_lowercase()
        } else {
            incoming.get_name().to_string()
        };
        let records = self.records(ty)?;
        let existing = records.get(&key);
        if existing.is_none() && !is_for_us {
            trace!("add_or_update: not for us: {}", incoming.get_name());
            return None;
        }
        let is_new = !existing.into_iter().flatten()
            .any(|r| r.record.matches(incoming.as_ref()));
        let charge = Self::payload_charge(&incoming)?;
        if charge > MAX_PAYLOAD_BYTES {
            return None;
        }
        let mut cache_id = 0;
        if is_new {
            // A hot owner cannot fill an unbounded RR vector. Replace its oldest
            // admission, then use the same FIFO policy for global pressure.
            if let Some(oldest) = existing.filter(|r| r.len() >= MAX_RECORDS_PER_OWNER)
                .and_then(|r| r.last()).map(|r| r.cache_id)
            {
                if !self.pressure_evict_record(oldest) {
                    return None;
                }
            }
            loop {
                let owner_key = usize::from(!self.records(ty).unwrap().contains_key(&key));
                let subtype_key = if ty == RRType::PTR && split_sub_domain(incoming.get_name()).1.is_some() {
                    incoming.any().downcast_ref::<DnsPointer>()
                        .map_or(0, |ptr| usize::from(!self.subtype.contains_key(ptr.alias())))
                } else {
                    0
                };
                if self.record_count < MAX_RECORDS
                    && self.key_count() + owner_key + subtype_key <= MAX_KEYS
                    && self.payload_bytes + charge <= MAX_PAYLOAD_BYTES
                {
                    break;
                }
                let id = *self.admission_order.first_key_value()?.0;
                if !self.pressure_evict_record(id) {
                    return None;
                }
            }
            if self.next_id == u64::MAX {
                // Preserve FIFO order even if the admission sequence rolls over.
                let order = std::mem::take(&mut self.admission_order);
                for (new_id, (old_id, (old_ty, old_key))) in order.into_iter().enumerate() {
                    let values = self.records_mut(old_ty).get_mut(&old_key).unwrap();
                    let record = values.iter_mut().find(|r| r.cache_id == old_id).unwrap();
                    record.cache_id = new_id as u64;
                    self.admission_order.insert(new_id as u64, (old_ty, old_key));
                }
                self.next_id = self.record_count as u64;
                self.subtype.clear();
                self.ptr_owners.clear();
                self.host_owners.clear();
                for record in self.ptr.values().chain(self.srv.values()).flatten() {
                    Self::add_record_indexes(
                        &mut self.ptr_owners, &mut self.host_owners, &mut self.subtype,
                        &record.record, record.cache_id,
                    );
                }
            }
            cache_id = self.next_id;
            self.next_id += 1;
            self.record_count += 1;
            self.payload_bytes += charge;
            self.admission_order.insert(cache_id, (ty, key.clone()));
            Self::add_record_indexes(
                &mut self.ptr_owners, &mut self.host_owners, &mut self.subtype, &incoming, cache_id,
            );
        }
        // Only admitted records may create an owner key or change flush timers.
        let record_vec = self.records_mut(ty).entry(key).or_default();

        if incoming.get_cache_flush() {
            let now = current_time_millis();
            let class = incoming.get_class();
            let rtype = incoming.get_type();

            record_vec.iter_mut().for_each(|r| {
                // When cache flush is asked, we set expire date to 1 second in the future if:
                // - The record has the same rclass
                // - The record was created more than 1 second ago.
                // - The record expire is more than 1 second away.
                // Ref: RFC 6762 Section 10.2
                //
                // Note: when the updated record actually expires, it will trigger events properly.
                let mut should_flush = false;

                if class == r.record.get_class()
                    && rtype == r.record.get_type()
                    && now > r.record.get_created() + 1000
                    && r.record.get_expire() > now + 1000
                {
                    should_flush = true;

                    // additional checks for address records.
                    if rtype == RRType::A || rtype == RRType::AAAA {
                        if let Some(addr) = r.record.any().downcast_ref::<DnsAddress>() {
                            if let Some(addr_b) = incoming.any().downcast_ref::<DnsAddress>() {
                                should_flush = addr.interface_id.index == addr_b.interface_id.index;
                            }
                        }
                    }
                }

                if should_flush {
                    trace!("FLUSH one record: {:?}", &r.record);
                    let new_expire = now + 1000;
                    r.record.set_expire(new_expire);

                    // Add a timer so the run loop will handle this expire.
                    timers.push(new_expire);
                }
            });
        }

        // update TTL for existing record or create a new record.
        let (idx, updated) = match record_vec
            .iter_mut()
            .enumerate()
            .find(|(_idx, r)| r.record.matches(incoming.as_ref()))
        {
            Some((i, r)) => {
                // It is possible that this record was just updated in cache_flush
                // processing. That's okay. We can still reset here.
                r.record.reset_ttl(incoming.as_ref());
                (i, false)
            }
            None => {
                let new_record = DnsRecordIntf {
                    record: incoming,
                    src_intf: intf.into(),
                    cache_id,
                    payload_bytes: charge,
                };
                record_vec.insert(0, new_record); // A new record.
                (0, true)
            }
        };

        Some((record_vec.get(idx).unwrap(), updated))
    }

    /// Remove a record from the cache if exists, otherwise no-op
    pub(crate) fn remove(&mut self, record: &DnsRecordBox) -> bool {
        let ty = record.get_type();
        let key = if matches!(ty, RRType::A | RRType::AAAA) {
            record.get_name().to_lowercase()
        } else {
            record.get_name().to_string()
        };
        let Some(records) = self.records(ty).and_then(|map| map.get(&key)) else {
            return false;
        };
        let ids: Vec<_> = records.iter().filter(|r| r.record.matches(record.as_ref()))
            .map(|r| r.cache_id).collect();
        let found = !ids.is_empty();
        for id in ids {
            self.evict_record(id);
        }
        found
    }

    /// Iterates all ADDR records and remove ones that expired.
    /// Returns the expired ones in a map of names and addresses.
    pub(crate) fn evict_expired_addr(&mut self, now: u64) -> HashMap<String, HashSet<ScopedIp>> {
        let mut removed = HashMap::new();

        self.retain_records(RRType::A, |addr| {
            let expired = addr.record.get_record().is_expired(now);
            if expired {
                if let Some(addr_record) = addr.record.any().downcast_ref::<DnsAddress>() {
                    trace!("evict expired ADDR: {:?}", addr_record);
                    removed.entry(addr.record.get_name().to_string())
                        .or_insert_with(HashSet::new).insert(addr_record.address());
                }
            }
            !expired
        });
        removed
    }

    /// Evicts expired PTR, SRV, TXT and NSEC records, including orphan records, and
    /// returns the set of expired instance names for each ty_domain.
    ///
    /// An instance in the returned set indicates its PTR and/or SRV record has expired.
    pub(crate) fn evict_expired_services(&mut self, now: u64) -> HashMap<String, HashSet<String>> {
        let mut expired_instances = HashMap::new();
        let expired_srv: HashSet<_> = self.srv.iter()
            .filter(|(_, records)| records.iter().all(|r| r.record.get_record().is_expired(now)))
            .map(|(name, _)| name.clone()).collect();
        for ty in [RRType::SRV, RRType::TXT, RRType::NSEC] {
            self.retain_records(ty, |r| !r.record.get_record().is_expired(now));
        }
        self.retain_records(RRType::PTR, |r| {
            let expired = r.record.get_record().is_expired(now);
            if let Some(ptr) = r.record.any().downcast_ref::<DnsPointer>() {
                if expired || expired_srv.contains(ptr.alias()) {
                    expired_instances.entry(r.record.get_name().to_string())
                        .or_insert_with(HashSet::new).insert(ptr.alias().to_string());
                }
            }
            !expired
        });
        expired_instances
    }

    /// Removes all records of a service type: PTR, SRV, TXT records and any ADDR records
    /// that are not referenced by any SRV record.
    pub(crate) fn remove_service_type(&mut self, ty_domain: &str) {
        let Some(ptr_records) = self.ptr.get_mut(ty_domain) else {
            return;
        };

        let mut hosts = HashSet::new();

        for ptr in ptr_records.iter() {
            if let Some(dns_ptr) = ptr.record.any().downcast_ref::<DnsPointer>() {
                let instance_name = dns_ptr.alias();

                // collect all hostnames from SRV records of this instance
                if let Some(srv_records) = self.srv.get_mut(instance_name) {
                    for srv in srv_records.iter() {
                        if let Some(dns_srv) = srv.record.any().downcast_ref::<DnsSrv>() {
                            hosts.insert(dns_srv.host().to_lowercase());
                        }
                    }
                }

                // remove all SRV records of this instance
                self.srv.remove(instance_name);

                // remove all TXT records of this instance
                self.txt.remove(instance_name);
            }
        }

        self.ptr.remove(ty_domain);

        // Check all hostnames in `hosts`: for each hostname, check if any SRV record
        // has `hostname` as its host. If no such SRV, remove the ADDR records of this hostname.
        for host in hosts {
            let mut has_srv = false;
            for srv_records in self.srv.values() {
                for srv in srv_records.iter() {
                    if let Some(dns_srv) = srv.record.any().downcast_ref::<DnsSrv>() {
                        if dns_srv.host().to_lowercase() == host {
                            has_srv = true;
                            break;
                        }
                    }
                }
                if has_srv {
                    break;
                }
            }

            if !has_srv {
                self.addr.remove(&host);
            }
        }
        self.reconcile();
    }

    /// Checks refresh due for PTR records of `ty_domain`.
    /// Returns all updated refresh time.
    pub(crate) fn refresh_due_ptr(&mut self, ty_domain: &str) -> HashSet<u64> {
        let now = current_time_millis();

        // Check all PTR records for this ty_domain.
        self.ptr
            .get_mut(ty_domain)
            .into_iter()
            .flatten()
            .filter_map(|record| record.record.updated_refresh_time(now))
            .collect()
    }

    /// Returns a tuple of:
    /// 1. the map of instance names together with RRType(s) that are due for refresh
    ///    its SRV or TXT records.
    /// 2. the set of new timers that are due for refresh.
    pub(crate) fn refresh_due_srv_txt(
        &mut self,
        ty_domain: &str,
    ) -> (HashMap<String, Vec<RRType>>, HashSet<u64>) {
        let now = current_time_millis();

        let instances: Vec<_> = self
            .ptr
            .get(ty_domain)
            .into_iter()
            .flatten()
            .filter(|record| !record.record.get_record().is_expired(now))
            .filter_map(|record| {
                record
                    .record
                    .any()
                    .downcast_ref::<DnsPointer>()
                    .map(|ptr| ptr.alias())
            })
            .collect();

        let mut refresh_due: HashMap<String, Vec<RRType>> = HashMap::new();
        let mut new_timers = HashSet::new();
        for instance in instances {
            // Check SRV records.
            let refresh_timers: HashSet<u64> = self
                .srv
                .get_mut(instance)
                .into_iter()
                .flatten()
                .filter_map(|record| record.record.updated_refresh_time(now))
                .collect();

            if !refresh_timers.is_empty() {
                refresh_due
                    .entry(instance.to_string())
                    .and_modify(|v| v.push(RRType::SRV))
                    .or_insert(vec![RRType::SRV]);
                new_timers.extend(refresh_timers);
            }

            // Check TXT records.
            let refresh_timers: HashSet<u64> = self
                .txt
                .get_mut(instance)
                .into_iter()
                .flatten()
                .filter_map(|record| record.record.updated_refresh_time(now))
                .collect();

            if !refresh_timers.is_empty() {
                refresh_due
                    .entry(instance.to_string())
                    .and_modify(|v| v.push(RRType::TXT))
                    .or_insert(vec![RRType::TXT]);
                new_timers.extend(refresh_timers);
            }
        }

        (refresh_due, new_timers)
    }

    /// Returns the set of `host`, where refreshing the A / AAAA records is due
    /// for a `ty_domain`.
    pub(crate) fn refresh_due_hosts(&mut self, ty_domain: &str) -> (HashSet<String>, HashSet<u64>) {
        let now = current_time_millis();

        let instances: Vec<_> = self
            .ptr
            .get(ty_domain)
            .into_iter()
            .flatten()
            .filter(|record| !record.record.get_record().is_expired(now))
            .filter_map(|record| {
                record
                    .record
                    .any()
                    .downcast_ref::<DnsPointer>()
                    .map(|ptr| ptr.alias())
            })
            .collect();

        // Collect hostnames we have browsers for by SRV records.
        let mut hostnames_browsed = HashSet::new();
        for instance in instances {
            let hosts: HashSet<String> = self
                .srv
                .get(instance)
                .into_iter()
                .flatten()
                .filter_map(|record| {
                    record
                        .record
                        .any()
                        .downcast_ref::<DnsSrv>()
                        .map(|srv| srv.host().to_string())
                })
                .collect();

            hostnames_browsed.extend(hosts);
        }
        let mut refresh_due = HashSet::new();
        let mut new_timers = HashSet::new();
        for hostname in hostnames_browsed {
            let refresh_timers: HashSet<u64> = self
                .addr
                .get_mut(&hostname)
                .into_iter()
                .flatten()
                .filter_map(|record| record.record.updated_refresh_time(now))
                .collect();

            if !refresh_timers.is_empty() {
                refresh_due.insert(hostname);
                new_timers.extend(refresh_timers);
            }
        }
        (refresh_due, new_timers)
    }

    /// Returns the set of A/AAAA records that are due for refresh for a `hostname`.
    ///
    /// For these records, their refresh time will be updated so that they will not refresh again.
    pub(crate) fn refresh_due_hostname_resolutions(
        &mut self,
        hostname: &str,
    ) -> HashSet<(String, ScopedIp)> {
        let now = current_time_millis();

        self.addr
            .get_mut(hostname)
            .into_iter()
            .flatten()
            .filter_map(|record| {
                let rec = record.record.get_record_mut();
                if rec.is_expired(now) || !rec.refresh_due(now) {
                    return None;
                }
                rec.refresh_no_more();

                Some((
                    hostname.to_owned(),
                    record
                        .record
                        .any()
                        .downcast_ref::<DnsAddress>()
                        .unwrap()
                        .address(),
                ))
            })
            .collect()
    }

    /// Returns a list of Known Answer for a given question of `name` with `qtype`.
    /// The timestamp `now` is passed in to check TTL.
    ///
    /// Reference:  RFC 6762 section 7.1
    pub(crate) fn get_known_answers<'a>(
        &'a self,
        name: &str,
        qtype: RRType,
        now: u64,
    ) -> Vec<&'a DnsRecordIntf> {
        let records_opt = match qtype {
            RRType::PTR => self.get_ptr(name),
            RRType::SRV => self.get_srv(name),
            RRType::A | RRType::AAAA => self.get_addr(name),
            RRType::TXT => self.get_txt(name),
            _ => None,
        };

        let records = match records_opt {
            Some(items) => items,
            None => return Vec::new(),
        };

        // From RFC 6762 section 7.1:
        // ..Generally, this applies only to Shared records, not Unique records,..
        //
        // ..a Multicast DNS querier SHOULD NOT include
        // records in the Known-Answer list whose remaining TTL is less than
        // half of their original TTL.
        records
            .iter()
            .filter(move |r| {
                !r.record.get_record().is_unique() && !r.record.get_record().halflife_passed(now)
            })
            .collect()
    }

    pub(crate) fn remove_addrs_on_disabled_intf(&mut self, disabled_if_index: u32) {
        for (host, records) in self.addr.iter_mut() {
            records.retain(|record| {
                let Some(dns_addr) = record.record.any().downcast_ref::<DnsAddress>() else {
                    return false; // invalid address record.
                };

                // Remove the record if it is on this interface.
                if dns_addr.interface_id.index == disabled_if_index {
                    debug!(
                        "removing ADDR on disabled intf: {:?} host {host}",
                        dns_addr.interface_id.name
                    );
                    false
                } else {
                    true
                }
            });
        }
        self.reconcile();
    }

    /// Removes all records that were received on `intf_id`.
    pub(crate) fn remove_records_on_intf(
        &mut self,
        intf_id: InterfaceId,
    ) -> HashMap<String, HashSet<String>> {
        let mut removed_instances = HashMap::new();

        self.ptr.iter_mut().for_each(|(name, records)| {
            let mut instances: HashSet<String> = HashSet::new();

            records.retain(|r| {
                if r.src_intf == intf_id {
                    if let Some(dns_ptr) = r.record.any().downcast_ref::<DnsPointer>() {
                        trace!("removing PTR on intf {:?}: {:?}", intf_id, dns_ptr);
                        instances.insert(dns_ptr.alias().to_string());
                    }
                    false
                } else {
                    true
                }
            });

            for instance in instances.drain() {
                // if no more record for this instance in `records`, it means its PTR record is removed.
                if !records.iter().any(|r| {
                    if let Some(dns_ptr) = r.record.any().downcast_ref::<DnsPointer>() {
                        dns_ptr.alias() == instance
                    } else {
                        false
                    }
                }) {
                    removed_instances
                        .entry(name.to_string())
                        .or_insert_with(HashSet::new)
                        .insert(instance);
                }
            }
        });

        self.ptr.retain(|_, records| !records.is_empty());

        self.srv.values_mut().for_each(|records| {
            records.retain(|r| r.src_intf != intf_id);
        });
        self.srv.retain(|_, records| !records.is_empty());

        self.txt.values_mut().for_each(|records| {
            records.retain(|r| r.src_intf != intf_id);
        });
        self.txt.retain(|_, records| !records.is_empty());

        self.addr.values_mut().for_each(|records| {
            records.retain(|r| r.src_intf != intf_id);
        });
        self.addr.retain(|_, records| !records.is_empty());

        self.nsec.values_mut().for_each(|records| {
            records.retain(|r| r.src_intf != intf_id);
        });
        self.nsec.retain(|_, records| !records.is_empty());

        self.reconcile();
        removed_instances
    }
}

/// Returns UNIX time in millis
pub(crate) fn current_time_millis() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("failed to get current UNIX time")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns_parser::{DnsNSec, DnsRecordExt};
    use std::net::{IpAddr, Ipv4Addr};

    fn intf() -> MyIntf {
        MyIntf { name: "cache-test".to_string(), index: 1, addrs: HashSet::new() }
    }

    fn insert(cache: &mut DnsCache, record: DnsRecordBox) {
        assert!(cache.add_or_update(&intf(), record, &mut Vec::new(), true).is_some());
        cache.take_pressure_evictions();
    }

    fn maps(cache: &DnsCache) -> [&HashMap<String, Vec<DnsRecordIntf>>; 5] {
        [&cache.ptr, &cache.srv, &cache.txt, &cache.addr, &cache.nsec]
    }

    fn assert_bounded(cache: &DnsCache) {
        let maps = maps(cache);
        assert!(maps.iter().map(|m| m.len()).sum::<usize>() + cache.subtype.len() <= 1024);
        assert!(maps.iter().flat_map(|m| m.values()).map(|r| r.len()).sum::<usize>() <= 1024);
        assert!(maps.iter().flat_map(|m| m.values()).all(|r| !r.is_empty() && r.len() <= 64));
        assert!(cache.ptr_owners.values().map(|ids| ids.len()).sum::<usize>() <= 1024);
        assert!(cache.host_owners.values().map(|ids| ids.len()).sum::<usize>() <= 1024);
        assert!(cache.subtype.values().map(|ids| ids.len()).sum::<usize>() <= 1024);
        assert!(cache.pressure_evictions.values().map(|names| names.len()).sum::<usize>() <= 1024);
        let notification_bytes: usize = cache.pressure_evictions.iter()
            .map(|(domain, names)| names.iter().map(|name| domain.len() + name.len()).sum::<usize>()).sum();
        assert!(notification_bytes <= 4 * 1024 * 1024);
        // Inspect retained record data, not admission counters.
        let payload: usize = maps.iter().flat_map(|m| m.values()).flatten().map(|r| {
            r.record.get_name().len()
                + match r.record.get_type() {
                    RRType::PTR => r.record.any().downcast_ref::<DnsPointer>().unwrap().alias().len(),
                    RRType::SRV => r.record.any().downcast_ref::<DnsSrv>().unwrap().host().len(),
                    RRType::TXT => r.record.any().downcast_ref::<DnsTxt>().unwrap().text().len(),
                    RRType::NSEC => 65535 + 255,
                    _ => 0,
                }
        }).sum();
        assert!(payload <= 4 * 1024 * 1024);
    }

    fn assert_empty(cache: &DnsCache) {
        assert!(maps(cache).iter().all(|m| m.is_empty()));
        assert_eq!(cache.subtype_count(), 0);
        assert!(cache.ptr_owners.is_empty());
        assert!(cache.host_owners.is_empty());
        assert!(cache.admission_order.is_empty());
    }

    #[test]
    fn rejected_unrelated_records_leave_no_owner_or_subtype_keys() {
        let mut cache = DnsCache::new();
        for i in 0..4096 {
            let name = format!("_kind{i}._sub._unrelated._udp.local.");
            let records = [
                DnsPointer::new(&name, RRType::PTR, 1, 120,
                    format!("instance{i}._unrelated._udp.local.")).boxed(),
                DnsSrv::new(&name, 1, 120, 0, 0, 80, "host.local.".into()).boxed(),
                DnsTxt::new(&name, 1, 120, vec![1, b'x']).boxed(),
                DnsAddress::new(&name, RRType::A, 1, 120,
                    IpAddr::V4(Ipv4Addr::LOCALHOST), InterfaceId::default()).boxed(),
                DnsNSec::new(&name, 1, 120, name.clone(), vec![0x40]).boxed(),
            ];
            for record in records {
                assert!(cache.add_or_update(&intf(), record, &mut Vec::new(), false).is_none());
            }
        }
        assert_empty(&cache);
    }

    #[test]
    fn orphan_floods_are_bounded_and_expire_without_ptrs() {
        let mut cache = DnsCache::new();
        for i in 0..4096 {
            let name = format!("orphan{i}._example._udp.local.");
            insert(&mut cache, DnsTxt::new(&name, 1, 1, vec![1, b'x']).boxed());
            insert(&mut cache, DnsSrv::new(&name, 1, 1, 0, 0, 80, "host.local.".into()).boxed());
            insert(&mut cache, DnsNSec::new(&name, 1, 1, name.clone(), vec![0x40]).boxed());
            if i % 128 == 0 {
                assert_bounded(&cache);
            }
        }
        assert_bounded(&cache);
        cache.evict_expired_services(current_time_millis() + 2000);
        assert_empty(&cache);
    }

    #[test]
    fn same_owner_vectors_and_large_txt_payloads_cannot_bypass_global_limits() {
        let mut cache = DnsCache::new();
        for i in 0u32..4096 {
            insert(&mut cache, DnsPointer::new("_example._udp.local.", RRType::PTR, 1, 120,
                format!("instance{i}._example._udp.local.")).boxed());
            insert(&mut cache, DnsTxt::new("shared._example._udp.local.", 1, 120,
                i.to_be_bytes().to_vec()).boxed());
            insert(&mut cache, DnsSrv::new("shared._example._udp.local.", 1, 120,
                0, 0, 80, format!("host{i}.local.")).boxed());
            insert(&mut cache, DnsAddress::new("host.local.", RRType::A, 1, 120,
                IpAddr::V4(Ipv4Addr::from(i)), InterfaceId::default()).boxed());
        }
        assert_bounded(&cache);
        for i in 0u32..256 {
            let mut data = vec![b'x'; 60000];
            data[..4].copy_from_slice(&i.to_be_bytes());
            insert(&mut cache, DnsTxt::new(&format!("large{i}._example._udp.local."), 1, 120, data).boxed());
            assert_bounded(&cache);
        }
        for i in 0..256 {
            insert(&mut cache, DnsNSec::new("shared._example._udp.local.", 1, 120,
                format!("next{i}.local."), vec![0x40]).boxed());
            assert_bounded(&cache);
        }
        cache.evict_expired_services(u64::MAX);
        cache.evict_expired_addr(u64::MAX);
        assert_empty(&cache);
    }

    #[test]
    fn subtype_churn_tracks_only_retained_ptr_records() {
        let mut cache = DnsCache::new();
        for i in 0..4096 {
            let owner = format!("_kind{i}._sub._example._udp.local.");
            insert(&mut cache, DnsPointer::new(&owner, RRType::PTR, 1, 1,
                format!("instance{i}._example._udp.local.")).boxed());
            assert_bounded(&cache);
        }
        let record = cache.ptr.values().next().unwrap()[0].record.clone_box();
        let alias = record.any().downcast_ref::<DnsPointer>().unwrap().alias().to_string();
        assert!(cache.remove(&record));
        assert!(cache.get_subtype(&alias).is_none());
        cache.evict_expired_services(current_time_millis() + 2000);
        assert_empty(&cache);
    }

    #[test]
    fn subtype_mapping_moves_to_a_surviving_advertisement() {
        let mut cache = DnsCache::new();
        let name = "instance._example._udp.local.";
        let first = "_first._sub._example._udp.local.";
        let second = "_second._sub._example._udp.local.";
        let boundary = current_time_millis() + 1000;
        let mut old = DnsPointer::new(first, RRType::PTR, 1, 1, name.into()).boxed();
        old.set_expire(boundary);
        let mut new = DnsPointer::new(second, RRType::PTR, 1, 2, name.into()).boxed();
        new.set_expire(boundary + 1000);
        insert(&mut cache, old);
        insert(&mut cache, new);
        assert_eq!(cache.get_subtype(name).map(String::as_str), Some(first));
        cache.evict_expired_services(boundary);
        assert_eq!(cache.get_subtype(name).map(String::as_str), Some(second));
        cache.evict_expired_services(boundary + 1000);
        assert_empty(&cache);
    }

    #[test]
    fn admission_recovers_after_pressure_and_normal_service_records_resolve() {
        let mut cache = DnsCache::new();
        for i in 0..4096 {
            insert(&mut cache, DnsTxt::new(&format!("hostile{i}._example._udp.local."),
                1, u32::MAX, vec![1, b'x']).boxed());
        }
        let name = "normal._example._udp.local.";
        insert(&mut cache, DnsPointer::new("_example._udp.local.", RRType::PTR, 1, 120, name.into()).boxed());
        insert(&mut cache, DnsSrv::new(name, 1, 120, 0, 0, 1234, "normal.local.".into()).boxed());
        insert(&mut cache, DnsTxt::new(name, 1, 120, vec![3, b'a', b'=', b'b']).boxed());
        insert(&mut cache, DnsAddress::new("normal.local.", RRType::A, 1, 120,
            IpAddr::V4(Ipv4Addr::LOCALHOST), InterfaceId::default()).boxed());
        assert!(cache.get_ptr("_example._udp.local.").unwrap().iter()
            .any(|r| r.record.any().downcast_ref::<DnsPointer>().unwrap().alias() == name));
        assert_eq!(cache.get_srv(name).unwrap()[0].record.any().downcast_ref::<DnsSrv>().unwrap().port(), 1234);
        assert_eq!(cache.get_txt(name).unwrap()[0].record.any().downcast_ref::<DnsTxt>().unwrap().text(),
            &[3, b'a', b'=', b'b']);
        assert_eq!(cache.get_addresses_for_host("normal.local.").values().map(|s| s.len()).sum::<usize>(), 1);
        assert_bounded(&cache);
    }

    #[test]
    fn renewal_at_expiration_boundary_preserves_records_until_the_new_deadline() {
        let mut cache = DnsCache::new();
        let name = "renew._example._udp.local.";
        let boundary = current_time_millis() + 1000;
        let make_records = |ttl| vec![
            DnsPointer::new("_example._udp.local.", RRType::PTR, 1, ttl, name.into()).boxed(),
            DnsSrv::new(name, 1, ttl, 0, 0, 80, "host.local.".into()).boxed(),
            DnsTxt::new(name, 1, ttl, vec![1, b'x']).boxed(),
        ];
        for mut record in make_records(1) {
            record.set_expire(boundary);
            insert(&mut cache, record);
        }
        assert!(cache.evict_expired_services(boundary - 1).is_empty());
        for record in make_records(2) {
            let (_, is_new) = cache.add_or_update(&intf(), record, &mut Vec::new(), true).unwrap();
            assert!(!is_new);
        }
        assert!(cache.evict_expired_services(boundary).is_empty());
        assert_eq!(cache.ptr_count() + cache.srv_count() + cache.txt_count(), 3);
        let renewed_deadline = maps(&cache).iter().flat_map(|m| m.values()).flatten()
            .map(|r| r.record.get_expire()).max().unwrap();
        let removed = cache.evict_expired_services(renewed_deadline);
        assert!(removed["_example._udp.local."].contains(name));
        assert_empty(&cache);
    }

    #[test]
    fn instance_ownership_and_deadlines_follow_live_records_not_packet_history() {
        let mut cache = DnsCache::new();
        let intf = intf();
        let name = "instance._example._udp.local.";
        let record = DnsPointer::new("_example._udp.local.", RRType::PTR, 1, 100, name.into()).boxed();
        let refresh = record.get_record().get_refresh_time();
        let expiry = record.get_expire();
        cache.add_or_update(&intf, record, &mut Vec::new(), true).unwrap();
        assert!(cache.contains_instance(name));
        assert!(!cache.contains_instance("absent._example._udp.local."));
        assert_eq!(cache.instance_names(), HashSet::from([name.to_string()]));
        assert_eq!(cache.next_deadline(refresh - 1), Some(refresh));
        assert_eq!(cache.next_deadline(refresh), Some(expiry));
        assert_eq!(cache.next_deadline(expiry), Some(expiry));
        cache.evict_expired_services(expiry);
        assert!(!cache.contains_instance(name));
        assert!(cache.instance_names().is_empty());
        assert_eq!(cache.next_deadline(expiry), None);

        let expired = DnsTxt::new(name, 1, 0, vec![1, b'x']).boxed();
        cache.add_or_update(&intf, expired, &mut Vec::new(), true).unwrap();
        assert!(!cache.contains_instance(name));
        assert!(cache.instance_names().is_empty());
    }

    #[test]
    fn pressure_invalidation_follows_ptr_srv_txt_and_address_owners() {
        let domain = "_example._udp.local.";
        let name = "visible._example._udp.local.";
        for first in 0..4 {
            let mut cache = DnsCache::new();
            let mut records = vec![
                DnsPointer::new(domain, RRType::PTR, 1, 120, name.into()).boxed(),
                DnsSrv::new(name, 1, 120, 0, 0, 80, "VISIBLE.local.".into()).boxed(),
                DnsTxt::new(name, 1, 120, vec![1, b'x']).boxed(),
                DnsAddress::new("visible.local.", RRType::A, 1, 120,
                    IpAddr::V4(Ipv4Addr::LOCALHOST), InterfaceId::default()).boxed(),
            ];
            records.swap(0, first);
            for record in records {
                insert(&mut cache, record);
            }
            for i in 0..1020 {
                insert(&mut cache, DnsTxt::new(&format!("unrelated{i}._example._udp.local."),
                    1, 120, vec![1, b'x']).boxed());
            }
            assert!(cache.add_or_update(&intf(),
                DnsTxt::new("pressure._example._udp.local.", 1, 120, vec![1, b'x']).boxed(),
                &mut Vec::new(), true).is_some());
            let invalidated = cache.take_pressure_evictions();
            assert_eq!(invalidated.get(domain), Some(&HashSet::from([name.to_string()])));
            if first != 0 {
                assert!(cache.contains_instance(name), "sibling ownership must not suppress invalidation");
            }
            assert!(cache.take_pressure_evictions().is_empty());
            assert_bounded(&cache);
        }
    }

    #[test]
    fn undrained_pressure_notifications_are_bounded_and_backpressure_recovers() {
        let mut cache = DnsCache::new();
        let domain = "_example._udp.local.";
        let mut accepted = 0;
        for i in 0..1200 {
            let record = DnsPointer::new(domain, RRType::PTR, 1, 120,
                format!("instance{i}._example._udp.local.")).boxed();
            if cache.add_or_update(&intf(), record, &mut Vec::new(), true).is_some() {
                accepted += 1;
            }
            assert_bounded(&cache);
        }
        assert_eq!(accepted, 64 + 1024);
        assert_eq!(cache.ptr_count(), 64);
        let invalidated = cache.take_pressure_evictions();
        assert_eq!(invalidated[domain].len(), 1024);
        assert!((0..1024).all(|i| invalidated[domain].contains(&format!("instance{i}._example._udp.local."))));
        let record = DnsPointer::new(domain, RRType::PTR, 1, 120,
            "legitimate._example._udp.local.".into()).boxed();
        assert!(cache.add_or_update(&intf(), record, &mut Vec::new(), true).is_some());
        assert!(cache.contains_instance("legitimate._example._udp.local."));
        assert_eq!(cache.take_pressure_evictions()[domain],
            HashSet::from(["instance1024._example._udp.local.".to_string()]));
    }
}
