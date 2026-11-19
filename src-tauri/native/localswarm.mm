// SPDX-License-Identifier: GPL-3.0-or-later
// Network.framework transport only. Framing, authentication and archive consent stay in Rust.
// includePeerToPeer / QUIC configuration follows LocalSwarm (Apache-2.0; see localswarm/NOTICE).
#import <Foundation/Foundation.h>
#import <Network/Network.h>
#import <Security/Security.h>
#include <ifaddrs.h>
#include <net/if.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <dns_sd.h>
#include <string>
#include <array>
#include <memory>
#include <mutex>
#include <condition_variable>
#include <cerrno>

using Completion = void (*)(void *, int, const uint8_t *, size_t);
using Accepted = void (*)(void *, void *);
using Dispose = void (*)(void *);
static dispatch_queue_t queue() {
    static dispatch_queue_t value = dispatch_queue_create("org.kiwix.localswarm.quic", DISPATCH_QUEUE_SERIAL);
    return value;
}
static sec_identity_t identity;
static NSData *certificate;

#if TARGET_OS_OSX
// Apple documents SecIdentityCreate as public Security API available since
// macOS 10.12 (developer.apple.com/documentation/security/secidentitycreate(_:_:_:)),
// but SDKs up to at least macOS 15.5 (Xcode 16.4) do not declare it. This
// matches the documented signature (nullable allocator and result, Create
// rule) and is a compatible redeclaration where a newer SDK declares it.
CF_ASSUME_NONNULL_BEGIN
extern "C" SecIdentityRef _Nullable SecIdentityCreate(CFAllocatorRef _Nullable allocator,
                                                      SecCertificateRef certificate,
                                                      SecKeyRef privateKey);
CF_ASSUME_NONNULL_END

// The identity is only usable for TLS if its own private key signs data that
// the pinned certificate's public key verifies. Checked once before use, so a
// mismatched or unusable key fails closed instead of at every handshake.
static bool identity_signs(SecIdentityRef candidate, SecCertificateRef leaf) {
    SecKeyRef privateKey = nullptr;
    if (SecIdentityCopyPrivateKey(candidate, &privateKey) != errSecSuccess || !privateKey) return false;
    SecKeyRef publicKey = SecCertificateCopyKey(leaf);
    const SecKeyAlgorithm algorithm = kSecKeyAlgorithmECDSASignatureMessageX962SHA256;
    NSData *message = [@"org.kiwix.localswarm identity self-check" dataUsingEncoding:NSUTF8StringEncoding];
    NSData *signature = publicKey && SecKeyIsAlgorithmSupported(privateKey, kSecKeyOperationTypeSign, algorithm)
        ? CFBridgingRelease(SecKeyCreateSignature(privateKey, algorithm, (__bridge CFDataRef)message, nullptr))
        : nil;
    bool valid = signature && SecKeyVerifySignature(publicKey, algorithm, (__bridge CFDataRef)message,
                                                     (__bridge CFDataRef)signature, nullptr);
    if (publicKey) CFRelease(publicKey);
    CFRelease(privateKey);
    return valid;
}
#endif

extern "C" bool kiwix_swarm_identity(const uint8_t *p12, size_t length, const uint8_t *der, size_t der_length) {
    @autoreleasepool {
#if TARGET_OS_OSX
        // SecIdentityCreate pairs the pinned certificate with a non-permanent
        // in-memory EC key. No keychain item, ACL or temporary file is created,
        // so nothing persists or needs cleanup (no keychain access intended).
        // SecPKCS12Import imports into the default keychain on macOS unless
        // kSecImportToMemoryOnly is set, which needs macOS 15 (minimum: 13.3).
        (void)p12; (void)length;
        NSData *expected = [NSData dataWithBytes:der length:der_length];
        NSData *keyData = [[NSData alloc] initWithBase64EncodedString:
            @"BMmKB08ZHj337ZVv1rqu+liHOmXlxj3MTDR5ozCmPE0R1xaZNjXbfO2Rw/fbN7W0SrKOFDoUVIiWvf5g0E4ALKDz/tTOlQpFA+XiilAsaH8isuY+k95px2uGaP/1PRGTCw=="
            options:0];
        SecCertificateRef leaf = SecCertificateCreateWithData(nullptr, (__bridge CFDataRef)expected);
        NSDictionary *attributes = @{
            (__bridge id)kSecAttrKeyType: (__bridge id)kSecAttrKeyTypeECSECPrimeRandom,
            (__bridge id)kSecAttrKeyClass: (__bridge id)kSecAttrKeyClassPrivate,
            (__bridge id)kSecAttrIsPermanent: @NO,
        };
        SecKeyRef key = SecKeyCreateWithData((__bridge CFDataRef)keyData, (__bridge CFDictionaryRef)attributes, nullptr);
        SecIdentityRef imported = leaf && key ? SecIdentityCreate(nullptr, leaf, key) : nullptr;
        if (imported && identity_signs(imported, leaf)) {
            identity = sec_identity_create(imported);
            certificate = expected;
        }
        if (imported) CFRelease(imported);
        if (key) CFRelease(key);
        if (leaf) CFRelease(leaf);
        return identity != nil;
#else
        CFArrayRef items = nullptr;
        NSMutableDictionary *options = [@{(__bridge id)kSecImportExportPassphrase: @"localswarm"} mutableCopy];
        OSStatus result = SecPKCS12Import((__bridge CFDataRef)[NSData dataWithBytes:p12 length:length], (__bridge CFDictionaryRef)options, &items);
        if (result != errSecSuccess || !items || CFArrayGetCount(items) != 1) { if (items) CFRelease(items); return false; }
        CFDictionaryRef item = (CFDictionaryRef)CFArrayGetValueAtIndex(items, 0);
        SecIdentityRef imported = (SecIdentityRef)CFDictionaryGetValue(item, kSecImportItemIdentity);
        SecCertificateRef leaf = nullptr;
        bool valid = imported && SecIdentityCopyCertificate(imported, &leaf) == errSecSuccess;
        NSData *expected = [NSData dataWithBytes:der length:der_length];
        if (valid) {
            NSData *actual = CFBridgingRelease(SecCertificateCopyData(leaf));
            valid = [actual isEqualToData:expected];
            CFRelease(leaf);
        }
        if (valid) { identity = sec_identity_create(imported); certificate = expected; }
        CFRelease(items);
        return valid && identity != nil;
#endif
    }
}

