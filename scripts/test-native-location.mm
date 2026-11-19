// Run on macOS with:
// xcrun clang++ -std=c++17 -fobjc-arc -fblocks -framework Foundation -framework CoreLocation scripts/test-native-location.mm -o /tmp/kiwix-location-test && /tmp/kiwix-location-test
#import <objc/runtime.h>
#include <cstdio>
#include <unordered_map>
#include "../src-tauri/native/location.mm"

static void retire(uint64_t token);

@interface TestLocationManager : CLLocationManager
@property(nonatomic) NSUInteger starts;
@property(nonatomic) NSUInteger stops;
@property(nonatomic) NSUInteger oneShots;
@property(nonatomic) NSUInteger prompts;
@property(nonatomic) CLAuthorizationStatus testAuthorization;
@property(nonatomic) BOOL retireDuringStatusQuery;
@property(nonatomic) uint64_t requestToken;
@end
@implementation TestLocationManager
- (instancetype)init {
    if ((self = [super init])) self.testAuthorization = kCLAuthorizationStatusAuthorizedAlways;
    return self;
}
- (CLAuthorizationStatus)authorizationStatus {
    if (self.retireDuringStatusQuery) retire(self.requestToken);
    return self.testAuthorization;
}
- (void)requestWhenInUseAuthorization { self.prompts++; }
- (void)startUpdatingLocation { self.starts++; }
- (void)stopUpdatingLocation { self.stops++; }
- (void)requestLocation { self.oneShots++; }
@end

struct Result {
    int calls = 0;
    int code = -1;
    double latitude = NAN;
    bool waiting = true;
};
static std::unordered_map<uint64_t, Result> results;
static uint64_t nextToken = 0;
static void retire(uint64_t token) { results.at(token).waiting = false; }
static bool live(uint64_t token) {
    auto found = results.find(token);
    return found != results.end() && found->second.waiting;
}
static void received(uint64_t token, int32_t code, KiwixPosition position, NSData *) {
    auto found = results.find(token);
    if (found == results.end()) return;
    auto &result = found->second;
    result.calls++;
    result.code = code;
    result.waiting = false;
    if (code == 0) result.latitude = position.latitude;
}
static BOOL enabled(id, SEL) { return YES; }
static CLLocation *fix(NSDate *date, double accuracy = 10) {
    return [[CLLocation alloc] initWithCoordinate:CLLocationCoordinate2DMake(37.4, -122.1)
                                        altitude:0 horizontalAccuracy:accuracy
                                verticalAccuracy:10 timestamp:date];
}
static KiwixLocationRequest *begin(TestLocationManager *manager, uint64_t timeout = 1000, bool waiting = true) {
    auto *request = [KiwixLocationRequest new];
    request.token = ++nextToken;
    results[request.token].waiting = waiting;
    request.isLive = live;
    request.callback = received;
    request.manager = manager;
    manager.requestToken = request.token;
    manager.delegate = request;
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
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager);
            auto &result = results.at(request.token);
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
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager);
            auto &result = results.at(request.token);
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date], -1)]];
            [request locationManager:manager didFailWithError:[NSError errorWithDomain:kCLErrorDomain code:kCLErrorLocationUnknown userInfo:nil]];
            CHECK(result.calls == 0 && request.timer.valid, "invalid fixes and transient unknown location must keep acquisition pending");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1 && result.code == 0, "transient provider failure must allow a subsequent fresh fix");
        }
        {
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager);
            auto &result = results.at(request.token);
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
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager, 20);
            auto &result = results.at(request.token);
            [request locationManager:manager didUpdateLocations:@[fix([request.started dateByAddingTimeInterval:-60])]];
            [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.1]];
            CHECK(result.calls == 1 && result.code == 3 && manager.stops == 1,
                  "only stale fixes must expire at the original acquisition deadline");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1, "a fix after timeout must not be delivered");
        }
        {
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager);
            auto &result = results.at(request.token);
            [request locationManager:manager didUpdateLocations:@[fix([request.started dateByAddingTimeInterval:-60])]];
            kiwix_location_cancel(request.token);
            [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
            CHECK(result.calls == 1 && result.code == 1 && manager.stops == 1,
                  "cancellation while awaiting freshness must terminate acquisition");
            [request locationManager:manager didUpdateLocations:@[fix([NSDate date])]];
            CHECK(result.calls == 1, "cancelled requests must not publish late fixes");
        }
        {
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager);
            auto &result = results.at(request.token);
            manager.testAuthorization = kCLAuthorizationStatusDenied;
            [request locationManagerDidChangeAuthorization:manager];
            CHECK(result.calls == 1 && result.code == 1 && manager.stops == 1,
                  "permission revocation must terminate pending acquisition");
        }
        {
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager, 0);
            auto &result = results.at(request.token);
            CHECK(result.calls == 1 && result.code == 3 && manager.starts == 0 && manager.oneShots == 0 && !request.timer,
                  "zero timeout must expire without starting acquisition");
        }
        {
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager);
            auto &result = results.at(request.token);
            [request locationManager:manager didFailWithError:[NSError errorWithDomain:kCLErrorDomain code:kCLErrorNetwork userInfo:nil]];
            CHECK(result.calls == 1 && result.code == 2 && manager.stops == 1,
                  "nontransient provider errors must still be reported");
        }
        {
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager);
            auto &result = results.at(request.token);
            [request locationManager:manager didFailWithError:[NSError errorWithDomain:@"TestDomain" code:kCLErrorDenied userInfo:nil]];
            CHECK(result.calls == 1 && result.code == 2,
                  "another domain's error code must not be treated as denied consent");
        }
        for (CLAuthorizationStatus status : {kCLAuthorizationStatusNotDetermined, kCLAuthorizationStatusAuthorizedAlways}) {
            auto *manager = [TestLocationManager new];
            manager.testAuthorization = status;
            manager.retireDuringStatusQuery = YES;
            auto *request = begin(manager);
            auto &result = results.at(request.token);
            CHECK(result.calls == 1 && result.code == 1 && manager.prompts == 0 && manager.starts == 0 && !request.timer,
                  "retirement during the OS status query must prevent prompting and acquisition");
        }
        {
            auto *manager = [TestLocationManager new];
            auto *request = begin(manager, 1000, false);
            auto &result = results.at(request.token);
            CHECK(result.calls == 1 && result.code == 1 && manager.starts == 0 && !request.timer,
                  "retirement before authorization must reject without acquiring a fix");
        }
        {
            // No location manager may be created once the queued request is retired.
            const uint64_t token = ++nextToken;
            kiwix_location_start(token, true, 30'000, live, received);
            [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.02]];
            CHECK(!activeRequests()[@(token)], "retired start must not enter the provider registry");
        }
        method_setImplementation(services, original);
        CHECK(activeRequests().count == 0, "all requests must release registry ownership");
        std::puts("PASS: stale-to-fresh recovery, invalid/transient recovery, real timeout, cancellation, revocation, zero timeout, terminal error, retired starts, and exactly-once cleanup");
    }
}
