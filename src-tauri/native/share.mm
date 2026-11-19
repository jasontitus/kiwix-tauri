// SPDX-License-Identifier: GPL-3.0-or-later
// AppKit picker ownership. Rust retains the source until the picker dismisses
// without a selection or the selected service reaches its terminal callback.
#import <AppKit/AppKit.h>
#include <cstdint>

using AdmitShare = bool (*)(uint64_t);
using KiwixShareCompletion = void (*)(uint64_t, uint8_t, int64_t);

// These values match complete in src/share.rs.
enum class ShareStatus : uint8_t {
    Shared = 0,
    Dismissed = 1,
    FileUnavailable = 2,
    WindowUnavailable = 3,
    AuthorityExpired = 4,
    Cancelled = 5,
    TimedOut = 6,
    ServiceFailed = 7,
    ReferenceUnavailable = 9,
};

static constexpr NSTimeInterval KiwixShareTimeout = 120;

@interface KiwixShareRequest : NSObject <NSSharingServicePickerDelegate, NSSharingServiceDelegate>
@property(nonatomic) uint64_t token;
@property(nonatomic) AdmitShare live;
@property(nonatomic) KiwixShareCompletion completion;
@property(nonatomic, strong) NSSharingServicePicker *picker;
@property(nonatomic, strong) NSMenu *menu;
@property(nonatomic, strong) NSSharingService *service;
@property(nonatomic, strong) NSWindow *anchor;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic) NSTimeInterval expires;
@property(nonatomic) ShareStatus closing;
@property(nonatomic) BOOL finished;
- (void)closeUnselected:(ShareStatus)reason;
- (void)finish:(ShareStatus)status detail:(NSInteger)detail;
@end

static NSMutableDictionary<NSNumber *, KiwixShareRequest *> *shareRequests() {
    static NSMutableDictionary<NSNumber *, KiwixShareRequest *> *requests = [NSMutableDictionary new];
    return requests;
}

@implementation KiwixShareRequest
- (void)closeUnselected:(ShareStatus)reason {
    if (self.finished || self.service || self.closing != ShareStatus::Shared) return;
    self.closing = reason;
    [self.timer invalidate]; self.timer = nil;
    // Only the picker owned by this request is closed. Once a service has
    // been selected, it can still hold the file and controls its own UI.
    if (self.menu) [self.menu cancelTracking];
    else [self.picker close];
}
- (void)finish:(ShareStatus)status detail:(NSInteger)detail {
    if (self.finished) return;
    self.finished = YES;
    [self.timer invalidate]; self.timer = nil;
    self.menu = nil;
    self.picker.delegate = nil; self.picker = nil;
    if (self.service.delegate == self) self.service.delegate = nil;
    self.service = nil; self.anchor = nil;
    KiwixShareCompletion completion = self.completion;
    uint64_t token = self.token;
    self.completion = nullptr; self.live = nullptr;
    [shareRequests() removeObjectForKey:@(token)];
    if (completion) completion(token, static_cast<uint8_t>(status), detail);
}
- (BOOL)selectedService:(NSSharingService *)service fromPicker:(NSSharingServicePicker *)picker {
    if (self.finished || !service || picker != self.picker || (self.service && self.service != service)) return NO;
    self.service = service;
    [self.timer invalidate]; self.timer = nil;
    return YES;
}
- (id<NSSharingServiceDelegate>)sharingServicePicker:(NSSharingServicePicker *)picker
                          delegateForSharingService:(NSSharingService *)service {
    return [self selectedService:service fromPicker:picker] ? self : nil;
}
- (void)sharingServicePicker:(NSSharingServicePicker *)picker didChooseSharingService:(NSSharingService *)service {
    if (self.finished || picker != self.picker) return;
    if (!service) {
        if (self.service) return;
        ShareStatus result = self.closing == ShareStatus::Shared ? ShareStatus::Dismissed : self.closing;
        [self finish:result detail:0];
        return;
    }
    if ([self selectedService:service fromPicker:picker] && !service.delegate) service.delegate = self;
}
- (void)sharingService:(NSSharingService *)service didShareItems:(NSArray *)items {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self.service == service) [self finish:ShareStatus::Shared detail:0];
    });
}
- (void)sharingService:(NSSharingService *)service didFailToShareItems:(NSArray *)items error:(NSError *)error {
    dispatch_async(dispatch_get_main_queue(), ^{
        if (self.service != service) return;
        bool cancelled = [error.domain isEqualToString:NSCocoaErrorDomain] && error.code == NSUserCancelledError;
        [self finish:cancelled ? ShareStatus::Cancelled : ShareStatus::ServiceFailed detail:error.code];
    });
}
@end

static bool matchesShareReference(NSURL *reference, uint64_t device, uint64_t inode) {
    // Ask the reference for its identity. Resolving it to the current leaf
    // pathname and then looking up that name would permit a different file
    // to be substituted between the two lookups.
    NSDictionary *values = [reference resourceValuesForKeys:@[
        NSURLFileIdentifierKey, NSURLIsRegularFileKey, NSURLVolumeURLKey
    ] error:nil];
    NSNumber *number = values[NSURLFileIdentifierKey];
    NSURL *volume = values[NSURLVolumeURLKey];
    if (!number || !volume.path || ![values[NSURLIsRegularFileKey] boolValue]
        || number.unsignedLongLongValue != inode) return false;
    NSDictionary *metadata = [NSFileManager.defaultManager attributesOfItemAtPath:volume.path error:nil];
    NSNumber *volumeDevice = metadata[NSFileSystemNumber];
    return volumeDevice && volumeDevice.unsignedLongLongValue == device;
}