static nw_parameters_t parameters(bool peerToPeer = true) {
    if (!identity) return nil;
    nw_parameters_t params = nw_parameters_create_quic(^(nw_protocol_options_t quic) {
        sec_protocol_options_t tls = nw_quic_copy_sec_protocol_options(quic);
        sec_protocol_options_add_tls_application_protocol(tls, "localswarm-v1");
        sec_protocol_options_set_local_identity(tls, identity);
        sec_protocol_options_set_verify_block(tls, ^(sec_protocol_metadata_t, sec_trust_t trust, sec_protocol_verify_complete_t complete) {
            SecTrustRef value = sec_trust_copy_ref(trust);
            CFArrayRef chain = SecTrustCopyCertificateChain(value);
            bool valid = chain && CFArrayGetCount(chain) == 1;
            if (valid) {
                SecCertificateRef leaf = (SecCertificateRef)CFArrayGetValueAtIndex(chain, 0);
                NSData *actual = CFBridgingRelease(SecCertificateCopyData(leaf));
                valid = [actual isEqualToData:certificate];
            }
            if (chain) CFRelease(chain);
            CFRelease(value);
            complete(valid);
        }, queue());
        nw_quic_set_initial_max_streams_bidirectional(quic, 1);
        nw_quic_set_initial_max_streams_unidirectional(quic, 0);
        nw_quic_set_initial_max_data(quic, 64 * 1024 * 1024);
        nw_quic_set_initial_max_stream_data_bidirectional_local(quic, 32 * 1024 * 1024);
        nw_quic_set_initial_max_stream_data_bidirectional_remote(quic, 32 * 1024 * 1024);
        nw_quic_set_initial_max_stream_data_unidirectional(quic, 0);
        nw_quic_set_idle_timeout(quic, 120000);
    });
    nw_parameters_set_include_peer_to_peer(params, peerToPeer);
    nw_parameters_prohibit_interface_type(params, nw_interface_type_cellular);
    nw_parameters_set_local_only(params, true);
    return params;
}

