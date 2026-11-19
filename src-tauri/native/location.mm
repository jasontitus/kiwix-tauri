// Main-thread CoreLocation provider for WKWebView, which has no Wry
// geolocation permission backend. Requests use non-reused Rust IDs. Native
// owns CoreLocation objects and passes position fields by value.
#import <Foundation/Foundation.h>
#import <CoreLocation/CoreLocation.h>
#import <TargetConditionals.h>
#include <cmath>
#include <cstdint>

struct KiwixPosition {
    double latitude, longitude, accuracy, altitude;
    double altitude_accuracy, heading, speed, timestamp;
};
static_assert(sizeof(KiwixPosition) == 8 * sizeof(double));
using KiwixLocationIsLive = bool (*)(uint64_t);
using KiwixLocationCallback = void (*)(uint64_t, int32_t, KiwixPosition, NSData *);

@interface KiwixLocationRequest : NSObject <CLLocationManagerDelegate>
@property(nonatomic, strong) CLLocationManager *manager;
@property(nonatomic, strong) NSTimer *timer;
@property(nonatomic, strong) NSDate *started;
@property(nonatomic) uint64_t token;
@property(nonatomic) uint64_t timeoutMs;
@property(nonatomic) KiwixLocationIsLive isLive;
@property(nonatomic) KiwixLocationCallback callback;
@property(nonatomic) BOOL completed;
@property(nonatomic) BOOL requesting;
- (void)authorizeOrStart;
- (void)finish:(int32_t)code position:(KiwixPosition)position message:(NSString *)message;
@end

static NSMutableDictionary<NSNumber *, KiwixLocationRequest *> *requests;
static NSMutableDictionary<NSNumber *, KiwixLocationRequest *> *activeRequests() {
    // All access, including initialization, is confined to the main queue.
    if (!requests) requests = [NSMutableDictionary new];
    return requests;
}

@implementation KiwixLocationRequest
- (void)finish:(int32_t)code position:(KiwixPosition)position message:(NSString *)message {
    if (self.completed) return;
    self.completed = YES;
    [self.timer invalidate];
    self.timer = nil;
    [self.manager stopUpdatingLocation];
    self.manager.delegate = nil;
    self.manager = nil;
    KiwixLocationCallback callback = self.callback;
    self.callback = nullptr;
    uint64_t token = self.token;
    [activeRequests() removeObjectForKey:@(token)];
    if (callback) {
        NSData *diagnostic = [[message dataUsingEncoding:NSUTF8StringEncoding] copy];
        callback(token, code, position, diagnostic);
    }
}

- (BOOL)hasLiveWaiter {
    if (self.isLive(self.token)) return YES;
    [self finish:1 position:KiwixPosition{} message:@"Archive location request was cancelled or revoked"];
    return NO;
}

- (void)authorizeOrStart {
    if (self.completed || self.requesting || ![self hasLiveWaiter]) return;
    if (![CLLocationManager locationServicesEnabled]) {
        [self finish:1 position:KiwixPosition{} message:@"System Location Services are disabled"];
        return;
    }
    CLAuthorizationStatus status = self.manager.authorizationStatus;
    // CoreLocation queries can run while a Rust worker retires the request.
    // Check again before handing any new work to the OS.
    if (![self hasLiveWaiter]) return;
    switch (status) {
        case kCLAuthorizationStatusNotDetermined:
            [self.manager requestWhenInUseAuthorization];
            break;
        case kCLAuthorizationStatusDenied:
            [self finish:1 position:KiwixPosition{} message:@"The system denied location access; change Location Services settings to allow it"];
            break;
        case kCLAuthorizationStatusRestricted:
            [self finish:1 position:KiwixPosition{} message:@"Location access is restricted by system policy"];
            break;
        case kCLAuthorizationStatusAuthorizedAlways:
#if TARGET_OS_IPHONE
        case kCLAuthorizationStatusAuthorizedWhenInUse:
#endif
            self.requesting = YES;
            // Permission waits are not part of the position-acquisition timeout.
            // A zero timeout must expire without starting GPS acquisition.
            if (self.timeoutMs == 0) {
                [self finish:3 position:KiwixPosition{} message:@"Location request timed out"];
                return;
            }
            self.started = [NSDate date];
            // The registry retains the request; the timer must not form a cycle.
            __weak KiwixLocationRequest *weakRequest = self;
            self.timer = [NSTimer timerWithTimeInterval:(double)self.timeoutMs / 1000.0
                                               repeats:NO block:^(NSTimer *timer) {
                (void)timer;
                [weakRequest finish:3 position:KiwixPosition{} message:@"Location request timed out"];
            }];
            [[NSRunLoop mainRunLoop] addTimer:self.timer forMode:NSRunLoopCommonModes];
            if ([self hasLiveWaiter]) [self.manager startUpdatingLocation];
            break;
    }
}

