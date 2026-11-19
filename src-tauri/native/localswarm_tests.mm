// SPDX-License-Identifier: GPL-3.0-or-later
// Exercise the real bridge while the stack retains content beyond its callback.
#import <Foundation/Foundation.h>
#import <Network/Network.h>
#include <dns_sd.h>
#include <ifaddrs.h>
#include <net/if.h>
#include <arpa/inet.h>
#include <algorithm>
#include <cstdio>
#include <vector>
#include <thread>
#include <atomic>

static dispatch_data_t retained_send;
static void capture_send(nw_connection_t, dispatch_data_t content, nw_content_context_t,
                         bool, nw_connection_send_completion_t completed) {
    retained_send = content;
    completed(nullptr);
}

static nw_connection_receive_completion_t pending_receive;
static unsigned receive_calls;
static void capture_receive(nw_connection_t, uint32_t, uint32_t, nw_connection_receive_completion_t callback) {
    ++receive_calls;
    pending_receive = callback;
}

// Reuse the same synthetic SDK reference for successive lookups. The
// callback cookie, not a reference address, must distinguish their lifetimes.
static char queryStorage;
static DNSServiceRef test_ref() { return reinterpret_cast<DNSServiceRef>(&queryStorage); }
static bool failQueueAssignment;
static unsigned releasedQueries;
static void *addressCookie;
static DNSServiceErrorType assign_queue(DNSServiceRef ref, dispatch_queue_t target) {
    if (ref != test_ref()) return DNSServiceSetDispatchQueue(ref, target);
    return failQueueAssignment ? kDNSServiceErr_BadState : kDNSServiceErr_NoError;
}
static void release_ref(DNSServiceRef ref) {
    if (ref == test_ref()) ++releasedQueries;
    else DNSServiceRefDeallocate(ref);
}
static DNSServiceErrorType lookup_address(DNSServiceRef *out, DNSServiceFlags flags, uint32_t index,
                                           DNSServiceProtocol protocol, const char *host,
                                           DNSServiceGetAddrInfoReply reply, void *cookie) {
    if (strcmp(host, "route-test.local.") != 0)
        return DNSServiceGetAddrInfo(out, flags, index, protocol, host, reply, cookie);
    addressCookie = cookie;
    *out = test_ref();
    return kDNSServiceErr_NoError;
}
// A two-interface path for the synchronous selected_path check. Only this
// thread substitutes the path, its interface identities and its address list.
struct RouteFixture {
    nw_path_t path = (nw_path_t)[NSObject new];
    nw_interface_t bound = (nw_interface_t)[NSObject new];
    nw_interface_t other = (nw_interface_t)[NSObject new];
    nw_endpoint_t peer;
    nw_endpoint_t local;
    ifaddrs records[2] {};
    unsigned liveBoundIndex = 7;
    bool reverse = false;
};
static thread_local RouteFixture *routeFixture;
static const char *test_interface_name(nw_interface_t interface) {
    if (routeFixture && interface == routeFixture->bound) return "test-a";
    if (routeFixture && interface == routeFixture->other) return "test-b";
    return nw_interface_get_name(interface);
}
static unsigned test_name_to_index(const char *name) {
    if (routeFixture && !strcmp(name, "test-a")) return routeFixture->liveBoundIndex;
    if (routeFixture && !strcmp(name, "test-b")) return 8;
    return if_nametoindex(name);
}
static void test_enumerate(nw_path_t path, nw_path_enumerate_interfaces_block_t visit) {
    if (!routeFixture || path != routeFixture->path) {
        nw_path_enumerate_interfaces(path, visit);
        return;
    }
    if (visit(routeFixture->reverse ? routeFixture->other : routeFixture->bound)) {
        visit(routeFixture->reverse ? routeFixture->bound : routeFixture->other);
    }
}
static int test_getifaddrs(ifaddrs **out) {
    if (!routeFixture) return getifaddrs(out);
    *out = routeFixture->records;
    return 0;
}
static void test_freeifaddrs(ifaddrs *records) {
    if (!routeFixture || records != routeFixture->records) freeifaddrs(records);
}
static uint32_t test_interface_index(nw_interface_t interface) {
    if (routeFixture && interface == routeFixture->bound) return 7;
    if (routeFixture && interface == routeFixture->other) return 8;
    return interface ? nw_interface_get_index(interface) : 7;
}