static bool direct_name(const char *name) {
    return name && (!strncmp(name, "awdl", 4) || !strncmp(name, "llw", 3));
}
static bool direct_interface(nw_interface_t interface) {
    return direct_name(nw_interface_get_name(interface));
}
static bool sharing_interface(nw_interface_t interface) {
    auto type = nw_interface_get_type(interface);
    return direct_interface(interface) || type == nw_interface_type_wifi || type == nw_interface_type_wired;
}
static bool local_address(const sockaddr *remote, nw_interface_t interface) {
    if (!remote || !interface || !sharing_interface(interface)) return false;
    const char *name = nw_interface_get_name(interface);
    if (!name) return false;
    ifaddrs *addresses = nullptr;
    if (getifaddrs(&addresses) != 0) return false;
    bool allowed = false;
    for (auto entry = addresses; entry; entry = entry->ifa_next) {
        if (!entry->ifa_addr || !entry->ifa_netmask || strcmp(entry->ifa_name, name) ||
            !(entry->ifa_flags & IFF_UP) || entry->ifa_addr->sa_family != remote->sa_family) continue;
        if (remote->sa_family == AF_INET) {
            auto ip = ((const sockaddr_in *)remote)->sin_addr.s_addr;
            auto local = ((const sockaddr_in *)entry->ifa_addr)->sin_addr.s_addr;
            auto mask = ((const sockaddr_in *)entry->ifa_netmask)->sin_addr.s_addr;
            auto host = ntohl(ip);
            allowed = mask && host && (host >> 24) != 127 && (host >> 28) != 14 &&
                host != 0xffffffff && (ip & mask) == (local & mask);
        } else if (remote->sa_family == AF_INET6) {
            auto ip = &((const sockaddr_in6 *)remote)->sin6_addr;
            auto local = &((const sockaddr_in6 *)entry->ifa_addr)->sin6_addr;
            auto mask = &((const sockaddr_in6 *)entry->ifa_netmask)->sin6_addr;
            bool matches = true, nonzero = false;
            for (size_t i = 0; i < 16; ++i) {
                nonzero |= mask->s6_addr[i] != 0;
                matches &= (ip->s6_addr[i] & mask->s6_addr[i]) == (local->s6_addr[i] & mask->s6_addr[i]);
            }
            allowed = nonzero && matches && !IN6_IS_ADDR_UNSPECIFIED(ip) &&
                !IN6_IS_ADDR_LOOPBACK(ip) && !IN6_IS_ADDR_MULTICAST(ip) && !IN6_IS_ADDR_V4MAPPED(ip);
        }
        if (allowed) break;
    }
    freeifaddrs(addresses);
    return allowed;
}
static bool local_path(nw_path_t path) {
    if (!path || nw_path_get_status(path) != nw_path_status_satisfied ||
        nw_path_uses_interface_type(path, nw_interface_type_cellular)) return false;
    auto endpoint = nw_path_copy_effective_remote_endpoint(path);
    const sockaddr *remote = endpoint ? nw_endpoint_get_address(endpoint) : nullptr;
    __block bool allowed = false;
    nw_path_enumerate_interfaces(path, ^bool(nw_interface_t interface) {
        allowed = local_address(remote, interface);
        return !allowed;
    });
    return allowed;
}
static std::string numeric_address(const sockaddr *address) {
    char text[INET6_ADDRSTRLEN] = {0};
    if (!address) return "";
    if (address->sa_family == AF_INET &&
        inet_ntop(AF_INET, &((const sockaddr_in *)address)->sin_addr, text, sizeof(text))) return text;
    if (address->sa_family == AF_INET6 &&
        inet_ntop(AF_INET6, &((const sockaddr_in6 *)address)->sin6_addr, text, sizeof(text))) return text;
    return "";
}
static std::string interface_address(const char *name, int family) {
    ifaddrs *addresses = nullptr;
    if (!name || getifaddrs(&addresses) != 0) return "";
    std::string text;
    for (auto entry = addresses; entry; entry = entry->ifa_next) {
        if (!entry->ifa_addr || !entry->ifa_name || strcmp(entry->ifa_name, name) ||
            !(entry->ifa_flags & IFF_UP) || entry->ifa_addr->sa_family != family) continue;
        bool usable = false;
        if (family == AF_INET) {
            auto host = ntohl(((const sockaddr_in *)entry->ifa_addr)->sin_addr.s_addr);
            usable = host && (host >> 24) != 127 && host != 0xffffffff;
        } else if (family == AF_INET6) {
            auto ip = &((const sockaddr_in6 *)entry->ifa_addr)->sin6_addr;
            usable = !IN6_IS_ADDR_UNSPECIFIED(ip) && !IN6_IS_ADDR_LOOPBACK(ip) && !IN6_IS_ADDR_MULTICAST(ip);
        }
        if (!usable) continue;
        text = numeric_address(entry->ifa_addr);
        if (!text.empty()) break;
    }
    freeifaddrs(addresses);
    return text;
}
/// Bounded evidence of the live path: interface, its local address, the peer.
static std::string route_text(nw_connection_t value) {
    nw_path_t path = value ? nw_connection_copy_current_path(value) : nil;
    if (!path) return "";
    __block std::string interfaceName;
    nw_path_enumerate_interfaces(path, ^bool(nw_interface_t interface) {
        const char *name = nw_interface_get_name(interface);
        // The callback's interface/name is borrowed. Copy while it is alive.
        if (name && sharing_interface(interface)) { interfaceName = name; return false; }
        return true;
    });
    if (interfaceName.empty()) return "";
    auto endpoint = nw_path_copy_effective_remote_endpoint(path);
    const sockaddr *remote = endpoint ? nw_endpoint_get_address(endpoint) : nullptr;
    std::string text = interfaceName + (direct_name(interfaceName.c_str()) ? " (peer-to-peer)" : " (infrastructure)");
    auto local = interface_address(interfaceName.c_str(), remote ? remote->sa_family : AF_INET);
    auto peer = numeric_address(remote);
    if (!local.empty()) text += " " + local;
    if (!peer.empty()) text += " -> " + peer;
    return text;
}


struct Connection : std::enable_shared_from_this<Connection> {
    nw_connection_t value;
    nw_browser_t browser;
    nw_interface_t interface;
    std::array<nw_interface_t, 32> routes {};
    size_t routeCount = 0, nextRoute = 0;
    uint64_t resolutionEpoch = 0;
    uint32_t requiredIndex = 0;
    DNSServiceRef resolve = nullptr, addresses = nullptr;
    std::string instance;
    bool peerToPeer = false, stopping = false, connectionStopped = false, browserStopped = true;
    bool isolatedTransport = false;
    // Library streams own one admitted transport. Only an explicit Rust close
    // after serving has drained may retire it; stream failure alone cannot.
    dispatch_block_t retireTransport = nil;
    bool transportStopped = true;
    uint16_t port = 0;
    std::mutex mutex;
    std::condition_variable stopped;
    bool cancelled = false;
    bool ready = false; // All fields except cancelled are confined to queue().
    bool readEnded = false;
    Completion completion = nullptr;
    void *context = nullptr;
    explicit Connection(nw_connection_t value): value(value) {}
};
using SwarmConnectionHandle = std::shared_ptr<Connection>;

