// SPDX-License-Identifier: GPL-3.0-or-later
//! Discovery records are multihomed sources, not one arbitrarily sorted IP.
use super::{discovery::Peer, network::{self, RateBudget, ReceiveActivity, Session}};
use futures_util::{future::BoxFuture, stream::FuturesUnordered, StreamExt};
use std::{collections::{HashMap, HashSet, VecDeque}, net::IpAddr, sync::{Arc, LazyLock}, time::Duration};
use tokio_util::sync::CancellationToken;

const MAX_ADDRESSES: usize = 32;
const MAX_SOURCE_ATTEMPTS: usize = 256;
const ADDRESS_STAGGER: Duration = Duration::from_millis(200);
const ADDRESS_DEADLINE: Duration = Duration::from_secs(20);
// A lost link is not a verdict on the peer: one endpoint may be redialed this
// many times per job, with a short doubling backoff, before the job reports the
// loss. The verified prefix is durable, so every attempt resumes from it.
pub(super) const MAX_SOURCE_RECONNECTS: u8 = 4;
const RECONNECT_BACKOFF: Duration = Duration::from_millis(200);
static ADDRESS_SLOTS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(64)));

#[derive(Clone, Debug)]
pub struct ConnectionTarget {
    hosts: Vec<String>,
    port: u16,
    discovered: bool,
    /// Apple-only: prefer AWDL/peer-to-peer before infrastructure.
    direct: bool,
}
impl ConnectionTarget {
    pub fn manual(host: String, port: u16) -> Self { Self { hosts: vec![host], port, discovered: false, direct: false } }
    pub fn with_direct(mut self, direct: bool) -> Self { self.direct = direct; self }
    pub fn direct(&self) -> bool { self.direct }
    pub(super) fn peer(peer: &Peer) -> Result<Self, String> {
        if peer.transport != "quic" || peer.port == 0 { return Err("Unsupported LocalSwarm discovered peer".into()); }
        let mut hosts = Vec::new();
        #[cfg(any(target_os = "ios", target_os = "macos"))]
        if super::apple_transport::service_instance(&peer.host).is_ok() {
            hosts.push(peer.host.clone());
        }
        for address in std::iter::once(&peer.host).chain(peer.addresses.iter()).take(MAX_ADDRESSES + 1) {
            let Ok(ip) = address.parse::<IpAddr>() else { continue; };
            if !network::usable_address(ip) { continue; }
            let address = ip.to_string();
            if !hosts.contains(&address) { hosts.push(address); }
            if hosts.len() == MAX_ADDRESSES { break; }
        }
        if hosts.is_empty() { return Err("The discovered LocalSwarm peer has no usable advertised address".into()); }
        Ok(Self { hosts, port: peer.port, discovered: true, direct: false })
    }
    pub(super) fn port(&self) -> u16 { self.port }
    fn contains(&self, host: &str, port: u16) -> bool { self.port == port && self.hosts.iter().any(|candidate| candidate == host) }
    fn prefer(&mut self, host: &str) {
        if let Some(index) = self.hosts.iter().position(|candidate| candidate == host) { self.hosts.swap(0, index); }
    }
    #[cfg(test)]
    pub(super) fn test_discovered(hosts: Vec<String>, port: u16) -> Self { Self { hosts, port, discovered: true, direct: false } }

    #[cfg(test)]
    pub(super) fn hosts(&self) -> &[String] { &self.hosts }
}
pub(super) fn selected_peer_target(peers: &[Peer], id: &str, sid: &str, port: u16, direct: bool) -> Result<ConnectionTarget, String> {
    if id.is_empty() || id.len() > 255 { return Err("Invalid LocalSwarm discovery selection".into()); }
    let peer = peers.iter().find(|peer| peer.id == id).ok_or("The selected LocalSwarm peer is no longer discovered; choose it again")?;
    if peer.swarm_id != sid || peer.port != port {
        return Err("The selected LocalSwarm share changed; inspect it again".into());
    }
    ConnectionTarget::peer(peer).map(|target| target.with_direct(direct))
}