#define nw_path_get_status(p) ((routeFixture && (p) == routeFixture->path) ? nw_path_status_satisfied : nw_path_get_status(p))
#define nw_path_uses_interface_type(p, t) ((routeFixture && (p) == routeFixture->path) ? false : nw_path_uses_interface_type(p, t))
#define nw_path_copy_effective_local_endpoint(p) ((routeFixture && (p) == routeFixture->path) ? routeFixture->local : nw_path_copy_effective_local_endpoint(p))
#define nw_path_copy_effective_remote_endpoint(p) ((routeFixture && (p) == routeFixture->path) ? routeFixture->peer : nw_path_copy_effective_remote_endpoint(p))
#define nw_interface_get_type(i) ((routeFixture && ((i) == routeFixture->bound || (i) == routeFixture->other)) ? nw_interface_type_wifi : nw_interface_get_type(i))
#define nw_path_enumerate_interfaces test_enumerate
#define nw_interface_get_name test_interface_name
#define if_nametoindex test_name_to_index
#define getifaddrs test_getifaddrs
#define freeifaddrs test_freeifaddrs
#define DNSServiceSetDispatchQueue assign_queue
#define DNSServiceRefDeallocate release_ref
#define DNSServiceGetAddrInfo lookup_address
#define nw_interface_get_index test_interface_index
#define nw_connection_receive capture_receive
#define nw_connection_send capture_send
#ifndef LOCALSWARM_BRIDGE_SOURCE
#define LOCALSWARM_BRIDGE_SOURCE "localswarm.mm"
#endif
#include LOCALSWARM_BRIDGE_SOURCE
#undef nw_path_get_status
#undef nw_path_uses_interface_type
#undef nw_path_copy_effective_local_endpoint
#undef nw_path_copy_effective_remote_endpoint
#undef nw_interface_get_type
#undef nw_path_enumerate_interfaces
#undef nw_interface_get_name
#undef if_nametoindex
#undef getifaddrs
#undef freeifaddrs
#undef DNSServiceSetDispatchQueue
#undef DNSServiceRefDeallocate
#undef DNSServiceGetAddrInfo
#undef nw_interface_get_index
#undef nw_connection_send
#undef nw_connection_receive

struct WriteResult {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    int status = -1;
    size_t length = 0;
    unsigned callbacks = 0;
};
static bool retained_content_is_owned(size_t length, bool release_before_callback) {
    std::vector<uint8_t> input(length);
    for (size_t i = 0; i < length; ++i) input[i] = (uint8_t)(i * 31 + i / 251);
    const auto expected = input;
    auto handle = std::make_shared<Connection>(nil);
    handle->ready = true;
    const auto id = connections.insert(handle);
    auto result = std::make_shared<WriteResult>();
    @autoreleasepool {
        auto data = dispatch_data_create(input.data(), length, nullptr, DISPATCH_DATA_DESTRUCTOR_DEFAULT);
        kiwix_swarm_write(id, data, ^(int status, size_t bytes) {
            result->status = status;
            result->length = bytes;
            ++result->callbacks;
            dispatch_semaphore_signal(result->done);
        });
    }
    if (release_before_callback) std::fill(input.begin(), input.end(), 0xa5);
    if (dispatch_semaphore_wait(result->done, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC))) return false;
    if (!release_before_callback) std::fill(input.begin(), input.end(), 0x5a);
    __block bool matches = false;
    dispatch_sync(queue(), ^{
        const void *bytes = nullptr;
        size_t size = 0;
        __attribute__((objc_precise_lifetime)) dispatch_data_t mapped =
            dispatch_data_create_map(retained_send, &bytes, &size);
        matches = size == expected.size() && std::equal(expected.begin(), expected.end(), (const uint8_t *)bytes);
        retained_send = nil;
    });
    kiwix_swarm_release(id);
    return matches && result->status == 0 && result->length == length && result->callbacks == 1;
}

struct Observation {
    uint64_t id;
    int status;
    size_t bytes;
    bool operator==(const Observation &other) const {
        return id == other.id && status == other.status && bytes == other.bytes;
    }
};
static std::vector<Observation> received;
static void read_completed(uint64_t id, int status, dispatch_data_t data) {
    received.push_back({id, status, data ? dispatch_data_get_size(data) : 0});
}