static NSArray<NSURL *> *shareItems(const char *paths, ShareStatus *failure) {
    NSData *json = [[NSString stringWithUTF8String:paths] dataUsingEncoding:NSUTF8StringEncoding];
    if (!json) return nil;
    id decoded = [NSJSONSerialization JSONObjectWithData:json options:0 error:nil];
    if (![decoded isKindOfClass:NSArray.class]) return nil;
    NSArray *parts = decoded;
    if (!parts.count || parts.count > 676) return nil;
    NSMutableArray<NSURL *> *items = [NSMutableArray arrayWithCapacity:parts.count];
    for (id part in parts) {
        if (![part isKindOfClass:NSDictionary.class]) return nil;
        NSString *path = part[@"path"];
        NSNumber *device = part[@"device"];
        NSNumber *inode = part[@"inode"];
        if (![path isKindOfClass:NSString.class] || !path.isAbsolutePath
            || ![device isKindOfClass:NSNumber.class] || ![inode isKindOfClass:NSNumber.class]) return nil;
        NSURL *reference = [[NSURL fileURLWithPath:path] fileReferenceURL];
        if (!reference.isFileReferenceURL) {
            *failure = ShareStatus::ReferenceUnavailable;
            return nil;
        }
        if (!matchesShareReference(reference, device.unsignedLongLongValue, inode.unsignedLongLongValue)) return nil;
        [items addObject:reference];
    }
    return items;
}

extern "C" bool kiwix_share_present(uint64_t token, void *rawHost, const char *paths,
                                   AdmitShare admit, AdmitShare live, KiwixShareCompletion completion) {
    if (!NSThread.isMainThread || !rawHost || !paths || !admit || !live || !completion || shareRequests().count) return false;
    if (!admit(token)) {
        completion(token, static_cast<uint8_t>(ShareStatus::AuthorityExpired), 0);
        return true;
    }
    ShareStatus failure = ShareStatus::FileUnavailable;
    NSArray<NSURL *> *items = shareItems(paths, &failure);
    if (!items) {
        completion(token, static_cast<uint8_t>(failure), 0);
        return true;
    }
    NSWindow *anchor = (__bridge NSWindow *)rawHost;
    NSWindow *selected = anchor.tabGroup.selectedWindow;
    NSView *view = anchor.contentView;
    if (!view || !anchor.isVisible || ![NSApp.windows containsObject:anchor] || (selected && selected != anchor)) {
        completion(token, static_cast<uint8_t>(ShareStatus::WindowUnavailable), 0);
        return true;
    }
    NSSharingServicePicker *picker = [[NSSharingServicePicker alloc] initWithItems:items];
    if (!picker) return false;
    NSMenuItem *share = picker.standardShareMenuItem;
    if (!share) return false;
    NSMenu *menu = [NSMenu new];
    [menu addItem:share];
    KiwixShareRequest *request = [KiwixShareRequest new];
    request.token = token; request.completion = completion; request.live = live;
    request.picker = picker; request.anchor = anchor; request.menu = menu;
    picker.delegate = request;
    shareRequests()[@(token)] = request;
    __weak KiwixShareRequest *weakRequest = request;
    request.expires = NSProcessInfo.processInfo.systemUptime + KiwixShareTimeout;
    // A menu tracks events on the main thread. The common-mode timer can
    // close this request's menu even while a queued cancellation is pending.
    request.timer = [NSTimer timerWithTimeInterval:0.25 repeats:YES block:^(NSTimer *) {
        KiwixShareRequest *liveRequest = weakRequest;
        if (!liveRequest || liveRequest.finished || liveRequest.service) return;
        if (!liveRequest.live(liveRequest.token)) [liveRequest closeUnselected:ShareStatus::Cancelled];
        else if (NSProcessInfo.processInfo.systemUptime >= liveRequest.expires)
            [liveRequest closeUnselected:ShareStatus::TimedOut];
    }];
    [[NSRunLoop mainRunLoop] addTimer:request.timer forMode:NSRunLoopCommonModes];
    // Enter the public menu-based share route after native preparation and
    // the final guarded application check. No mouse event from IPC is used.
    if (!admit(token)) {
        [request finish:ShareStatus::AuthorityExpired detail:0];
        return true;
    }
    NSPoint point = NSMakePoint(NSMidX(view.bounds), NSMidY(view.bounds));
    BOOL chose = [menu popUpMenuPositioningItem:nil atLocation:point inView:view];
    request.menu = nil;
    if (!request.finished && !request.service) {
        if (!chose) {
            ShareStatus result = request.closing == ShareStatus::Shared ? ShareStatus::Dismissed : request.closing;
            [request finish:result detail:0];
        } else if (request.closing != ShareStatus::Shared) {
            [picker close];
        }
    }
    return true;
}

extern "C" void kiwix_share_cancel(uint64_t token) {
    dispatch_async(dispatch_get_main_queue(), ^{ [shareRequests()[@(token)] closeUnselected:ShareStatus::Cancelled]; });
}