static void clear_resolution(const SwarmConnectionHandle &handle) {
    ++handle->resolutionEpoch;
    // DNS-SD requires deallocation on its configured serial callback queue.
    if (handle->resolve) { DNSServiceRefDeallocate(handle->resolve); handle->resolve = nullptr; }
    if (handle->addresses) { DNSServiceRefDeallocate(handle->addresses); handle->addresses = nullptr; }
}
static void finished_connection(const SwarmConnectionHandle &handle) {
    if (!handle->stopping || !handle->connectionStopped || !handle->browserStopped) return;
    if (handle->completion) {
        auto callback = handle->completion; handle->completion = nullptr;
        callback(handle->context, -1, nullptr, 0);
    }
    if (!handle->transportStopped) return;
    { std::lock_guard<std::mutex> lock(handle->mutex); handle->cancelled = true; }
    handle->stopped.notify_all();
}
static void cancel_connection(const SwarmConnectionHandle &handle) {
    handle->stopping = true; handle->ready = false;
    clear_resolution(handle);
    if (handle->browser && !handle->browserStopped) nw_browser_cancel(handle->browser);
    if (handle->value && !handle->connectionStopped) nw_connection_cancel(handle->value);
    finished_connection(handle);
}
static bool selected_path(const SwarmConnectionHandle &handle, nw_path_t path) {
    if (!local_path(path)) return false;
    if (!handle->requiredIndex) return true;
    if (!handle->interface || nw_interface_get_index(handle->interface) != handle->requiredIndex) return false;
    const char *name = nw_interface_get_name(handle->interface);
    if (!name || if_nametoindex(name) != handle->requiredIndex) return false;
    __block bool selected = false;
    nw_path_enumerate_interfaces(path, ^bool(nw_interface_t interface) {
        if (nw_interface_get_index(interface) == handle->requiredIndex) { selected = true; return false; }
        return true;
    });
    return selected;
}
static void start_connection(SwarmConnectionHandle handle) {
    handle->connectionStopped = false;
    nw_connection_set_queue(handle->value, queue());
    nw_connection_set_path_changed_handler(handle->value, ^(nw_path_t path) {
        if (handle->ready && !selected_path(handle, path)) cancel_connection(handle);
    });
    nw_connection_set_state_changed_handler(handle->value, ^(nw_connection_state_t state, __unused nw_error_t error) {
        if (state == nw_connection_state_ready) {
            handle->ready = !handle->stopping && selected_path(handle, nw_connection_copy_current_path(handle->value));
            if (!handle->ready) { cancel_connection(handle); return; }
            if (handle->completion) {
                auto callback = handle->completion; handle->completion = nullptr;
                callback(handle->context, 0, nullptr, 0);
            }
        } else if (state == nw_connection_state_failed) {
            cancel_connection(handle);
        } else if (state == nw_connection_state_cancelled) {
            handle->connectionStopped = true;
            nw_connection_set_path_changed_handler(handle->value, nil);
            nw_connection_set_state_changed_handler(handle->value, nil);
            handle->value = nil; // Break the connection/handler ownership cycle after its final callback.
            cancel_connection(handle);
        }
    });
    nw_connection_start(handle->value);
}
static void resolve_next(SwarmConnectionHandle handle);
static void schedule_next(SwarmConnectionHandle handle, int64_t delay) {
    if (handle->nextRoute >= handle->routeCount) return;
    auto epoch = handle->resolutionEpoch;
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, delay), queue(), ^{
        if (!handle->stopping && !handle->value && handle->resolutionEpoch == epoch) resolve_next(handle);
    });
}
static void resolved_address(DNSServiceRef, DNSServiceFlags flags, uint32_t index, DNSServiceErrorType error,
                             const char *, const sockaddr *address, uint32_t ttl, void *context) {
    auto handle = static_cast<Connection *>(context)->shared_from_this();
    if (handle->stopping || handle->value) return;
    if (error) { schedule_next(handle, 300 * NSEC_PER_MSEC); return; }
    if (!(flags & kDNSServiceFlagsAdd) || !ttl || index != nw_interface_get_index(handle->interface) ||
        !local_address(address, handle->interface)) {
        schedule_next(handle, 300 * NSEC_PER_MSEC);
        return;
    }
    sockaddr_storage target {};
    if (address->sa_family == AF_INET && address->sa_len >= sizeof(sockaddr_in)) {
        auto value = (sockaddr_in *)&target;
        *value = *(const sockaddr_in *)address; value->sin_port = handle->port;
    } else if (address->sa_family == AF_INET6 && address->sa_len >= sizeof(sockaddr_in6)) {
        auto value = (sockaddr_in6 *)&target;
        *value = *(const sockaddr_in6 *)address; value->sin6_port = handle->port;
        if (IN6_IS_ADDR_LINKLOCAL(&value->sin6_addr)) value->sin6_scope_id = index;
    } else { return; }
    auto params = parameters(handle->peerToPeer);
    if (!params) { cancel_connection(handle); return; }
    nw_parameters_require_interface(params, handle->interface);
    auto stack = nw_parameters_copy_default_protocol_stack(params);
    nw_ip_options_set_hop_limit(nw_protocol_stack_copy_internet_protocol(stack), 1);
    auto endpoint = nw_endpoint_create_address((sockaddr *)&target);
    clear_resolution(handle);
    // No QUIC packet is sent until numeric on-link validation and binding above.
    if (!endpoint) { cancel_connection(handle); return; }
    if (handle->isolatedTransport) {
        // Network.framework may otherwise join another request's QUIC
        // transport. A fresh privacy context is the public cache boundary;
        // its logging description is not its identity. Never reuse a context
        // across library requests or fall back to the process-wide context.
        auto privacy = nw_privacy_context_create("Nearby library request");
        if (!privacy) { cancel_connection(handle); return; }
        nw_parameters_set_privacy_context(params, privacy);
    }
    handle->value = nw_connection_create(endpoint, params);
    if (!handle->value) { cancel_connection(handle); return; }
    start_connection(handle);
    nw_browser_cancel(handle->browser);
}
static void resolved_service(DNSServiceRef, DNSServiceFlags, uint32_t index, DNSServiceErrorType error,
                             const char *, const char *host, uint16_t port, uint16_t, const unsigned char *, void *context) {
    auto handle = static_cast<Connection *>(context)->shared_from_this();
    if (handle->stopping || handle->value || handle->addresses) return;
    if (error || !host || !port || index != nw_interface_get_index(handle->interface)) {
        cancel_connection(handle); return;
    }
    size_t length = strnlen(host, 1009);
    if (length < 7 || length >= 1009 || strcasecmp(host + length - 7, ".local.")) {
        cancel_connection(handle); return;
    }
    // DNS-SD resolve subscriptions remain active through TXT/SRV changes. Retire
    // this one before admitting exactly one address operation; copy its borrowed
    // hostname before deallocation invalidates the callback payload.
    std::string resolvedHost(host, length);
    if (handle->resolve) { DNSServiceRefDeallocate(handle->resolve); handle->resolve = nullptr; }
    handle->port = port; // DNS-SD supplies network byte order.
    auto result = DNSServiceGetAddrInfo(&handle->addresses,
        kDNSServiceFlagsIncludeP2P | kDNSServiceFlagsIncludeAWDL, index,
        kDNSServiceProtocol_IPv4 | kDNSServiceProtocol_IPv6, resolvedHost.c_str(), resolved_address, handle.get());
    if (!result) result = DNSServiceSetDispatchQueue(handle->addresses, queue());
    if (result) cancel_connection(handle);
}
static void resolve_next(SwarmConnectionHandle handle) {
    clear_resolution(handle);
    if (handle->stopping || handle->value) return;
    if (handle->nextRoute >= handle->routeCount) { cancel_connection(handle); return; }
    handle->interface = handle->routes[handle->nextRoute++];
    auto error = DNSServiceResolve(&handle->resolve,
        kDNSServiceFlagsIncludeP2P | kDNSServiceFlagsIncludeAWDL,
        nw_interface_get_index(handle->interface), handle->instance.c_str(),
        "_localswarm._tcp", "local.", resolved_service, handle.get());
    if (!error) error = DNSServiceSetDispatchQueue(handle->resolve, queue());
    if (error) { clear_resolution(handle); schedule_next(handle, 0); }
    else schedule_next(handle, NSEC_PER_SEC);
}
static void *make_connection(const char *instance, bool peerToPeer, uint32_t requiredIndex, bool isolatedTransport) {
    if (!identity || !instance || !*instance || strnlen(instance, 64) > 63) return nullptr;
    auto handle = std::make_shared<Connection>(nil);
    handle->connectionStopped = true;
    handle->instance = instance; handle->peerToPeer = peerToPeer;
    handle->requiredIndex = requiredIndex;
    handle->isolatedTransport = isolatedTransport;
    return new SwarmConnectionHandle(std::move(handle));
}
extern "C" void *kiwix_swarm_connect_on_interface(const char *instance, bool peerToPeer, uint32_t requiredIndex) {
    return make_connection(instance, peerToPeer, requiredIndex, true);
}
extern "C" void *kiwix_swarm_connect(const char *instance, bool peerToPeer) {
    return make_connection(instance, peerToPeer, 0, false);
}
extern "C" uint32_t kiwix_swarm_interface_index(void *raw) {
    auto handle = *(SwarmConnectionHandle *)raw;
    __block uint32_t index = 0;
    dispatch_sync(queue(), ^{
        if (!handle->ready || handle->stopping || !handle->interface) return;
        const char *name = nw_interface_get_name(handle->interface);
        auto actual = nw_interface_get_index(handle->interface);
        if (name && if_nametoindex(name) == actual && selected_path(handle, nw_connection_copy_current_path(handle->value))) index = actual;
    });
    return index;
}
extern "C" size_t kiwix_swarm_local_address(void *raw, uint8_t *buffer, size_t capacity) {
    if (!buffer || capacity < 16) return 0;
    auto handle = *(SwarmConnectionHandle *)raw;
    __block size_t length = 0;
    dispatch_sync(queue(), ^{
        if (!handle->ready || handle->stopping || !handle->value) return;
        auto path = nw_connection_copy_current_path(handle->value);
        if (!selected_path(handle, path)) return;
        auto endpoint = nw_path_copy_effective_local_endpoint(path);
        const sockaddr *local = endpoint ? nw_endpoint_get_address(endpoint) : nullptr;
        if (!local) return;
        if (local->sa_family == AF_INET) {
            memcpy(buffer, &((const sockaddr_in *)local)->sin_addr, 4); length = 4;
        } else if (local->sa_family == AF_INET6) {
            auto address = ((const sockaddr_in6 *)local)->sin6_addr;
            if (IN6_IS_ADDR_LINKLOCAL(&address)) { address.s6_addr[2] = 0; address.s6_addr[3] = 0; }
            memcpy(buffer, &address, 16); length = 16;
        }
    });
    return length;
}
extern "C" bool kiwix_swarm_interface_has_address(uint32_t index, const uint8_t *bytes, size_t length) {
    if (!index || !bytes || (length != 4 && length != 16)) return false;
    ifaddrs *addresses = nullptr;
    if (getifaddrs(&addresses) != 0) return false;
    bool found = false;
    for (auto item = addresses; item && !found; item = item->ifa_next) {
        if (!item->ifa_name || !item->ifa_addr || !(item->ifa_flags & IFF_UP) || if_nametoindex(item->ifa_name) != index) continue;
        if (length == 4 && item->ifa_addr->sa_family == AF_INET) {
            found = memcmp(bytes, &((const sockaddr_in *)item->ifa_addr)->sin_addr, 4) == 0;
        } else if (length == 16 && item->ifa_addr->sa_family == AF_INET6) {
            // Include link-local addresses: the retained interface index is
            // their scope, so unrelated interfaces with the same IP cannot match.
            auto actual = ((const sockaddr_in6 *)item->ifa_addr)->sin6_addr;
            std::array<uint8_t, 16> expected;
            memcpy(expected.data(), bytes, expected.size());
            // Darwin may embed the link-local scope in bytes 2–3. The exact
            // interface index above already supplies that scope independently.
            if (IN6_IS_ADDR_LINKLOCAL(&actual)) { actual.s6_addr[2] = 0; actual.s6_addr[3] = 0; }
            if (expected[0] == 0xfe && (expected[1] & 0xc0) == 0x80) { expected[2] = 0; expected[3] = 0; }
            found = memcmp(expected.data(), &actual, expected.size()) == 0;
        }
    }
    freeifaddrs(addresses);
    return found;
}
extern "C" void kiwix_swarm_start(void *raw, void *context, Completion completion) {
    auto handle = *(SwarmConnectionHandle *)raw;
    dispatch_async(queue(), ^{
        handle->completion = completion; handle->context = context;
        if (handle->value) { start_connection(handle); return; }
        auto params = nw_parameters_create();
        nw_parameters_set_include_peer_to_peer(params, handle->peerToPeer);
        nw_parameters_prohibit_interface_type(params, nw_interface_type_cellular);
        auto descriptor = nw_browse_descriptor_create_bonjour_service("_localswarm._tcp", "local.");
        handle->browser = nw_browser_create(descriptor, params);
        if (!handle->browser) { cancel_connection(handle); return; }
        handle->browserStopped = false;
        nw_browser_set_queue(handle->browser, queue());
        nw_browser_set_state_changed_handler(handle->browser, ^(nw_browser_state_t state, nw_error_t) {
            if (state == nw_browser_state_failed) cancel_connection(handle);
            else if (state == nw_browser_state_cancelled) {
                handle->browserStopped = true;
                nw_browser_set_browse_results_changed_handler(handle->browser, nil);
                nw_browser_set_state_changed_handler(handle->browser, nil);
                handle->browser = nil; // The cancelled browser must not retain its captured owner.
                finished_connection(handle);
            }
        });
        nw_browser_set_browse_results_changed_handler(handle->browser, ^(nw_browse_result_t, nw_browse_result_t result, bool) {
            if (!result || handle->stopping || handle->value) return;
            auto endpoint = nw_browse_result_copy_endpoint(result);
            const char *name = nw_endpoint_get_bonjour_service_name(endpoint);
            if (!name || handle->instance != name) return;
            nw_browse_result_enumerate_interfaces(result, ^bool(nw_interface_t interface) {
                if (!sharing_interface(interface) || direct_interface(interface) != handle->peerToPeer) return true;
                auto index = nw_interface_get_index(interface);
                if (handle->requiredIndex && index != handle->requiredIndex) return true;
                for (size_t i = 0; i < handle->routeCount; ++i)
                    if (nw_interface_get_index(handle->routes[i]) == index) return true;
                if (handle->routeCount < handle->routes.size()) handle->routes[handle->routeCount++] = interface;
                return true;
            });
            if (!handle->routeCount) return;
            if (!handle->interface) resolve_next(handle);
            else schedule_next(handle, NSEC_PER_SEC);
        });
        nw_browser_start(handle->browser);
    });
}
// Network.framework lends its immutable dispatch data for the callback.
using ReadCompletion = void (*)(uint64_t, int, dispatch_data_t);
extern "C" void kiwix_swarm_read(void *raw, size_t maximum, uint64_t request, ReadCompletion completion) {
    auto handle = *(SwarmConnectionHandle *)raw;
    dispatch_async(queue(), ^{
        if (!handle->ready) { completion(request, -1, nil); return; }
        if (handle->readEnded) { completion(request, 0, nil); return; }
        nw_connection_receive(handle->value, 1, (uint32_t)maximum, ^(dispatch_data_t data, nw_content_context_t context, bool complete, nw_error_t error) {
            if (!handle->ready) { completion(request, -1, nil); return; }
            const bool empty = !data || dispatch_data_get_size(data) == 0;
            if (error) {
                // A receive queued before the final callback can still reach the OS.
                const bool exhausted = empty && handle->readEnded &&
                    nw_error_get_error_domain(error) == nw_error_domain_posix && nw_error_get_error_code(error) == ENODATA;
                completion(request, exhausted ? 0 : -1, nil);
                return;
            }
            if (complete && context && nw_content_context_get_is_final(context)) {
                // The final payload and FIN can arrive together. Deliver the bytes
                // now and retain EOF for the next read on this same stream.
                handle->readEnded = true;
            } else if (empty && complete && context) {
                // Let the live Rust reader request the next message. A cancelled
                // reader must never re-arm its operation from this native queue.
                completion(request, 1, nil);
                return;
            }
            completion(request, empty && !handle->readEnded ? -1 : 0, data);
        });
    });
}
extern "C" void kiwix_swarm_write(void *raw, const uint8_t *bytes, size_t length, void *context, Completion completion) {
    auto handle = *(SwarmConnectionHandle *)raw;
    // QUIC may retain content after the send callback (before acknowledgement).
    // Copy synchronously: Rust's borrowed input may be released on callback/cancel.
    auto data = dispatch_data_create(bytes, length, nullptr, DISPATCH_DATA_DESTRUCTOR_DEFAULT);
    dispatch_async(queue(), ^{
        if (!handle->ready) { completion(context, -1, nullptr, 0); return; }
        nw_connection_send(handle->value, data, NW_CONNECTION_DEFAULT_MESSAGE_CONTEXT, false, ^(nw_error_t error) {
            completion(context, error ? -1 : 0, nullptr, length);
        });
    });
}
extern "C" void kiwix_swarm_cancel(void *raw) {
    auto handle = *(SwarmConnectionHandle *)raw;
    dispatch_async(queue(), ^{
        auto retire = handle->retireTransport;
        handle->retireTransport = nil;
        if (retire) retire();
        cancel_connection(handle);
    });
}
extern "C" void kiwix_swarm_join(void *raw) {
    auto handle = *(SwarmConnectionHandle *)raw;
    kiwix_swarm_cancel(raw);
    std::unique_lock<std::mutex> lock(handle->mutex);
    handle->stopped.wait(lock, [&] { return handle->cancelled; });
}
extern "C" void kiwix_swarm_release(void *raw) { delete (SwarmConnectionHandle *)raw; }
extern "C" long kiwix_swarm_route(void *raw, char *buffer, size_t capacity) {
    auto handle = *(SwarmConnectionHandle *)raw;
    __block std::string text;
    dispatch_sync(queue(), ^{
        if (!handle->ready || !handle->value || handle->stopping) return;
        text = route_text(handle->value);
        if (text.empty() && handle->interface) {
            // Fall back to the interface the native resolver required. This is
            // the same route the connection was created on, even if the live
            // path cannot be enumerated yet.
            const char *name = nw_interface_get_name(handle->interface);
            if (name) {
                auto endpoint = nw_connection_copy_endpoint(handle->value);
                const sockaddr *remote = endpoint ? nw_endpoint_get_address(endpoint) : nullptr;
                text = std::string(name) + (direct_name(name) ? " (peer-to-peer)" : " (infrastructure)");
                auto local = interface_address(name, remote ? remote->sa_family : AF_INET);
                auto peer = numeric_address(remote);
                if (!local.empty()) text += " " + local;
                if (!peer.empty()) text += " -> " + peer;
            }
        }
    });
    if (text.empty() || !buffer || capacity == 0) return -1;
    auto length = text.size() < capacity - 1 ? text.size() : capacity - 1;
    memcpy(buffer, text.data(), length);
    buffer[length] = '\0';
    return (long)length;
}