// Each synchronous block runs after read has registered on the same serial
// queue. The mock retains the callback so the route can change before delivery.
static bool finish_and_revoke() {
    auto handle = std::make_shared<Connection>(nil);
    handle->ready = true;
    auto stream = connections.insert(handle);
    auto nonfinal = nw_content_context_create("part");
    auto data = dispatch_data_create("end", 3, nullptr, DISPATCH_DATA_DESTRUCTOR_DEFAULT);
    auto deliver = [&](uint64_t id, dispatch_data_t bytes, nw_content_context_t context, bool complete, bool revoke) {
        kiwix_swarm_read(stream, 16, id, read_completed);
        dispatch_sync(queue(), ^{
            if (revoke) handle->ready = false;
            auto callback = pending_receive;
            pending_receive = nil;
            if (callback) callback(bytes, context, complete, nil);
        });
    };
    deliver(1, nil, nonfinal, true, false);
    deliver(2, data, NW_CONNECTION_FINAL_MESSAGE_CONTEXT, false, false);
    deliver(3, data, NW_CONNECTION_FINAL_MESSAGE_CONTEXT, true, false);
    kiwix_swarm_read(stream, 16, 4, read_completed);
    dispatch_sync(queue(), ^{});
    auto second = std::make_shared<Connection>(nil);
    second->ready = true;
    kiwix_swarm_release(stream);
    handle = second;
    stream = connections.insert(second);
    deliver(5, data, NW_CONNECTION_FINAL_MESSAGE_CONTEXT, true, true);
    const std::vector<Observation> expected{{1, 1, 0}, {2, 0, 3}, {3, 0, 3}, {4, 0, 0}, {5, -1, 0}};
    kiwix_swarm_release(stream);
    return received == expected && receive_calls == 4 && !pending_receive && !second->readEnded;
}

// A queued operation owns the selected connection even when its ID is retired.
// Replayed and wrong-kind IDs must fail without acting on a replacement.
static bool retired_ids_do_not_alias() {
    auto first = std::make_shared<Connection>(nil);
    first->connectionStopped = true;
    first->ready = true;
    const auto id = connections.insert(first);
    auto listener = std::make_shared<Listener>(nil, nil, nil, false);
    listener->listenerStopped = true;
    const auto listener_id = listeners.insert(listener);
    auto data = dispatch_data_create("x", 1, nullptr, DISPATCH_DATA_DESTRUCTOR_DEFAULT);
    __block int before = -1;
    kiwix_swarm_write(id, data, ^(int status, size_t) { before = status; });
    std::thread joiner([=] { kiwix_swarm_join(id); });
    kiwix_swarm_release(id);
    kiwix_swarm_release(id);
    joiner.join();
    dispatch_sync(queue(), ^{});
    if (before != 0 || !first->cancelled || connections.get(id)) return false;
    auto next = std::make_shared<Connection>(nil);
    next->connectionStopped = true;
    const auto next_id = connections.insert(next);
    kiwix_swarm_release(listener_id);
    kiwix_swarm_listener_release(next_id);
    kiwix_swarm_listener_join(next_id);
    kiwix_swarm_join(listener_id);
    bool ok = next_id != id && listeners.get(listener_id) == listener && connections.get(next_id) == next;
    for (auto invalid : {uint64_t(0), id, listener_id}) {
        __block unsigned errors = 0;
        kiwix_swarm_start(invalid, ^(int status, size_t) { if (status < 0) ++errors; });
        kiwix_swarm_write(invalid, data, ^(int status, size_t) { if (status < 0) ++errors; });
        kiwix_swarm_read(invalid, 1, invalid, read_completed);
        dispatch_sync(queue(), ^{});
        ok &= errors == 2 && received.back() == Observation{invalid, -1, 0} &&
              kiwix_swarm_interface_index(invalid) == 0;
    }
    __block unsigned refused = 0;
    kiwix_swarm_cancel(next_id);
    kiwix_swarm_start(next_id, ^(int status, size_t) { if (status < 0) ++refused; });
    dispatch_sync(queue(), ^{});
    ok &= refused == 1 && !next->browser;
    kiwix_swarm_listener_release(listener_id);
    kiwix_swarm_release(next_id);
    dispatch_sync(queue(), ^{ retained_send = nil; });
    return ok && next->cancelled && listener->cancelled;
}