/// All attempts remain owned until their cancellation/connection drain has
/// finished. In particular, do not timeout/drop a QUIC connect future: it owns
/// a socket even before a Session exists.
pub(super) async fn connect_target(
    target: &ConnectionTarget, sid: &str, pin: Option<&str>, peer_id: &str,
    cancel: &CancellationToken, budget: Arc<RateBudget>, attempted: &mut HashSet<(String, u16)>,
) -> Result<(String, Session), String> {
    if cancel.is_cancelled() { return Err("LocalSwarm connection stopped".into()); }
    if !target.discovered {
        let host = target.hosts.first().ok_or("LocalSwarm connection has no address")?;
        attempted.insert((host.clone(), target.port));
        return network::connect_with_route(host, target.port, sid, pin, peer_id, cancel.child_token(), budget, target.direct)
            .await.map(|session| (host.clone(), session));
    }
    if target.hosts.is_empty() || target.hosts.len() > MAX_ADDRESSES { return Err("Invalid discovered LocalSwarm address count".into()); }
    let tokens: Vec<_> = target.hosts.iter().map(|_| cancel.child_token()).collect();
    let mut attempts: FuturesUnordered<BoxFuture<'_, (usize, Result<Session, String>)>> = FuturesUnordered::new();
    for (index, host) in target.hosts.iter().enumerate() {
        let token = tokens[index].clone(); let budget = budget.clone();
        attempts.push(Box::pin(async move {
            let delay = ADDRESS_STAGGER.saturating_mul(index as u32);
            tokio::select! {
                biased;
                _ = token.cancelled() => return (index, Err("LocalSwarm address attempt stopped".into())),
                _ = tokio::time::sleep(delay) => (),
            }
            let work = async {
                let _slot = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Err("LocalSwarm address attempt stopped".into()),
                    permit = ADDRESS_SLOTS.clone().acquire_owned() => permit.map_err(|error| error.to_string())?,
                };
                network::connect_with_route(host, target.port, sid, pin, peer_id, token.clone(), budget, target.direct).await
            };
            tokio::pin!(work);
            let result = tokio::select! {
                biased;
                result = &mut work => result,
                _ = tokio::time::sleep(ADDRESS_DEADLINE) => {
                    token.cancel();
                    if let Ok(session) = work.await { session.shutdown().await; }
                    Err("LocalSwarm advertised address connection timed out".into())
                }
            };
            (index, result)
        }));
    }
    let mut winner = None;
    let mut last_error = "No advertised LocalSwarm address responded".to_string();
    while let Some((index, result)) = attempts.next().await {
        match result {
            Ok(session) if winner.is_none() && !cancel.is_cancelled() => {
                attempted.insert((target.hosts[index].clone(), target.port));
                winner = Some((target.hosts[index].clone(), session));
                for (other, token) in tokens.iter().enumerate() { if other != index { token.cancel(); } }
            },
            Ok(session) => session.shutdown().await,
            Err(error) if winner.is_none() => {
                attempted.insert((target.hosts[index].clone(), target.port));
                last_error = error;
            },
            Err(_) => (),
        }
    }
    if cancel.is_cancelled() {
        if let Some((_, session)) = winner { session.shutdown().await; }
        return Err("LocalSwarm connection stopped".into());
    }
    winner.ok_or(last_error)
}

pub(super) fn enqueue_discovered_sources(
    sid: &str, peers: impl IntoIterator<Item = Peer>, attempted: &HashSet<(String, u16)>,
    reserved: &HashSet<(String, u16)>, queued: &mut VecDeque<ConnectionTarget>, direct: bool,
) {
    // Membership is one hash lookup per advertised host: `attempted` and
    // `reserved` are already sets, and the queued list is indexed once so a
    // discovery refresh never scans it per host.
    let mut queued_keys: HashSet<(String, u16)> = queued.iter()
        .flat_map(|target| target.hosts.iter().map(move |host| (host.clone(), target.port)))
        .collect();
    let mut added = Vec::new();
    for peer in peers {
        if peer.swarm_id != sid { continue; }
        let Ok(mut target) = ConnectionTarget::peer(&peer) else { continue; };
        target.direct = direct;
        target.hosts.retain(|host| {
            let key = (host.clone(), target.port);
            !attempted.contains(&key) && !reserved.contains(&key) && queued_keys.insert(key)
        });
        if !target.hosts.is_empty() { added.push(target); }
    }
    queued.extend(added);
}

pub(super) fn source_candidates(host: String, port: u16, sid: &str, peers: Vec<Peer>, direct: bool) -> VecDeque<ConnectionTarget> {
    let mut queued = VecDeque::new();
    enqueue_discovered_sources(sid, peers, &HashSet::new(), &HashSet::new(), &mut queued, direct);
    let primary = if let Some(index) = queued.iter().position(|target| target.contains(&host, port)) {
        let mut target = queued.remove(index).unwrap(); target.prefer(&host); target
    } else { ConnectionTarget::manual(host, port).with_direct(direct) };
    queued.push_front(primary);
    queued
}

