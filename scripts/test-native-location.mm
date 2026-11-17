// Run on macOS with:
// xcrun clang++ -std=c++17 -fobjc-arc -fblocks -framework Foundation -framework CoreLocation scripts/test-native-location.mm -o /tmp/kiwix-location-test && /tmp/kiwix-location-test
#import <objc/runtime.h>
#include <cstdio>
#include "../src-tauri/native/location.mm"

@interface TestLocationManager : CLLocationManager
@property(nonatomic) NSUInteger starts;
@property(nonatomic) NSUInteger stops;
@property(nonatomic) NSUInteger oneShots;
@property(nonatomic) CLAuthorizationStatus testAuthorization;
@end
@implementation TestLocationManager
- (CLAuthorizationStatus)authorizationStatus { return self.testAuthorization; }
- (void)startUpdatingLocation { self.starts++; }
- (void)stopUpdatingLocation { self.stops++; }
- (void)requestLocation { self.oneShots++; }
@end

struct Result {
    int calls = 0;
    int code = -1;
    double latitude = NAN;
};
static void received(void *context, int32_t code, const KiwixPosition *position, const char *) {
    auto *result = static_cast<Result *>(context);
    result->calls++;
    result->code = code;
    if (position) result->latitude = position->latitude;
}
static BOOL enabled(id, SEL) { return YES; }
static CLLocation *fix(NSDate *date, double accuracy = 10) {
    return [[CLLocation alloc] initWithCoordinate:CLLocationCoordinate2DMake(37.4, -122.1)
                                        altitude:0 horizontalAccuracy:accuracy
                                verticalAccuracy:10 timestamp:date];
}
static KiwixLocationRequest *begin(Result &result, TestLocationManager *manager, uint64_t timeout = 1000) {
    static uint64_t token = 0;
    auto *request = [KiwixLocationRequest new];
    request.token = ++token;
    request.context = &result;
    request.callback = received;
    request.manager = manager;
    manager.delegate = request;
    manager.testAuthorization = kCLAuthorizationStatusAuthorizedAlways;
    request.timeoutMs = timeout;
    activeRequests()[@(request.token)] = request;
    [request authorizeOrStart];
    return request;
}
#define CHECK(condition, message) do { if (!(condition)) { std::fprintf(stderr, "FAIL: %s\n", message); return 1; } } while (0)

int main() {
    @autoreleasepool {
        Method services = class_getClassMethod([CLLocationManager class], @selector(locationServicesEnabled));
        IMP original = method_setImplementation(services, reinterpret_cast<IMP>(enabled));
        // Only the hardware boundary is substituted; production authorization,
        // delegate transitions, timer, cancellation, and cleanup execute below.
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager);
            auto *timer = request.timer;
            [request locationManager:manager didUpdateLocations:@[fix([request.started dateByAddingTimeInterval:-60])]];
            CHECK(result.calls == 0 && !request.completed && timer.valid,
                  "stale initial fix must remain pending until a fresh fix or deadline");
            CHECK(manager.starts == 1 && manager.oneShots == 0, "acquisition must continue beyond a cached first fix");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1 && result.code == 0 && result.latitude == 37.4,
                  "fresh fix after stale fix must succeed");
            CHECK(manager.stops == 1 && !timer.valid && manager.delegate == nil && !activeRequests()[@(request.token)],
                  "success must stop acquisition and release timer, delegate, and registry ownership");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            [request locationManager:manager didFailWithError:[NSError errorWithDomain:kCLErrorDomain code:kCLErrorDenied userInfo:nil]];
            CHECK(result.calls == 1, "late provider callbacks must not complete twice");
        }
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager);
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date], -1)]];
            [request locationManager:manager didFailWithError:[NSError errorWithDomain:kCLErrorDomain code:kCLErrorLocationUnknown userInfo:nil]];
            CHECK(result.calls == 0 && request.timer.valid, "invalid fixes and transient unknown location must keep acquisition pending");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1 && result.code == 0, "transient provider failure must allow a subsequent fresh fix");
        }
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager);
            [request locationManager:manager didUpdateLocations:@[fix([NSDate dateWithTimeIntervalSinceNow:60])]];
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date], NAN)]];
            auto *invalid = [[CLLocation alloc] initWithCoordinate:CLLocationCoordinate2DMake(91, -122)
                                                        altitude:0 horizontalAccuracy:10 verticalAccuracy:10
                                                        timestamp:[NSDate date]];
            [request locationManager:manager didUpdateLocations:@[invalid]];
            CHECK(result.calls == 0 && request.timer.valid,
                  "future and malformed fixes must not terminate fresh acquisition");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1 && result.code == 0,
                  "a valid fresh fix must recover from malformed provider updates");
        }
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager, 20);
            [request locationManager:manager didUpdateLocations:@[fix([request.started dateByAddingTimeInterval:-60])]];
            [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
            CHECK(result.calls == 1 && result.code == 3 && manager.stops == 1,
                  "only stale fixes must expire at the original acquisition deadline");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1, "a fix after timeout must not be delivered");
        }
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager);
            [request locationManager:manager didUpdateLocations:@[fix([request.started dateByAddingTimeInterval:-60])]];
            kiwix_location_cancel(request.token);
            [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
            CHECK(result.calls == 1 && result.code == 1 && manager.stops == 1,
                  "cancellation while awaiting freshness must terminate acquisition");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1, "cancelled requests must not publish late fixes");
        }
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager);
            manager.testAuthorization = kCLAuthorizationStatusDenied;
            [request locationManagerDidChangeAuthorization:manager];
            CHECK(result.calls == 1 && result.code == 1 && manager.stops == 1,
                  "permission revocation must terminate pending acquisition");
        }
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager, 0);
            CHECK(result.calls == 1 && result.code == 3 && manager.starts == 0 && manager.oneShots == 0 && !request.timer,
                  "zero timeout must expire without starting acquisition");
        }
        {
            Result result;
            auto *manager = [TestLocationManager new];
            auto *request = begin(result, manager);
            [request locationManager:manager didFailWithError:[NSError errorWithDomain:kCLErrorDomain code:kCLErrorNetwork userInfo:nil]];
            CHECK(result.calls == 1 && result.code == 2 && manager.stops == 1,
                  "nontransient provider errors must still be reported");
        }
        method_setImplementation(services, original);
        CHECK(activeRequests().count == 0, "all requests must release registry ownership");
        std::puts("PASS: stale-to-fresh recovery, invalid/transient recovery, real timeout, cancellation, revocation, zero timeout, terminal error, and exactly-once cleanup");
    }
}