static bool concurrent_leases_and_exhaustion() {
    HandleTable<unsigned> table(1);
    std::atomic<bool> ok(true);
    std::vector<std::thread> workers;
    for (unsigned worker = 0; worker < 8; ++worker) {
        workers.emplace_back([&, worker] {
            for (unsigned i = 0; i < 256; ++i) {
                auto value = std::make_shared<unsigned>(i + worker);
                const auto id = table.insert(value);
                auto pinned = table.get(id);
                if (!id || pinned != value || table.take(id) != value || table.get(id) ||
                    table.take(id) || *pinned != i + worker) ok = false;
            }
        });
    }
    for (auto& worker : workers) worker.join();
    for (auto last : {UINT64_MAX, UINT64_MAX - 1}) {
        HandleTable<unsigned> end(last - 2);
        auto value = std::make_shared<unsigned>(1);
        if (end.insert(value) != last - 2 || end.insert(value) != last ||
            end.take(last) != value || end.insert(value) || end.get(last)) return false;
    }
    return ok;
}

static bool selected_route_stays_on_bound_subnet() {
    RouteFixture fixture;
    sockaddr_in ips[4] {};
    const char *values[] = {"192.0.2.1", "198.51.100.1", "255.255.255.0", "198.51.100.2"};
    for (unsigned i = 0; i < 4; ++i) {
        ips[i].sin_len = sizeof(sockaddr_in);
        ips[i].sin_family = AF_INET;
        if (inet_pton(AF_INET, values[i], &ips[i].sin_addr) != 1) return false;
    }
    fixture.peer = nw_endpoint_create_address((sockaddr *)&ips[3]);
    if (!fixture.peer) return false;
    char names[][7] = {"test-a", "test-b"};
    for (unsigned i = 0; i < 2; ++i) {
        auto &record = fixture.records[i];
        record.ifa_name = names[i];
        record.ifa_flags = IFF_UP;
        record.ifa_addr = (sockaddr *)&ips[i];
        record.ifa_netmask = (sockaddr *)&ips[2];
    }
    fixture.records[0].ifa_next = &fixture.records[1];
    routeFixture = &fixture;
    auto handle = std::make_shared<Connection>(nil);
    bool ok = true;
    for (bool reverse : {false, true}) {
        fixture.reverse = reverse;
        // An inbound path may use B. An outbound path bound to A may
        // never claim B's subnet, even before Rust requests an index.
        handle->interface = nil;
        handle->requiredIndex = 0;
        ok &= selected_path(handle, fixture.path);
        handle->interface = fixture.bound;
        ok &= !selected_path(handle, fixture.path);
        handle->requiredIndex = 7;
        ok &= !selected_path(handle, fixture.path);
        fixture.records[0].ifa_addr = (sockaddr *)&ips[1];
        ok &= selected_path(handle, fixture.path);
        fixture.liveBoundIndex = 9;
        ok &= !selected_path(handle, fixture.path);
        fixture.liveBoundIndex = 7;
        handle->requiredIndex = 8;
        ok &= !selected_path(handle, fixture.path);
        handle->interface = nil;
        ok &= !selected_path(handle, fixture.path);
        fixture.records[0].ifa_addr = (sockaddr *)&ips[0];
    }
    routeFixture = nullptr;
    return ok;
}

