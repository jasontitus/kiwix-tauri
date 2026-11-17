// SPDX-License-Identifier: GPL-3.0-or-later
// UserNotifications is a library-admission sink, never a download-completion sink.
#import <Foundation/Foundation.h>
#import <UserNotifications/UserNotifications.h>
#import <TargetConditionals.h>
#if TARGET_OS_IPHONE
#import <UIKit/UIKit.h>
#else
#import <AppKit/AppKit.h>
#endif
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <cstdlib>
#include <cstring>
#include <climits>
#include <atomic>

static const NSUInteger JournalLimit = 1024 * 1024;
static const NSUInteger NoticeLimit = 2048;
static const NSUInteger TapLimit = 32;
static NSString *const NoticePrefix = @"org.kiwix.acquisition.";
static NSString *const StateName = @"state.json";
typedef void (*KiwixNotificationPermissionCallback)(void *, int, const char *);
typedef void (*KiwixNotificationWaker)(void);
static std::atomic<KiwixNotificationWaker> notificationWaker{nullptr};
extern "C" void kiwix_notifications_set_waker(KiwixNotificationWaker wake) {
    notificationWaker.store(wake, std::memory_order_release);
}

static NSError *notificationError(NSString *message) {
    return [NSError errorWithDomain:@"org.kiwix.notifications" code:1
                          userInfo:@{NSLocalizedDescriptionKey: message}];
}
static BOOL text(id value, NSUInteger limit) {
    return [value isKindOfClass:NSString.class] && [value length] > 0
        && [(NSString *)value lengthOfBytesUsingEncoding:NSUTF8StringEncoding] <= limit
        && [(NSString *)value rangeOfCharacterFromSet:NSCharacterSet.controlCharacterSet].location == NSNotFound;
}
static BOOL identity(NSDictionary *value) {
    return [value isKindOfClass:NSDictionary.class] && text(value[@"receipt"], 512)
        && text(value[@"bookId"], 128) && text(value[@"installationId"], 128)
        && text(value[@"profileId"], 36) && [[NSUUID alloc] initWithUUIDString:value[@"profileId"]] != nil;
}
static BOOL sameIdentity(NSDictionary *a, NSDictionary *b) {
    return [a[@"receipt"] isEqual:b[@"receipt"]] && [a[@"bookId"] isEqual:b[@"bookId"]]
        && [a[@"installationId"] isEqual:b[@"installationId"]] && [a[@"profileId"] isEqual:b[@"profileId"]];
}
static BOOL writeAll(int fd, NSData *data) {
    const char *bytes = (const char *)data.bytes;
    NSUInteger left = data.length;
    while (left) {
        ssize_t count = write(fd, bytes, left);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) return NO;
        bytes += count; left -= (NSUInteger)count;
    }
    return fsync(fd) == 0;
}

@interface KiwixNotifications : NSObject <UNUserNotificationCenterDelegate>
@property(nonatomic, strong) id<UNUserNotificationCenterDelegate> previous;
@property(nonatomic, strong) NSMutableDictionary<NSString *, NSDictionary *> *notices;
@property(nonatomic, strong) NSMutableArray<NSString *> *order;
@property(nonatomic, strong) NSMutableArray<NSDictionary *> *taps;
@property(nonatomic, strong) NSMutableArray<NSDictionary *> *earlyTaps;
@property(nonatomic) BOOL validBundle;
@property(nonatomic) BOOL enabled;
@property(nonatomic) BOOL protectedMode;
@property(nonatomic) BOOL configured;
@property(nonatomic) BOOL writeAllowed;
@property(nonatomic) BOOL dirty;
@property(nonatomic) BOOL permissionInFlight;
@property(nonatomic) uint64_t epoch;
@property(nonatomic) int directory;
@property(nonatomic) int profile;
@property(nonatomic) BOOL profileLoaded;
@property(nonatomic, strong) NSString *profileId;
@property(nonatomic, strong) NSError *failure;
- (BOOL)openDirectory:(BOOL)create error:(NSError **)error;
- (BOOL)bindProfile:(NSDictionary *)request error:(NSError **)error;
- (BOOL)load:(NSError **)error;
- (BOOL)save:(NSError **)error;
- (NSDictionary *)call:(NSDictionary *)request error:(NSError **)error;
@end

