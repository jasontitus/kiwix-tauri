// SPDX-License-Identifier: GPL-3.0-or-later
// Exercise the real bridge while the stack retains content beyond its callback.
#import <Foundation/Foundation.h>
#import <Network/Network.h>
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

#define nw_connection_receive capture_receive
#define nw_connection_send capture_send
#ifndef LOCALSWARM_BRIDGE_SOURCE
#define LOCALSWARM_BRIDGE_SOURCE "localswarm.mm"
#endif
#include LOCALSWARM_BRIDGE_SOURCE
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
        return passed && read_ok && ids_ok ? 0 : 1;
    }
}