static bool route_receipt_uses_effective_endpoints() {
    RouteFixture fixture;
    sockaddr_in ips[4] {};
    const char *values[] = {"192.0.2.1", "192.0.2.19", "255.255.255.0", "192.0.2.42"};
    for (unsigned i = 0; i < 4; ++i) {
        ips[i].sin_len = sizeof(sockaddr_in);
        ips[i].sin_family = AF_INET;
        if (inet_pton(AF_INET, values[i], &ips[i].sin_addr) != 1) return false;
    }
    char name[] = "test-a";
    for (unsigned i = 0; i < 2; ++i) {
        auto &record = fixture.records[i];
        record.ifa_name = name;
        record.ifa_flags = IFF_UP;
        record.ifa_addr = (sockaddr *)&ips[i];
        record.ifa_netmask = (sockaddr *)&ips[2];
    }
    fixture.records[0].ifa_next = &fixture.records[1];
    fixture.peer = nw_endpoint_create_address((sockaddr *)&ips[3]);
    fixture.local = nw_endpoint_create_address((sockaddr *)&ips[1]);
    routeFixture = &fixture;
    const auto receipt = [&] { return route_text(fixture.path, fixture.bound, 7); };
    bool ok = receipt() == "test-a (infrastructure) 192.0.2.19 -> 192.0.2.42";
    // Put the alias on another interface with the same reachable subnet.
    // Unbound inbound routes must select B; an explicit binding to A fails.
    char otherName[] = "test-b";
    fixture.records[1].ifa_name = otherName;
    ok &= receipt().empty();
    for (bool reverse : {true, false}) {
        fixture.reverse = reverse;
        ok &= route_text(fixture.path, nil, 0) == "test-b (infrastructure) 192.0.2.19 -> 192.0.2.42";
    }
    // When both live interfaces own the effective IP and are on-link, there
    // is no unique unbound receipt. Ordinary inbound readiness still works.
    fixture.records[0].ifa_addr = (sockaddr *)&ips[1];
    auto incoming = std::make_shared<Connection>(nil);
    for (bool reverse : {true, false}) {
        fixture.reverse = reverse;
        ok &= route_text(fixture.path, nil, 0).empty() && selected_path(incoming, fixture.path);
        ok &= receipt() == "test-a (infrastructure) 192.0.2.19 -> 192.0.2.42";
        fixture.liveBoundIndex = 9;
        ok &= route_text(fixture.path, nil, 0) == "test-b (infrastructure) 192.0.2.19 -> 192.0.2.42";
        ok &= receipt().empty();
        fixture.liveBoundIndex = 7;
    }
    fixture.records[0].ifa_addr = (sockaddr *)&ips[0];
    fixture.records[1].ifa_name = name;
    fixture.local = nil;
    ok &= receipt().empty();
    fixture.local = nw_endpoint_create_host("localhost", "443");
    ok &= receipt().empty();
    fixture.local = nw_endpoint_create_address((sockaddr *)&ips[1]);
    fixture.peer = nil;
    ok &= receipt().empty();
    routeFixture = nullptr;
    return ok;
}

static bool compact_route_masks() {
    sockaddr_in local4 {}, peer4 {};
    local4.sin_len = peer4.sin_len = sizeof(local4);
    local4.sin_family = peer4.sin_family = AF_INET;
    local4.sin_addr.s_addr = htonl(0xc0000201);
    peer4.sin_addr.s_addr = htonl(0xc00002fe);
    // A /24 occupies seven bytes in the Darwin routing ABI.
    std::array<uint8_t, offsetof(sockaddr_in, sin_addr) + 3> mask4 {};
    mask4[0] = mask4.size();
    mask4[1] = AF_INET;
    std::fill(mask4.end() - 3, mask4.end(), 0xff);
    auto check4 = [&] {
        return on_link_address(copy_socket_address((const sockaddr *)&peer4),
            copy_socket_address((const sockaddr *)&local4), (const sockaddr *)mask4.data(), 7);
    };
    if (!check4()) return false;
    peer4.sin_addr.s_addr = htonl(0xc0000301);
    if (check4()) return false;
    mask4[0] -= 1;  // /16 now. The byte past the declared length stays 0xff.
    if (!check4()) return false;
    mask4[1] = AF_UNSPEC;
    if (!check4()) return false;
    mask4[1] = AF_INET6;
    if (check4()) return false;
    mask4[1] = AF_INET;
    mask4[0] = offsetof(sockaddr_in, sin_addr);
    if (check4()) return false;
    mask4[0] = mask4.size();
    std::fill(mask4.end() - 3, mask4.end(), 0);
    if (check4()) return false;

    sockaddr_in6 local6 {}, peer6 {};
    local6.sin6_len = peer6.sin6_len = sizeof(local6);
    local6.sin6_family = peer6.sin6_family = AF_INET6;
    if (inet_pton(AF_INET6, "2001:db8:42:3::1", &local6.sin6_addr) != 1 ||
        inet_pton(AF_INET6, "2001:db8:42:3::2", &peer6.sin6_addr) != 1) return false;
    std::array<uint8_t, offsetof(sockaddr_in6, sin6_addr) + 8> mask6 {};
    mask6[0] = mask6.size();
    mask6[1] = AF_INET6;
    std::fill(mask6.end() - 8, mask6.end(), 0xff);
    auto check6 = [&] {
        return on_link_address(copy_socket_address((const sockaddr *)&peer6),
            copy_socket_address((const sockaddr *)&local6), (const sockaddr *)mask6.data(), 7);
    };
    if (!check6()) return false;
    peer6.sin6_len = offsetof(sockaddr_in6, sin6_addr) + sizeof(in6_addr) - 1;
    if (check6()) return false;
    peer6.sin6_len = sizeof(peer6);
    peer6.sin6_addr.s6_addr[7] ^= 1;
    if (check6()) return false;
    peer6.sin6_addr = local6.sin6_addr;
    peer6.sin6_addr.s6_addr[3] ^= 1; // Global prefixes keep their second word.
    if (check6()) return false;

    if (inet_pton(AF_INET6, "fe80::1", &local6.sin6_addr) != 1 ||
        inet_pton(AF_INET6, "fe80::2", &peer6.sin6_addr) != 1) return false;
    local6.sin6_addr.s6_addr[3] = 7;
    peer6.sin6_scope_id = 7;
    if (!check6()) return false;
    std::swap(local6, peer6);
    if (!check6()) return false;
    local6.sin6_scope_id = 8;
    if (check6()) return false;
    local6.sin6_scope_id = 7;
    peer6.sin6_addr.s6_addr[3] = 8;
    if (check6()) return false;
    peer6.sin6_scope_id = 7; // A matching explicit zone cannot excuse an embedded mismatch.
    if (check6()) return false;
    for (const auto *denied : {"::", "::1", "ff02::1", "::ffff:192.0.2.1"}) {
        if (inet_pton(AF_INET6, denied, &peer6.sin6_addr) != 1) return false;
        local6.sin6_addr = peer6.sin6_addr;
        if (check6()) return false;
    }
    return true;
}