@implementation KiwixNotifications
- (instancetype)init {
    if (!(self = [super init])) return nil;
    _directory = -1; _profile = -1; _notices = [NSMutableDictionary new]; _order = [NSMutableArray new];
    _taps = [NSMutableArray new]; _earlyTaps = [NSMutableArray new]; _protectedMode = YES;
    NSBundle *application = NSBundle.mainBundle;
    NSString *bundle = application.bundleIdentifier;
    _validBundle = text(bundle, 255) && ![bundle containsString:@"/"]
        && [application.bundleURL.pathExtension.lowercaseString isEqual:@"app"]
        && [[application objectForInfoDictionaryKey:@"CFBundlePackageType"] isEqual:@"APPL"];
    if (!_validBundle) {
        _failure = notificationError(@"Native notifications require an installed application bundle; headless and unbundled executables are unsupported");
        return self;
    }
    // No filesystem discovery belongs to the early application delegate. The
    // settings owner supplies a Store-validated retained profile capability.
    return self;
}
- (void)dealloc {
    if (_directory >= 0) close(_directory);
    if (_profile >= 0) close(_profile);
}
- (BOOL)bindProfile:(NSDictionary *)request error:(NSError **)error {
    NSNumber *number = request[@"profileFd"], *device = request[@"profileDevice"], *inode = request[@"profileInode"];
    if (![number isKindOfClass:NSNumber.class] || number.longLongValue < 0 || number.longLongValue > INT_MAX
        || ![device isKindOfClass:NSNumber.class] || ![inode isKindOfClass:NSNumber.class]) {
        if (error) *error = notificationError(@"Notification configuration requires a retained profile capability"); return NO;
    }
    int duplicate = fcntl(number.intValue, F_DUPFD_CLOEXEC, 0);
    struct stat st;
    if (duplicate < 0 || fstat(duplicate, &st) != 0 || !S_ISDIR(st.st_mode)
        || (uint64_t)st.st_dev != device.unsignedLongLongValue || (uint64_t)st.st_ino != inode.unsignedLongLongValue) {
        if (duplicate >= 0) close(duplicate);
        if (error) *error = notificationError(@"Notification profile capability changed identity"); return NO;
    }
    if (self.profile >= 0) {
        struct stat retained;
        BOOL same = fstat(self.profile, &retained) == 0 && retained.st_dev == st.st_dev && retained.st_ino == st.st_ino;
        close(duplicate);
        if (!same) { if (error) *error = notificationError(@"This notification delegate already belongs to another profile"); return NO; }
    } else self.profile = duplicate;
    if (!self.profileLoaded) {
        if (![self openDirectory:NO error:error] || ![self load:error]) return NO;
        self.profileLoaded = YES;
    }
    return YES;
}
- (BOOL)load:(NSError **)error {
    if (self.directory < 0) return YES;
    int file = openat(self.directory, StateName.fileSystemRepresentation, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC);
    if (file < 0) {
        if (errno == ENOENT) return YES;
        if (error) *error = notificationError(@"Opening notification handoff journal"); return NO;
    }
    struct stat st;
    if (fstat(file, &st) != 0 || !S_ISREG(st.st_mode) || st.st_nlink != 1
        || st.st_uid != geteuid() || st.st_size < 0 || (uint64_t)st.st_size > JournalLimit) {
        close(file); if (error) *error = notificationError(@"Notification handoff journal is not a bounded owned regular file"); return NO;
    }
    NSMutableData *data = [NSMutableData dataWithLength:(NSUInteger)st.st_size];
    NSUInteger offset = 0;
    while (offset < data.length) {
        ssize_t count = read(file, (char *)data.mutableBytes + offset, data.length - offset);
        if (count < 0 && errno == EINTR) continue;
        if (count <= 0) break;
        offset += (NSUInteger)count;
    }
    close(file);
    NSDictionary *saved = offset == data.length ? [NSJSONSerialization JSONObjectWithData:data options:0 error:error] : nil;
    if (![saved isKindOfClass:NSDictionary.class] || ![saved[@"version"] isEqual:@2]
        || !text(saved[@"profileId"], 36) || ![[NSUUID alloc] initWithUUIDString:saved[@"profileId"]]
        || ![saved[@"profileDevice"] isKindOfClass:NSNumber.class] || ![saved[@"profileInode"] isKindOfClass:NSNumber.class]
        || ![saved[@"notices"] isKindOfClass:NSArray.class] || [saved[@"notices"] count] > NoticeLimit
        || ![saved[@"taps"] isKindOfClass:NSArray.class] || [saved[@"taps"] count] > TapLimit) {
        if (error && !*error) *error = notificationError(@"Invalid notification handoff journal"); return NO;
    }
    struct stat root;
    if (fstat(self.profile, &root) != 0) { if (error) *error = notificationError(@"Reading retained notification profile identity"); return NO; }
    // A copied/restored profile is a new native notification principal. Never
    // redirect a notice for the original profile into a byte-identical clone.
    if ((uint64_t)root.st_dev != [saved[@"profileDevice"] unsignedLongLongValue]
        || (uint64_t)root.st_ino != [saved[@"profileInode"] unsignedLongLongValue]) return YES;
    self.profileId = saved[@"profileId"];
    for (NSDictionary *notice in saved[@"notices"]) {
        NSString *key = [notice isKindOfClass:NSDictionary.class] ? notice[@"id"] : nil;
        NSString *prefix = [NoticePrefix stringByAppendingFormat:@"%@.", self.profileId];
        if (!identity(notice) || ![notice[@"profileId"] isEqual:self.profileId]
            || !text(key, 128) || ![key hasPrefix:prefix] || self.notices[key]) {
            if (error) *error = notificationError(@"Invalid notification admission identity"); return NO;
        }
        self.notices[key] = notice; [self.order addObject:key];
    }
    for (NSDictionary *tap in saved[@"taps"]) {
        NSString *key = [tap isKindOfClass:NSDictionary.class] ? tap[@"id"] : nil;
        NSDictionary *notice = text(key, 128) ? self.notices[key] : nil;
        if (!notice || !identity(tap) || !sameIdentity(notice, tap)) {
            if (error) *error = notificationError(@"Invalid notification tap identity"); return NO;
        }
        [self.taps addObject:tap];
    }
    return YES;
}
- (BOOL)openDirectory:(BOOL)create error:(NSError **)error {
    if (create && !self.writeAllowed) {
        if (error) *error = notificationError(@"Notification checkpoint requires writable admission"); return NO;
    }
    if (self.profile < 0) { if (error) *error = notificationError(@"Notification profile was not admitted"); return NO; }
    if (self.directory >= 0) {
        struct stat retained, named;
        if (fstat(self.directory, &retained) != 0
            || fstatat(self.profile, ".notification-handoff-v1", &named, AT_SYMLINK_NOFOLLOW) != 0
            || !S_ISDIR(named.st_mode) || retained.st_dev != named.st_dev || retained.st_ino != named.st_ino) {
            if (error) *error = notificationError(@"Notification handoff directory changed identity"); return NO;
        }
        return YES;
    }
    if (create) {
        int made = mkdirat(self.profile, ".notification-handoff-v1", 0700);
        if (made != 0 && errno != EEXIST) {
            if (error) *error = notificationError(@"Creating notification handoff directory"); return NO;
        }
        if (made == 0 && fsync(self.profile) != 0) {
            if (error) *error = notificationError(@"Synchronizing notification profile directory"); return NO;
        }
    }
    int directory = openat(self.profile, ".notification-handoff-v1", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    if (directory < 0 && !create && errno == ENOENT) return YES;
    struct stat st, named;
    if (directory < 0 || fstat(directory, &st) != 0 || st.st_uid != geteuid() || (st.st_mode & 0077) != 0
        || fstatat(self.profile, ".notification-handoff-v1", &named, AT_SYMLINK_NOFOLLOW) != 0
        || !S_ISDIR(named.st_mode) || st.st_dev != named.st_dev || st.st_ino != named.st_ino) {
        if (directory >= 0) close(directory);
        if (error) *error = notificationError(@"Notification handoff directory is not private"); return NO;
    }
    self.directory = directory;
    return YES;
}
- (BOOL)save:(NSError **)error {
    if (self.failure) { if (error) *error = self.failure; return NO; }
    if (!self.writeAllowed || ![self openDirectory:YES error:error]) {
        if (error && !*error) *error = notificationError(@"Notification checkpoint is not writable"); return NO;
    }
    NSMutableArray *notices = [NSMutableArray new];
    for (NSString *key in self.order) [notices addObject:self.notices[key]];
    struct stat root;
    if (fstat(self.profile, &root) != 0) { if (error) *error = notificationError(@"Reading retained notification profile identity"); return NO; }
    if (!self.profileId) {
        self.profileId = NSUUID.UUID.UUIDString.lowercaseString;
        self.dirty = YES;
    }
    NSData *data = [NSJSONSerialization dataWithJSONObject:@{@"version":@2, @"profileId":self.profileId,
        @"profileDevice":@((uint64_t)root.st_dev), @"profileInode":@((uint64_t)root.st_ino),
        @"notices":notices, @"taps":self.taps} options:0 error:error];
    if (!data || data.length > JournalLimit) { if (error && !*error) *error = notificationError(@"Notification journal exceeds its bound"); return NO; }
    NSString *temporary = [@".state-" stringByAppendingString:NSUUID.UUID.UUIDString];
    int file = openat(self.directory, temporary.fileSystemRepresentation,
        O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
    if (file < 0) { if (error) *error = notificationError(@"Creating notification checkpoint"); return NO; }
    BOOL ok = writeAll(file, data); close(file);
    if (ok) ok = renameat(self.directory, temporary.fileSystemRepresentation, self.directory, StateName.fileSystemRepresentation) == 0;
    if (ok) ok = fsync(self.directory) == 0;
    if (!ok) {
        unlinkat(self.directory, temporary.fileSystemRepresentation, 0);
        if (error) *error = notificationError(@"Synchronizing notification checkpoint");
        self.failure = error ? *error : notificationError(@"Notification checkpoint failed");
    }
    if (ok) self.dirty = NO;
    return ok;
}
- (NSDictionary *)call:(NSDictionary *)request error:(NSError **)error {
    @synchronized (self) {
        NSString *command = request[@"command"];
        if (!self.validBundle) { if (error) *error = self.failure; return nil; }
        UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
        if ([command isEqual:@"configure"]) {
            if (![request[@"enabled"] isKindOfClass:NSNumber.class] || ![request[@"protected"] isKindOfClass:NSNumber.class]) {
                if (error) *error = notificationError(@"Invalid notification policy"); return nil;
            }
            BOOL enabled = [request[@"enabled"] boolValue], protectedMode = [request[@"protected"] boolValue];
            if (!self.configured || enabled != self.enabled || protectedMode != self.protectedMode) self.epoch++;
            self.enabled = enabled; self.protectedMode = protectedMode; self.configured = YES;
            self.writeAllowed = !protectedMode;
            NSError *bindingError = nil;
            if (![self bindProfile:request error:&bindingError]) {
                self.failure = bindingError; self.enabled = NO; self.protectedMode = YES; self.writeAllowed = NO;
            }
            if (self.enabled && !self.protectedMode && !self.failure) {
                for (NSDictionary *received in self.earlyTaps) {
                    NSDictionary *notice = self.notices[received[@"id"]];
                    BOOL duplicate = [self.taps indexOfObjectPassingTest:^BOOL(NSDictionary *tap, NSUInteger, BOOL *) {
                        return [tap[@"id"] isEqual:received[@"id"]];
                    }] != NSNotFound;
                    if (notice && sameIdentity(notice, received) && !duplicate && self.taps.count < TapLimit) {
                        [self.taps addObject:notice]; self.dirty = YES;
                    }
                }
            }
            [self.earlyTaps removeAllObjects];
            if (!self.enabled || self.protectedMode) {
                NSArray *owned = self.order.copy;
                if (self.taps.count || self.notices.count) self.dirty = YES;
                [self.taps removeAllObjects]; [self.notices removeAllObjects]; [self.order removeAllObjects];
                // Never delete another profile's or another plugin's notices.
                [center removePendingNotificationRequestsWithIdentifiers:owned];
                [center removeDeliveredNotificationsWithIdentifiers:owned];
                NSString *profilePrefix = self.profileId ? [NoticePrefix stringByAppendingFormat:@"%@.", self.profileId] : nil;
                // A delayed removal must not erase a newly admitted notice.
                [center getPendingNotificationRequestsWithCompletionHandler:^(NSArray<UNNotificationRequest *> *requests) {
                    @synchronized (self) {
                        NSMutableArray *keys = [NSMutableArray new];
                        for (UNNotificationRequest *item in requests) {
                            if (profilePrefix && [item.identifier hasPrefix:profilePrefix]
                                && (!self.notices[item.identifier] || !self.enabled || self.protectedMode)) [keys addObject:item.identifier];
                        }
                        [center removePendingNotificationRequestsWithIdentifiers:keys];
                    }
                }];
                [center getDeliveredNotificationsWithCompletionHandler:^(NSArray<UNNotification *> *notifications) {
                    @synchronized (self) {
                        NSMutableArray *keys = [NSMutableArray new];
                        for (UNNotification *item in notifications) {
                            NSString *key = item.request.identifier;
                            if (profilePrefix && [key hasPrefix:profilePrefix]
                                && (!self.notices[key] || !self.enabled || self.protectedMode)) [keys addObject:key];
                        }
                        [center removeDeliveredNotificationsWithIdentifiers:keys];
                    }
                }];
            }
            // Suppression is effective immediately in memory/OS, but protected
            // startup and read-only operation never create or rewrite a journal.
            if (self.failure) { if (error) *error = self.failure; return nil; }
            if (!self.protectedMode && (self.dirty || (self.enabled && !self.profileId)) && ![self save:error]) return nil;
            return @{@"profileId":self.profileId ?: NSNull.null};
        }
        if (self.failure) { if (error) *error = self.failure; return nil; }
        if ([command isEqual:@"peek"]) return @{@"tap": self.enabled && !self.protectedMode && self.taps.count ? self.taps.firstObject : NSNull.null};
        if ([command isEqual:@"ack"]) {
            NSString *key = request[@"id"];
            if (!text(key, 128)) { if (error) *error = notificationError(@"Invalid notification acknowledgment"); return nil; }
            NSIndexSet *indexes = [self.taps indexesOfObjectsPassingTest:^BOOL(NSDictionary *tap, NSUInteger, BOOL *) { return [tap[@"id"] isEqual:key]; }];
            if (!indexes.count) return @{};
            self.dirty = YES;
            [self.taps removeObjectsAtIndexes:indexes];
            return [self save:error] ? @{} : nil;
        }
        if ([command isEqual:@"post"]) {
            if (!self.configured || !self.enabled || self.protectedMode) return @{@"accepted":@NO};
            NSDictionary *notice = request[@"notice"];
            NSString *key = [notice isKindOfClass:NSDictionary.class] ? notice[@"id"] : nil;
            NSString *profilePrefix = self.profileId ? [NoticePrefix stringByAppendingFormat:@"%@.", self.profileId] : nil;
            if (!identity(notice) || ![notice[@"profileId"] isEqual:self.profileId]
                || !text(key, 128) || !profilePrefix || ![key hasPrefix:profilePrefix]
                || !text(request[@"title"], 512) || !text(request[@"body"], 2048)) {
                if (error) *error = notificationError(@"Invalid verified-admission notification"); return nil;
            }
            if (self.notices[key]) {
                if (!sameIdentity(self.notices[key], notice)) { if (error) *error = notificationError(@"Notification identity collision"); return nil; }
                return @{@"accepted":@NO};
            }
            // The service only calls Post for a NEW consumed-receipt transaction.
            // This bounded native history also suppresses duplicate bridge calls.
            while (self.order.count >= NoticeLimit) {
                NSString *oldest = self.order.firstObject;
                if ([self.taps indexOfObjectPassingTest:^BOOL(NSDictionary *tap, NSUInteger, BOOL *) { return [tap[@"id"] isEqual:oldest]; }] != NSNotFound) {
                    if (error) *error = notificationError(@"Acknowledge pending notification opens before adding more"); return nil;
                }
                [self.notices removeObjectForKey:oldest]; [self.order removeObjectAtIndex:0];
            }
            self.notices[key] = notice; [self.order addObject:key];
            self.dirty = YES;
            // Persist before requesting OS delivery: recovery never replays an
            // uncertain attempt and therefore never turns polling into alerts.
            if (![self save:error]) return nil;
            uint64_t epoch = self.epoch;
            UNMutableNotificationContent *content = [UNMutableNotificationContent new];
            content.title = request[@"title"]; content.body = request[@"body"];
            content.userInfo = notice; content.sound = UNNotificationSound.defaultSound;
            UNNotificationRequest *notification = [UNNotificationRequest requestWithIdentifier:key content:content trigger:nil];
            [center getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
                @synchronized (self) {
                    if (epoch != self.epoch || !self.enabled || self.protectedMode || self.failure) return;
                    if (settings.authorizationStatus != UNAuthorizationStatusAuthorized && settings.authorizationStatus != UNAuthorizationStatusProvisional) return;
                    [center addNotificationRequest:notification withCompletionHandler:^(NSError *) {
                        @synchronized (self) {
                            if (epoch != self.epoch || !self.enabled || self.protectedMode || self.failure) {
                                UNUserNotificationCenter *current = UNUserNotificationCenter.currentNotificationCenter;
                                [current removePendingNotificationRequestsWithIdentifiers:@[key]];
                                [current removeDeliveredNotificationsWithIdentifiers:@[key]];
                            }
                            // Denial/revocation/OS delivery errors never undo a
                            // verified library admission or trigger a retry alert.
                        }
                    }];
                }
            }];
            return @{@"accepted":@YES};
        }
        if (error) *error = notificationError(@"Unknown notification operation"); return nil;
    }
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center didReceiveNotificationResponse:(UNNotificationResponse *)response withCompletionHandler:(void (^)(void))completion {
    NSString *key = response.notification.request.identifier;
    if (![key hasPrefix:NoticePrefix]) {
        if ([self.previous respondsToSelector:_cmd]) [self.previous userNotificationCenter:center didReceiveNotificationResponse:response withCompletionHandler:completion];
        else completion();
        return;
    }
    BOOL queued = NO;
    @synchronized (self) {
        NSDictionary *notice = self.notices[key], *received = response.notification.request.content.userInfo;
        if (!self.configured && identity(received) && text(key, 128)
            && [received[@"id"] isEqual:key] && self.earlyTaps.count < TapLimit
            && [response.actionIdentifier isEqual:UNNotificationDefaultActionIdentifier]) {
            // OS metadata is only a bounded pending claim until a Store profile
            // supplies the durable matching admission. Never use it for paths.
            [self.earlyTaps addObject:@{@"id":key, @"profileId":received[@"profileId"],
                @"receipt":received[@"receipt"], @"bookId":received[@"bookId"], @"installationId":received[@"installationId"]}];
            queued = YES;
        }
        if (self.configured && !self.failure && self.enabled && !self.protectedMode && notice && identity(received)
            && sameIdentity(notice, received) && [response.actionIdentifier isEqual:UNNotificationDefaultActionIdentifier]) {
            BOOL duplicate = [self.taps indexOfObjectPassingTest:^BOOL(NSDictionary *tap, NSUInteger, BOOL *) { return [tap[@"id"] isEqual:key]; }] != NSNotFound;
            if (!duplicate && self.taps.count < TapLimit) {
                [self.taps addObject:notice]; self.dirty = YES; NSError *error = nil;
                if (self.writeAllowed && ![self save:&error]) [self.taps removeLastObject];
                else queued = YES;
            }
        }
    }
    // Early cold taps stay in memory until Store admits writable operation;
    // Configure checkpoints them before any trusted-root/JavaScript handoff.
    completion();
    // No native lock or retained Rust context crosses this wakeup. A dropped
    // Service is handled by Rust's weak reference; UIKit completion is released.
    if (queued) {
        KiwixNotificationWaker wake = notificationWaker.load(std::memory_order_acquire);
        if (wake) wake();
    }
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center willPresentNotification:(UNNotification *)notification withCompletionHandler:(void (^)(UNNotificationPresentationOptions))completion {
    if (![notification.request.identifier hasPrefix:NoticePrefix]) {
        if ([self.previous respondsToSelector:_cmd]) [self.previous userNotificationCenter:center willPresentNotification:notification withCompletionHandler:completion];
        else completion(UNNotificationPresentationOptionNone);
        return;
    }
    @synchronized (self) {
        NSDictionary *notice = self.notices[notification.request.identifier];
        BOOL allowed = self.configured && !self.failure && self.enabled && !self.protectedMode && notice
            && identity(notification.request.content.userInfo) && sameIdentity(notice, notification.request.content.userInfo);
        completion(allowed ? (UNNotificationPresentationOptionBanner | UNNotificationPresentationOptionSound) : UNNotificationPresentationOptionNone);
    }
}
- (void)userNotificationCenter:(UNUserNotificationCenter *)center openSettingsForNotification:(UNNotification *)notification {
    if ([self.previous respondsToSelector:_cmd]) [self.previous userNotificationCenter:center openSettingsForNotification:notification];
}
@end

static KiwixNotifications *nativeNotifications;
extern "C" void kiwix_notifications_install_early(void) {
    // Called synchronously by Rust's entry point before UIApplicationMain on iOS.
    // No permission request or asynchronous setup is hidden in installation.
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        nativeNotifications = [KiwixNotifications new];
        // UNUserNotificationCenter asserts for unbundled executables. Retain the
        // unsupported-state error, but never access that API in this process.
        if (!nativeNotifications.validBundle) return;
        UNUserNotificationCenter *center = UNUserNotificationCenter.currentNotificationCenter;
        nativeNotifications.previous = center.delegate;
        center.delegate = nativeNotifications;
    });
}
extern "C" char *kiwix_notifications_call(const char *input) {
    @autoreleasepool {
        NSError *error = nil;
        NSDictionary *request = nil;
        if (input && strlen(input) <= 16384) {
            NSData *data = [NSData dataWithBytes:input length:strlen(input)];
            request = [NSJSONSerialization JSONObjectWithData:data options:0 error:&error];
        }
        NSDictionary *result = nil;
        if (![request isKindOfClass:NSDictionary.class]) error = notificationError(@"Invalid notification request");
        else if ([request[@"command"] isEqual:@"capability"]) {
            KiwixNotifications *state = nativeNotifications;
            if (!state) {
                result = @{@"available":@NO, @"reason":@"Native notifications were not installed for this process"};
            } else {
                // call: publishes validBundle/failure under the delegate lock, so
                // capability must read them there too: failure is a nonatomic
                // strong property and an unsynchronized read can race an assignment.
                @synchronized (state) {
                    BOOL available = state.validBundle && !state.failure;
                    NSString *reason = state.failure.localizedDescription ?: @"Native notifications are unavailable";
                    result = available ? @{@"available":@YES} : @{@"available":@NO, @"reason":reason};
                }
            }
        }
        else if (!nativeNotifications) error = notificationError(@"Notification delegate was not installed before launch");
        else result = [nativeNotifications call:request error:&error];
        NSDictionary *reply = error ? @{@"error":error.localizedDescription} : @{@"value":result ?: @{}};
        NSData *data = [NSJSONSerialization dataWithJSONObject:reply options:0 error:nullptr];
        NSString *encoded = [[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding];
        return strdup(encoded.UTF8String ?: "{\"error\":\"Encoding native notification reply\"}");
    }
}
extern "C" void kiwix_notifications_free(char *value) { free(value); }
extern "C" void kiwix_notifications_request_permission(void *context, KiwixNotificationPermissionCallback callback) {
    dispatch_async(dispatch_get_main_queue(), ^{
        @autoreleasepool {
            KiwixNotifications *state = nativeNotifications;
            if (!state) { callback(context, -1, "Notification delegate was not installed before launch"); return; }
            // Same nonatomic property as capability: read it under the delegate lock.
            BOOL unsupported = NO;
            NSString *unsupportedReason = nil;
            @synchronized (state) {
                unsupported = !state.validBundle;
                if (unsupported) unsupportedReason = state.failure.localizedDescription;
            }
            if (unsupported) { callback(context, -1, unsupportedReason.UTF8String); return; }
#if TARGET_OS_IPHONE
            BOOL active = UIApplication.sharedApplication.applicationState == UIApplicationStateActive;
#else
            BOOL active = NSApplication.sharedApplication.isActive;
#endif
            @synchronized (state) {
                if (!active || state.failure || !state.configured || state.protectedMode || state.permissionInFlight) {
                    callback(context, -1, "Notification permission requires an unprotected active user action"); return;
                }
                state.permissionInFlight = YES;
                uint64_t epoch = state.epoch;
                [UNUserNotificationCenter.currentNotificationCenter requestAuthorizationWithOptions:(UNAuthorizationOptionAlert | UNAuthorizationOptionSound)
                    completionHandler:^(BOOL granted, NSError *error) {
                        @synchronized (state) {
                            state.permissionInFlight = NO;
                            if (epoch != state.epoch || state.protectedMode) { callback(context, 0, nullptr); return; }
                            // Denial is an ordinary opt-in result, never an
                            // acquisition failure or automatic settings loop.
                            if (error && error.code != UNErrorCodeNotificationsNotAllowed) callback(context, -1, error.localizedDescription.UTF8String);
                            else callback(context, granted ? 1 : 0, nullptr);
                        }
                    }];
            }
        }
    });
}