struct Pending { cancel: CancellationToken, endpoints: Vec<(String, u16)> }
struct Completion { id: usize, attempted: HashSet<(String, u16)>, result: Result<(String, Session), String> }
/// A reconnect may continue only while the failure is about the link. Policy,
/// cancellation, admission and validation verdicts are decisions, not link
/// state, and must surface their own error immediately.
fn reconnect_allowed(result: &Result<(String, Session), String>) -> bool {
    const TERMINAL: [&str; 6] = [
        "policy interrupted", "cancelled", "stopped", "not accepting", "admission closed", "Invalid LocalSwarm",
    ];
    match result {
        Ok(_) => false,
        Err(error) => !TERMINAL.iter().any(|marker| error.contains(marker)),
    }
}
/// One endpoint whose live session was lost, waiting out its reconnect backoff.
struct ScheduledReconnect { at: tokio::time::Instant, host: String, port: u16 }
/// Pending optional sources are polled alongside payload, never serially in
/// front of chunks from a working source. Only four source sessions/dials may
/// occupy a job, independent of how many addresses each peer advertises.
pub(super) struct SourceDialer {
    queued: VecDeque<ConnectionTarget>, attempted: HashSet<(String, u16)>, reserved: HashSet<(String, u16)>,
    pending: HashMap<usize, Pending>, futures: FuturesUnordered<BoxFuture<'static, Completion>>,
    next_id: usize, address_attempts: usize, sid: String, pin: Option<String>, peer_id: String,
    cancel: CancellationToken, budget: Arc<RateBudget>, last_error: String,
    activity: Arc<ReceiveActivity>, direct: bool,
    /// Reconnects already spent per endpoint, for the whole job.
    reconnects: HashMap<(String, u16), u8>,
    scheduled: Vec<ScheduledReconnect>,
}
impl SourceDialer {
    pub fn new(queued: VecDeque<ConnectionTarget>, sid: String, pin: Option<String>, peer_id: String, cancel: CancellationToken, budget: Arc<RateBudget>, activity: Arc<ReceiveActivity>) -> Self {
        Self { queued, attempted: HashSet::new(), reserved: HashSet::new(), pending: HashMap::new(), futures: FuturesUnordered::new(), next_id: 0, address_attempts: 0, sid, pin, peer_id, cancel, budget, activity, direct: false, reconnects: HashMap::new(), scheduled: Vec::new(), last_error: "No LocalSwarm peer has the missing chunks".into() }
    }
    pub fn with_direct(mut self, direct: bool) -> Self { self.direct = direct; self }
    pub fn add_discovered(&mut self, peers: Vec<Peer>) -> bool {
        if !self.activity.accepting() || self.address_attempts >= MAX_SOURCE_ATTEMPTS { return false; }
        enqueue_discovered_sources(&self.sid, peers, &self.attempted, &self.reserved, &mut self.queued, self.direct);
        !self.queued.is_empty()
    }
    pub fn fill(&mut self, active: usize) {
        self.promote_reconnects();
        while active + self.pending.len() < 4 {
            if self.queued.is_empty() { break; }
            let Some(dial_cancel) = self.activity.dial_token() else {
                self.last_error = "LocalSwarm is not accepting new receive sessions".into();
                break;
            };
            let Some(mut target) = self.queued.pop_front() else { break; };
            target.hosts.retain(|host| {
                let key = (host.clone(), target.port);
                !self.attempted.contains(&key) && !self.reserved.contains(&key)
            });
            target.hosts.truncate(MAX_SOURCE_ATTEMPTS.saturating_sub(self.address_attempts));
            if target.hosts.is_empty() {
                if self.address_attempts == MAX_SOURCE_ATTEMPTS {
                    self.queued.clear();
                    self.last_error = "LocalSwarm source replacement limit reached; retained verified data can be resumed".into();
                    break;
                }
                continue;
            }
            self.address_attempts += target.hosts.len();
            let id = self.next_id; self.next_id += 1;
            let token = self.cancel.child_token();
            let endpoints: Vec<_> = target.hosts.iter().map(|host| (host.clone(), target.port)).collect();
            self.reserved.extend(endpoints.iter().cloned());
            self.pending.insert(id, Pending { cancel: token.clone(), endpoints });
            let sid = self.sid.clone(); let pin = self.pin.clone(); let peer_id = self.peer_id.clone(); let budget = self.budget.clone();
            let activity = self.activity.clone();
            self.futures.push(Box::pin(async move {
                let mut attempted = HashSet::new();
                let result = {
                    let work = connect_target(&target, &sid, pin.as_deref(), &peer_id, &token, budget, &mut attempted);
                    tokio::pin!(work);
                    tokio::select! {
                        biased;
                        _ = dial_cancel.cancelled() => {
                            // Retain and drain every owned pre-Session socket;
                            // never propagate this dial token to an active peer.
                            token.cancel();
                            if let Ok((_, session)) = work.await { session.shutdown().await; }
                            Err("LocalSwarm receive admission closed during connection".into())
                        },
                        result = &mut work => result,
                    }
                };
                let result = match result {
                    Ok((host, mut session)) => match session.track_receive(&activity) {
                        Ok(()) => Ok((host, session)),
                        Err(error) => { session.shutdown().await; Err(error) },
                    },
                    Err(error) => Err(error),
                };
                Completion { id, attempted, result }
            }));
        }
    }
    /// A live session was lost. `error` is the failure that killed it, which the
    /// caller has already classified as a transport loss: retire only this
    /// connection and schedule a bounded, backed-off redial of the same endpoint
    /// through the normal dial path, which revalidates the local link and repeats
    /// the handshake, manifest and bitfield before any chunk request.
    pub fn retire(&mut self, session: &Session, error: &str) {
        let Some((host, port)) = session.retry_target() else { return; };
        self.schedule_reconnect(host.to_string(), port, error);
    }
    fn schedule_reconnect(&mut self, host: String, port: u16, error: &str) {
        let spent = self.reconnects.entry((host.clone(), port)).or_insert(0);
        if *spent >= MAX_SOURCE_RECONNECTS {
            self.last_error = format!("LocalSwarm source reconnect limit reached after {MAX_SOURCE_RECONNECTS} attempts; retained verified data can be resumed (last error: {error})");
            return;
        }
        let delay = RECONNECT_BACKOFF.saturating_mul(1u32 << *spent);
        *spent += 1;
        self.scheduled.push(ScheduledReconnect { at: tokio::time::Instant::now() + delay, host, port });
    }
    /// Move reconnects whose backoff has elapsed back into the dial queue and
    /// report the wait until the next one is due. A promoted endpoint leaves the
    /// retired set, so the usual `attempted`/`reserved` filtering lets it dial
    /// again; closing admission drops the schedule instead, because that pauses
    /// a job rather than failing it.
    fn promote_reconnects(&mut self) -> Option<Duration> {
        if self.scheduled.is_empty() { return None; }
        if !self.activity.accepting() { self.scheduled.clear(); return None; }
        let now = tokio::time::Instant::now();
        let mut next: Option<Duration> = None;
        let mut due = Vec::new();
        self.scheduled.retain(|reconnect| {
            if reconnect.at <= now { due.push((reconnect.host.clone(), reconnect.port)); false }
            else {
                let wait = reconnect.at - now;
                next = Some(next.map_or(wait, |known: Duration| known.min(wait)));
                true
            }
        });
        for (host, port) in due {
            self.attempted.remove(&(host.clone(), port));
            self.queued.push_back(ConnectionTarget::manual(host, port).with_direct(self.direct));
        }
        next
    }
    pub fn has_pending(&self) -> bool { !self.futures.is_empty() || !self.scheduled.is_empty() }
    /// A dial future exists right now; scheduled-but-not-due reconnects are not
    /// dials, so the receiver must wait for their deadline instead of polling.
    pub fn has_dials(&self) -> bool { !self.futures.is_empty() }
    pub fn has_scheduled(&self) -> bool { !self.scheduled.is_empty() }
    pub fn scheduled_wait(&self) -> Option<Duration> {
        self.scheduled.iter().map(|reconnect| reconnect.at.saturating_duration_since(tokio::time::Instant::now())).min()
    }
    pub fn error(&self) -> String { self.last_error.clone() }
    pub fn fail(&mut self, error: String) { self.last_error = error; }
    pub async fn next(&mut self) -> Option<Session> {
        if self.futures.is_empty() {
            // Only a backed-off reconnect can be outstanding. Wait it out, then
            // report "nothing yet" so the caller's next `fill` starts the dial
            // under the same four-dial cap as any other source.
            if let Some(wait) = self.promote_reconnects() {
                tokio::select! {
                    _ = self.cancel.cancelled() => { self.scheduled.clear(); return None; }
                    _ = tokio::time::sleep(wait) => {}
                }
                self.promote_reconnects();
            }
            return None;
        }
        let completion = self.futures.next().await?;
        if let Some(pending) = self.pending.remove(&completion.id) {
            for endpoint in pending.endpoints { self.reserved.remove(&endpoint); }
        }
        // An endpoint that is already reconnecting gets its next bounded attempt
        // even when the redial itself failed while the link was still down. A
        // policy, cancellation, admission or validation verdict is not a link
        // state: it stays terminal with its own error, immediately.
        let reconnecting: Vec<(String, u16)> = if reconnect_allowed(&completion.result) {
            completion.attempted.iter().filter(|endpoint| self.reconnects.contains_key(*endpoint)).cloned().collect()
        } else { Vec::new() };
        self.attempted.extend(completion.attempted);
        match completion.result {
            Ok((_, session)) => Some(session),
            Err(error) => {
                self.last_error = error.clone();
                for (host, port) in reconnecting { self.schedule_reconnect(host, port, &error); }
                None
            },
        }
    }
    pub async fn stop(&mut self) {
        self.scheduled.clear();
        for pending in self.pending.values() { pending.cancel.cancel(); }
        while self.has_pending() { if let Some(session) = self.next().await { session.shutdown().await; } }
    }
}
