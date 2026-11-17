// SPDX-License-Identifier: GPL-3.0-or-later
// Presentation-only NSSharingServicePicker bridge. Rust owns the shared file,
// its bounded lifetime and the admission that produced it; this module never
// reads archive bytes, opens URLs or retains a file beyond the sheet.
#import <AppKit/AppKit.h>
#include <stdint.h>

typedef void (*KiwixShareCompletion)(void *context, const char *error);

// A sheet the user leaves open still holds the file URL. Retire the request
// (and let Rust delete its scratch) after a bounded interval.
static const NSTimeInterval KiwixShareTimeout = 120;

@interface KiwixShareRequest : NSObject <NSSharingServicePickerDelegate, NSSharingServiceDelegate>
@property(nonatomic) uint64_t token;
@property(nonatomic) void *context;
@property(nonatomic) KiwixShareCompletion completion;
@property(nonatomic, strong) NSURL *url;
@property(nonatomic, strong) NSSharingServicePicker *picker;
@property(nonatomic, strong) NSWindow *anchor;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic) BOOL finished;
- (void)finish:(NSString *)error;
@end

static NSMutableDictionary<NSNumber *, KiwixShareRequest *> *shareRequests(void) {
    static NSMutableDictionary *requests;
    static dispatch_once_t once;
    dispatch_once(&once, ^{ requests = [NSMutableDictionary new]; });
    return requests;
}

@implementation KiwixShareRequest
- (void)finish:(NSString *)error {
    if (self.finished) return;
    self.finished = YES;
    [self.timer invalidate]; self.timer = nil;
    self.picker.delegate = nil; self.picker = nil;
    self.anchor = nil;
    KiwixShareCompletion completion = self.completion;
    void *context = self.context;
    self.completion = NULL; self.context = NULL;
    [shareRequests() removeObjectForKey:@(self.token)];
    if (completion) completion(context, error ? error.UTF8String : NULL);
}
- (void)sharingServicePicker:(NSSharingServicePicker *)picker didChooseSharingService:(NSSharingService *)service {
    // Completion (not choice) ends ownership: some services read the file while
    // composing. A dismissed picker without a choice is bounded by the timer.
    service.delegate = self;
}
- (void)sharingService:(NSSharingService *)service didShareItems:(NSArray *)items {
    [self finish:nil];
}
- (void)sharingService:(NSSharingService *)service didFailToShareItems:(NSArray *)items error:(NSError *)error {
    [self finish:error.localizedDescription ?: @"The share service failed"];
}
@end

extern "C" void kiwix_share_present(uint64_t token, const char *path, void *context, KiwixShareCompletion completion) {
    NSString *pathString = path ? [NSString stringWithUTF8String:path] : nil;
    dispatch_async(dispatch_get_main_queue(), ^{
        if (shareRequests()[@(token)] != nil) return;
        if (!pathString || !completion) {
            if (completion) completion(context, "Invalid share request");
            return;
        }
        NSURL *url = [NSURL fileURLWithPath:pathString];
        NSNumber *regular = nil;
        NSError *attributesError = nil;
        [url getResourceValue:&regular forKey:NSURLIsRegularFileKey error:&attributesError];
        if (attributesError || ![regular boolValue]) {
            completion(context, "The file to share is unavailable");
            return;
        }
        NSWindow *anchor = NSApp.keyWindow ?: NSApp.mainWindow ?: NSApp.windows.firstObject;
        NSView *view = anchor.contentView;
        if (!view) {
            completion(context, "No application window is available to present the share sheet");
            return;
        }
        KiwixShareRequest *request = [KiwixShareRequest new];
        request.token = token; request.context = context; request.completion = completion;
        request.url = url; request.anchor = anchor;
        shareRequests()[@(token)] = request;
        NSSharingServicePicker *picker = [[NSSharingServicePicker alloc] initWithItems:@[url]];
        picker.delegate = request;
        request.picker = picker;
        __weak KiwixShareRequest *weakRequest = request;
        request.timer = [NSTimer timerWithTimeInterval:KiwixShareTimeout repeats:NO block:^(NSTimer *timer) {
            (void)timer;
            [weakRequest finish:@"The share sheet did not complete"];
        }];
        [[NSRunLoop mainRunLoop] addTimer:request.timer forMode:NSRunLoopCommonModes];
        NSRect rect = NSMakeRect(NSMidX(view.bounds), NSMidY(view.bounds), 1, 1);
        [picker showRelativeToRect:rect ofView:view preferredEdge:NSRectEdgeMinY];
    });
}

extern "C" void kiwix_share_cancel(uint64_t token) {
    dispatch_async(dispatch_get_main_queue(), ^{
        [shareRequests()[@(token)] finish:@"Sharing was cancelled"];
    });
}
