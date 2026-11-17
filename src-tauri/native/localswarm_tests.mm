// SPDX-License-Identifier: GPL-3.0-or-later
// Exercise the real bridge while the stack retains content beyond its callback.
#import <Foundation/Foundation.h>
#import <Network/Network.h>
#include <algorithm>
#include <cstdio>
#include <vector>

static dispatch_data_t retained_send;
static void capture_send(nw_connection_t, dispatch_data_t content, nw_content_context_t,
                         bool, nw_connection_send_completion_t completed) {
    retained_send = content;
    completed(nullptr);
}

#define nw_connection_send capture_send
#ifndef LOCALSWARM_BRIDGE_SOURCE
#define LOCALSWARM_BRIDGE_SOURCE "localswarm.mm"
#endif
#include LOCALSWARM_BRIDGE_SOURCE
#undef nw_connection_send

struct WriteResult {
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    int status = -1;
    size_t length = 0;
    unsigned callbacks = 0;
};
static void write_completed(void *context, int status, const uint8_t *, size_t length) {
    auto result = static_cast<WriteResult *>(context);
    result->status = status;
    result->length = length;
    ++result->callbacks;
    dispatch_semaphore_signal(result->done);
}

static bool retained_content_is_owned(size_t length, bool release_before_callback) {
    std::vector<uint8_t> input(length);
    for (size_t i = 0; i < length; ++i) input[i] = (uint8_t)(i * 31 + i / 251);
    const auto expected = input;
    auto handle = std::make_shared<Connection>(nil);
    handle->ready = true;
    WriteResult result;
    kiwix_swarm_write(&handle, input.data(), input.size(), &result, write_completed);
    if (release_before_callback) std::fill(input.begin(), input.end(), 0xa5);
    if (dispatch_semaphore_wait(result.done, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC))) return false;
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
    return matches && result.status == 0 && result.length == length && result.callbacks == 1;
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
        return passed ? 0 : 1;
    }
}