- (void)locationManagerDidChangeAuthorization:(CLLocationManager *)manager {
    (void)manager;
    // A system revocation while a fix is pending must also terminate it.
    CLAuthorizationStatus status = self.manager.authorizationStatus;
    if (status == kCLAuthorizationStatusDenied || status == kCLAuthorizationStatusRestricted) {
        [self finish:1 position:KiwixPosition{} message:@"System location permission was revoked"];
    } else {
        [self authorizeOrStart];
    }
}

- (void)locationManager:(CLLocationManager *)manager didUpdateLocations:(NSArray<CLLocation *> *)locations {
    (void)manager;
    if (self.completed || !self.requesting) return;
    CLLocation *location = locations.lastObject;
    // CoreLocation can deliver cached or invalid fixes before acquiring a fresh
    // position. Keep updates running under the original acquisition deadline;
    // maximumAge caching belongs to the JavaScript bridge.
    double timestamp = location ? location.timestamp.timeIntervalSince1970 : NAN;
    double now = NSDate.date.timeIntervalSince1970;
    if (!location || !CLLocationCoordinate2DIsValid(location.coordinate) ||
        !std::isfinite(location.horizontalAccuracy) || location.horizontalAccuracy < 0 ||
        !std::isfinite(timestamp) || timestamp > now + 1.0 ||
        [location.timestamp timeIntervalSinceDate:self.started] < -1.0) return;
    KiwixPosition position {
        location.coordinate.latitude, location.coordinate.longitude,
        location.horizontalAccuracy,
        location.verticalAccuracy >= 0 ? location.altitude : NAN,
        location.verticalAccuracy,
        location.course >= 0 && location.speed > 0 ? location.course : NAN,
        location.speed,
        [location.timestamp timeIntervalSince1970] * 1000.0,
    };
    [self finish:0 position:position message:@""];
}

- (void)locationManager:(CLLocationManager *)manager didFailWithError:(NSError *)error {
    (void)manager;
    BOOL locationError = [error.domain isEqualToString:kCLErrorDomain];
    if (locationError && error.code == kCLErrorLocationUnknown) return;
    [self finish:(locationError && error.code == kCLErrorDenied ? 1 : 2)
          position:KiwixPosition{} message:error.localizedDescription ?: @"Location provider failed"];
}
@end

extern "C" void kiwix_location_start(uint64_t token, bool highAccuracy, uint64_t timeoutMs,
                                     KiwixLocationIsLive isLive, KiwixLocationCallback callback) {
    dispatch_async(dispatch_get_main_queue(), ^{
        @autoreleasepool {
            // A dropped waiter cannot start a delayed OS authorization prompt.
            if (!isLive(token) || activeRequests()[@(token)]) return;
            KiwixLocationRequest *request = [KiwixLocationRequest new];
            request.token = token;
            request.isLive = isLive;
            request.callback = callback;
            request.timeoutMs = MIN(timeoutMs, (uint64_t)30000);
            activeRequests()[@(token)] = request;
            request.manager = [CLLocationManager new];
            request.manager.desiredAccuracy = highAccuracy ? kCLLocationAccuracyBest : kCLLocationAccuracyHundredMeters;
            request.manager.delegate = request;
            [request authorizeOrStart];
        }
    });
}

extern "C" void kiwix_location_cancel(uint64_t token) {
    dispatch_async(dispatch_get_main_queue(), ^{
        KiwixLocationRequest *request = activeRequests()[@(token)];
        [request finish:1 position:KiwixPosition{} message:@"Archive location request was cancelled or revoked"];
    });
}