static bool endpoint_addresses_outlive_their_sources() {
    SocketAddress copied4, copied6;
    @autoreleasepool {
        sockaddr_in ip4 {};
        ip4.sin_len = sizeof(ip4);
        ip4.sin_family = AF_INET;
        ip4.sin_port = htons(54321);
        if (inet_pton(AF_INET, "192.0.2.19", &ip4.sin_addr) != 1) return false;
        copied4 = copy_endpoint_address(nw_endpoint_create_address((const sockaddr *)&ip4));

        sockaddr_in6 ip6 {};
        ip6.sin6_len = sizeof(ip6);
        ip6.sin6_family = AF_INET6;
        ip6.sin6_port = htons(12345);
        ip6.sin6_scope_id = if_nametoindex("lo0");
        if (!ip6.sin6_scope_id || inet_pton(AF_INET6, "fe80::42", &ip6.sin6_addr) != 1) return false;
        copied6 = copy_endpoint_address(nw_endpoint_create_address((const sockaddr *)&ip6));
        if (copy_endpoint_address(nil).family() != AF_UNSPEC ||
            copy_endpoint_address(nw_endpoint_create_host("localhost", "443")).family() != AF_UNSPEC) return false;
    }
    std::array<uint8_t, sizeof(in6_addr)> bytes {};
    if (!copied4.v4() || numeric_address(copied4) != "192.0.2.19" ||
        copied4.v4()->sin_port != htons(54321) || copy_ip_identity(copied4, bytes.data()) != 4 ||
        memcmp(bytes.data(), &copied4.v4()->sin_addr, 4)) return false;
    in6_addr expected {};
    if (inet_pton(AF_INET6, "fe80::42", &expected) != 1 || !copied6.v6() ||
        copied6.v6()->sin6_port != htons(12345) || copied6.v6()->sin6_scope_id != if_nametoindex("lo0") ||
        copy_ip_identity(copied6, bytes.data()) != bytes.size() ||
        memcmp(bytes.data(), &expected, bytes.size())) return false;

    // Retain the source bytes, but display/compare the IP without Darwin's
    // embedded zone. The caller checks the interface identity separately.
    auto embedded = *copied6.v6();
    embedded.sin6_addr.s6_addr[3] = 7;
    embedded.sin6_scope_id = 7;
    const auto scoped = copy_socket_address((const sockaddr *)&embedded);
    if (!scope_matches_interface(scoped, 7) || scope_matches_interface(scoped, 8) ||
        !scope_matches_interface(copied4, 8)) return false;
    if (scoped.v6()->sin6_addr.s6_addr[3] != 7 || numeric_address(scoped) != "fe80::42" || copy_ip_identity(scoped, bytes.data()) != bytes.size() ||
        memcmp(bytes.data(), &expected, bytes.size())) return false;

    // Truncated/unsupported socket records have no printable address or IP identity.
    for (auto family : {AF_INET, AF_INET6, AF_UNIX}) {
        std::array<uint8_t, 2> header {2, static_cast<uint8_t>(family)};
        const auto absent = copy_socket_address(reinterpret_cast<const sockaddr *>(header.data()));
        if (absent.family() != AF_UNSPEC || !numeric_address(absent).empty() || copy_ip_identity(absent, bytes.data())) return false;
    }
    return true;
}