struct Listener {
    nw_listener_t value;
    std::mutex mutex;
    std::condition_variable stopped;
    bool cancelled = false;
    bool stopping = false, listenerStopped = false;
    enum class GroupPhase : uint8_t { AwaitingStream, Active, Retiring };
    std::array<nw_connection_group_t, 16> groups {};
    std::array<uint64_t, 16> groupEpochs {};
    std::array<GroupPhase, 16> groupPhases {};
    std::array<std::weak_ptr<Connection>, 16> requests {};
    bool oneRequest;
    size_t groupCount = 0;
    Completion ready = nullptr;
    void *ready_context = nullptr;
    Accepted accept;
    Dispose dispose;
    void *context;
    Listener(nw_listener_t value, void *context, Accepted accept, Dispose dispose, bool oneRequest):
        value(value), oneRequest(oneRequest), accept(accept), dispose(dispose), context(context) {}
    ~Listener() { if (context) dispose(context); }
};
using ListenerHandle = std::shared_ptr<Listener>;
static void finished_listener(const ListenerHandle &listener) {
    if (!listener->listenerStopped || listener->groupCount) return;
    { std::lock_guard<std::mutex> lock(listener->mutex); listener->cancelled = true; }
    listener->stopped.notify_all();
}
static void cancel_listener(const ListenerHandle &listener) {
    listener->stopping = true;
    for (size_t index = 0; index < listener->groups.size(); ++index) {
        if (listener->groups[index]) nw_connection_group_cancel(listener->groups[index]);
    }
    if (listener->value && !listener->listenerStopped) nw_listener_cancel(listener->value);
    finished_listener(listener);
}
extern "C" void *kiwix_swarm_listen(void *context, Accepted accept, Dispose dispose, void *ready_context, Completion ready, bool oneRequest) {
    auto params = parameters();
    auto value = params ? nw_listener_create(params) : nil;
    if (!value) { dispose(context); ready(ready_context, -1, nullptr, 0); return nullptr; }
    auto listener = std::make_shared<Listener>(value, context, accept, dispose, oneRequest);
    listener->ready = ready; listener->ready_context = ready_context;
    nw_listener_set_queue(value, queue());
    // A multiplex listener owns QUIC transports separately from their incoming
    // application streams. Bound and retain the transports here; Rust receives
    // only their actual incoming byte streams, not a transport placeholder.
    nw_listener_set_new_connection_group_handler(value, ^(nw_connection_group_t group) {
        size_t slot = 0;
        while (slot < listener->groups.size() && listener->groups[slot]) ++slot;
        if (listener->stopping || !listener->context || slot == listener->groups.size()) {
            nw_connection_group_cancel(group); return;
        }
        listener->groups[slot] = group;
        ++listener->groupCount;
        void *key = (__bridge void *)group;
        const auto epoch = ++listener->groupEpochs[slot];
        listener->groupPhases[slot] = Listener::GroupPhase::AwaitingStream;
        nw_connection_group_set_queue(group, queue());
        nw_connection_group_set_receive_handler(group, 1, true,
            ^(__unused dispatch_data_t content, __unused nw_content_context_t context, __unused bool complete) {
                auto current = listener->groups[slot];
                if ((__bridge void *)current == key && listener->groupEpochs[slot] == epoch) {
                    listener->groupPhases[slot] = Listener::GroupPhase::Retiring;
                    nw_connection_group_cancel(current);
                }
            });
        nw_connection_group_set_new_connection_handler(group, ^(nw_connection_t incoming) {
            if (listener->stopping || !listener->context || (__bridge void *)listener->groups[slot] != key ||
                listener->groupEpochs[slot] != epoch || listener->groupPhases[slot] == Listener::GroupPhase::Retiring ||
                (listener->oneRequest && listener->groupPhases[slot] != Listener::GroupPhase::AwaitingStream)) {
                nw_connection_cancel(incoming); return;
            }
            listener->groupPhases[slot] = Listener::GroupPhase::Active;
            auto request = std::make_shared<Connection>(incoming);
            if (listener->oneRequest) {
                listener->requests[slot] = request;
                request->transportStopped = false;
                std::weak_ptr<Listener> weak = listener;
                request->retireTransport = ^{
                    auto owner = weak.lock();
                    if (!owner || owner->groupEpochs[slot] != epoch ||
                        (__bridge void *)owner->groups[slot] != key ||
                        owner->groupPhases[slot] == Listener::GroupPhase::Retiring) return;
                    // Cancelling an accepted nw_connection closes only its
                    // stream. Retire the owning QUIC transport after Rust has
                    // consumed the application ACK and drained source work.
                    owner->groupPhases[slot] = Listener::GroupPhase::Retiring;
                    nw_connection_group_cancel(owner->groups[slot]);
                };
            }
            listener->accept(listener->context, new SwarmConnectionHandle(std::move(request)));
        });
        nw_connection_group_set_state_changed_handler(group,
            ^(nw_connection_group_state_t state, __unused nw_error_t error) {
                auto current = listener->groups[slot];
                if ((__bridge void *)current != key || listener->groupEpochs[slot] != epoch) return;
                if (state == nw_connection_group_state_failed) {
                    listener->groupPhases[slot] = Listener::GroupPhase::Retiring;
                    nw_connection_group_cancel(current);
                } else if (state == nw_connection_group_state_cancelled) {
                    listener->groups[slot] = nil;
                    listener->groupPhases[slot] = Listener::GroupPhase::Retiring;
                    --listener->groupCount;
                    if (auto request = listener->requests[slot].lock()) {
                        request->transportStopped = true;
                        finished_connection(request);
                    }
                    listener->requests[slot].reset();
                    finished_listener(listener);
                }
            });
        nw_connection_group_start(group);
        // A completed handshake without an application stream must not hold
        // one of the bounded admission slots for the transport's idle timeout.
        // The weak owner and generation fence cannot retain or cancel a later
        // listener/transport that happens to reuse this slot or object address.
        std::weak_ptr<Listener> weak = listener;
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 8 * NSEC_PER_SEC), queue(), ^{
            auto active = weak.lock();
            if (active && !active->stopping && active->groupEpochs[slot] == epoch &&
                active->groupPhases[slot] == Listener::GroupPhase::AwaitingStream &&
                (__bridge void *)active->groups[slot] == key) {
                active->groupPhases[slot] = Listener::GroupPhase::Retiring;
                nw_connection_group_cancel(active->groups[slot]);
            }
        });
    });
    nw_listener_set_state_changed_handler(value, ^(nw_listener_state_t state, __unused nw_error_t error) {
        if (state == nw_listener_state_ready && listener->ready && !listener->stopping) {
            auto callback = listener->ready; listener->ready = nullptr;
            callback(listener->ready_context, 0, nullptr, nw_listener_get_port(listener->value));
        } else if (state == nw_listener_state_failed) { cancel_listener(listener); }
        else if (state == nw_listener_state_cancelled) {
            listener->stopping = true;
            listener->listenerStopped = true;
            listener->value = nil;
            if (listener->ready) { auto callback = listener->ready; listener->ready = nullptr; callback(listener->ready_context, -1, nullptr, 0); }
            // All accept callbacks precede cancellation on this queue. Closing
            // the Rust sender wakes its supervisor after initial readiness too.
            if (listener->context) {
                listener->dispose(listener->context);
                listener->context = nullptr;
            }
            for (size_t index = 0; index < listener->groups.size(); ++index) {
                if (listener->groups[index]) nw_connection_group_cancel(listener->groups[index]);
            }
            finished_listener(listener);
        }
    });
    nw_listener_start(value);
    return new ListenerHandle(listener);
}
extern "C" void kiwix_swarm_listener_cancel(void *raw) {
    auto listener = *(ListenerHandle *)raw;
    dispatch_async(queue(), ^{ cancel_listener(listener); });
}
extern "C" void kiwix_swarm_listener_join(void *raw) {
    auto handle = *(ListenerHandle *)raw;
    kiwix_swarm_listener_cancel(raw);
    std::unique_lock<std::mutex> lock(handle->mutex);
    handle->stopped.wait(lock, [&] { return handle->cancelled; });
}
extern "C" void kiwix_swarm_listener_release(void *raw) { delete (ListenerHandle *)raw; }