static bool resolved_addresses_preserve_endpoint() {
    const auto port = htons(54321);
    const uint32_t index = 7;
    sockaddr_storage target {};
    sockaddr header {};
    if (copy_resolved_address(nullptr, port, index, target)) return false;
    for (auto family : {AF_INET, AF_INET6}) {
        header.sa_family = family;
        for (auto length : {0, 1, 2, int(family == AF_INET ? sizeof(sockaddr_in) : sizeof(sockaddr_in6)) - 1}) {
            header.sa_len = length;
            if (copy_resolved_address(&header, port, index, target)) return false;
        }
    }
    sockaddr_in source4 {};
    source4.sin_len = sizeof(source4);
    source4.sin_family = AF_INET;
    source4.sin_addr.s_addr = htonl(0xc0000201);
    if (!copy_resolved_address((const sockaddr *)&source4, port, index, target)) return false;
    sockaddr_in result4;
    memcpy(&result4, &target, sizeof(result4));
    if (result4.sin_len != sizeof(result4) || result4.sin_family != AF_INET ||
        result4.sin_port != port || result4.sin_addr.s_addr != source4.sin_addr.s_addr) return false;

    sockaddr_in6 source6 {};
    source6.sin6_len = sizeof(source6);
    source6.sin6_family = AF_INET6;
    for (const char *host : {"fe80::23", "2001:db8::42"}) {
        if (inet_pton(AF_INET6, host, &source6.sin6_addr) != 1) return false;
        source6.sin6_scope_id = IN6_IS_ADDR_LINKLOCAL(&source6.sin6_addr) ? 0 : 19;
        sockaddr_storage extended {};
        memcpy(&extended, &source6, sizeof(source6));
        extended.ss_len = sizeof(extended);
        if (!copy_resolved_address((const sockaddr *)&extended, port, index, target)) return false;
        sockaddr_in6 result6;
        memcpy(&result6, &target, sizeof(result6));
        if (result6.sin6_len != sizeof(result6) || result6.sin6_family != AF_INET6 ||
            result6.sin6_port != port || memcmp(&result6.sin6_addr, &source6.sin6_addr, sizeof(in6_addr)) ||
            result6.sin6_scope_id != (IN6_IS_ADDR_LINKLOCAL(&source6.sin6_addr) ? index : 19)) return false;
    }
    if (inet_pton(AF_INET6, "fe80::23", &source6.sin6_addr) != 1) return false;
    source6.sin6_scope_id = index;
    source6.sin6_addr.s6_addr[3] = index;
    if (!copy_resolved_address((const sockaddr *)&source6, port, index, target)) return false;
    sockaddr_in6 result6;
    memcpy(&result6, &target, sizeof(result6));
    if (result6.sin6_scope_id != index || result6.sin6_addr.s6_addr[3] != 0) return false;
    source6.sin6_scope_id = index + 1;
    if (copy_resolved_address((const sockaddr *)&source6, port, index, target)) return false;
    source6.sin6_scope_id = index;
    source6.sin6_addr.s6_addr[3] = index + 1;
    return !copy_resolved_address((const sockaddr *)&source6, port, index, target);
}

static bool route_queries_reject_retired_callbacks() {
    auto handle = std::make_shared<Connection>(nil);
    auto create = [](DNSServiceRef *out, void *) {
        *out = test_ref();
        return kDNSServiceErr_NoError;
    };
    auto failed = [](DNSServiceRef *, void *) { return kDNSServiceErr_BadState; };
    const auto ref = test_ref();
    const auto port = htons(49170);
    if (start_query(handle, handle->resolve, failed) || releasedQueries || !queryOwners.empty()) return false;
    failQueueAssignment = true;
    bool started = start_query(handle, handle->resolve, create);
    failQueueAssignment = false;
    if (started || releasedQueries != 1 || handle->resolve.id || !queryOwners.empty()) return false;
    if (!start_query(handle, handle->resolve, create)) return false;
    void *first = reinterpret_cast<void *>(handle->resolve.id);
    clear_resolution(handle);
    if (!start_query(handle, handle->resolve, create)) return false;
    void *current = reinterpret_cast<void *>(handle->resolve.id);
    // Stale failure, wrong phase and wrong successful SDK ref must leave
    // the replacement untouched, even when the SDK reuses the old ref.
    resolved_service(ref, 0, 0, kDNSServiceErr_BadState, nullptr, nullptr, 0, 0, nullptr, first);
    resolved_service(nullptr, 0, 7, 0, nullptr, nullptr, 0, 0, nullptr, current);
    resolved_address(ref, 0, 0, kDNSServiceErr_BadState, nullptr, nullptr, 0, current);
    if (first == current || handle->stopping || !handle->resolve.id || releasedQueries != 2) return false;
    auto epoch = handle->resolutionEpoch;
    resolved_service(ref, 0, 7, 0, nullptr, "route-test.local.", port, 0, nullptr, current);
    if (handle->port != port || handle->resolve.id || !addressCookie ||
        handle->addresses.id != reinterpret_cast<uintptr_t>(addressCookie) || releasedQueries != 3 ||
        handle->resolutionEpoch != epoch || queryOwners.size() != 1) return false;
    resolved_service(ref, 0, 0, kDNSServiceErr_BadState, nullptr, nullptr, 0, 0, nullptr, current);
    resolved_service(ref, 0, 0, kDNSServiceErr_BadState, nullptr, nullptr, 0, 0, nullptr, addressCookie);
    if (handle->stopping || releasedQueries != 3) return false;
    cancel_connection(handle);
    if (releasedQueries != 4 || !queryOwners.empty()) return false;
    std::weak_ptr<Connection> weak = handle;
    handle.reset();
    resolved_address(ref, 0, 0, kDNSServiceErr_BadState, nullptr, nullptr, 0, addressCookie);
    return weak.expired() && releasedQueries == 4;
}

static bool route_query_exhaustion_stops_creation() {
    // Last test in this fixture process. Leave the allocator exhausted.
    nextQueryId = UINTPTR_MAX;
    auto handle = std::make_shared<Connection>(nil);
    unsigned creations = 0;
    auto create = [&](DNSServiceRef *out, void *) {
        ++creations;
        *out = test_ref();
        return kDNSServiceErr_NoError;
    };
    if (!start_query(handle, handle->resolve, create) || handle->resolve.id != UINTPTR_MAX) return false;
    retire_query(handle->resolve);
    return !start_query(handle, handle->resolve, create) && creations == 1 && queryOwners.empty();
}

int main() {
    @autoreleasepool {
        bool passed = true;
        for (size_t length : {size_t(81), size_t(16 * 1024)}) {
            for (bool before : {false, true}) {
                bool ok = retained_content_is_owned(length, before);
                std::printf("{\"length\":%zu,\"releaseBeforeCallback\":%s,\"ownedContentPreserved\":%s}\n",
                            length, before ? "true" : "false", ok ? "true" : "false");
                passed &= ok;
            }
        }
        const bool read_ok = finish_and_revoke();
        std::printf("{\"finalPayloadAndRevocation\":%s}\n", read_ok ? "true" : "false");
        const bool ids_ok = retired_ids_do_not_alias() && concurrent_leases_and_exhaustion();
        std::printf("{\"typedHandleLifetimes\":%s}\n", ids_ok ? "true" : "false");
        const bool receipt_ok = route_receipt_uses_effective_endpoints();
        std::printf("{\"effectiveEndpointReceipt\":%s}\n", receipt_ok ? "true" : "false");
        passed &= receipt_ok;
        const bool endpoint_ok = endpoint_addresses_outlive_their_sources();
        std::printf("{\"ownedEndpointAddresses\":%s}\n", endpoint_ok ? "true" : "false");
        passed &= endpoint_ok;
        const bool addresses_ok = resolved_addresses_preserve_endpoint();
        std::printf("{\"resolvedAddressBounds\":%s}\n", addresses_ok ? "true" : "false");
        __block bool queries_ok = false;
        dispatch_sync(queue(), ^{
            queries_ok = route_queries_reject_retired_callbacks() && route_query_exhaustion_stops_creation();
        });
        std::printf("{\"routeQueryOwnership\":%s}\n", queries_ok ? "true" : "false");
        const bool routes_ok = selected_route_stays_on_bound_subnet();
        std::printf("{\"boundRouteSubnet\":%s}\n", routes_ok ? "true" : "false");
        passed &= routes_ok;
        const bool masks_ok = compact_route_masks();
        std::printf("{\"compactRouteMasks\":%s}\n", masks_ok ? "true" : "false");
        return passed && read_ok && ids_ok && addresses_ok && queries_ok && masks_ok ? 0 : 1;
    }
}
