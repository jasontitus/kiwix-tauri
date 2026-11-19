# Bonjour discovery callback ownership

In the Apple discovery worker at `7a56109`, DNS-SD receives an operation number in its opaque context argument. The number does not point to Rust data. It is unique across workers and cannot wrap. Rust accepts a callback only while it is creating or processing that exact operation on the same thread. Multiple address records registered on one DNS-SD connection share that connection’s number. Browse, watch, resolve and service registration receive their own numbers.

A callback with a retired or unknown number returns before reading its payload or charging the discovery quota. When a nested operation finishes, the outer operation regains its scope. A synchronous error queued during creation is processed after the worker finishes registering or rolling back the operation. Removing a failed registration prevents its events from reaching a subsequent announcement of the same service.

The worker still calls native DNS-SD for its service references and processes borrowed strings and TXT bytes returned by the operating system. The Objective-C++ connection resolver has its own separate DNS-SD callbacks.

## Behavior tested

Independent checks exercised matching and stale events, concurrent workers, nested scope restoration, permission denial, traffic limits, allocation exhaustion and rollback after partial multi-record registration. All 14 tests in the controlled fixture passed with compiler warnings treated as errors.

On macOS, two real discovery workers used the system Bonjour service to browse and publish a temporary library. A reader observed the expected session, port, item count, generation and display name. After withdrawal it observed disappearance. The publisher reopened the same service at a new generation, and a stale withdrawal left that generation discoverable. The reader stopped and restarted browsing, found the new generation, and observed its final withdrawal. Both workers shut down. This exercise passed once using a system-host registration and once using an explicit IPv4 address record. The harness ran the production discovery engine with stand-ins for the outer protocol and routing helpers. It shared no archive files. The complete optimized Mac app test executable also passed its library restart and stale-withdrawal regression and four live Apple network regressions.

## Local processing cost

The actual callback code ran against the same C replay fixture on both builds. The timed interval included decoding, adding to the bounded queue, draining it and checking each event. The Mac ran 16 alternating pairs per case after a warm-up pair. Across 192 measured processes, the fixture checked 6,291,456 events. The table shows the median per-process nanoseconds per event. The final column compares matched pairs before taking the median. Other work shared the machine. [Individual timings and binary hashes](2026-11-18-bonjour-callbacks.json)

| Event | Events per native call | Previous (ns) | With operation numbers (ns) | Paired change |
|---|---:|---:|---:|---:|
| Watch | 1 | 83.99 | 90.15 | +6.65% |
| Watch | 32 | 88.05 | 89.87 | +1.44% |
| Resolve, 32-byte TXT | 1 | 419.41 | 440.99 | +5.28% |
| Resolve, 32-byte TXT | 32 | 400.19 | 402.78 | +1.81% |
| Resolve, 2,048-byte TXT | 1 | 718.08 | 725.74 | +1.63% |
| Resolve, 2,048-byte TXT | 32 | 716.27 | 714.01 | +0.70% |

For a single 32-byte resolve event, the observed difference was about 22 ns. Operation creation, operating-system discovery latency, network transfer and user-interface work were outside the timer.

## iOS simulator discovery

Using ordinary system interface selection, both revisions discovered the first registration of a temporary library and withdrew it. After browsing restarted, the reader discovered generation 2 and retained it after a stale withdrawal. An independent DNS query observed its explicit IPv4 address record being added and removed. Both workers shut down. This test used an ephemeral socket on an assigned address and shared no archive contents.

A separate local-only control exposed a difference in system registration behavior. The simulator’s default local-only host was `localhost.`, which both revisions rejected under the existing requirement for a name ending in `.local.`. An explicitly named local-only host passed on both versions, including independent observations of two address records and their removal.

A separate timing fixture replayed callbacks locally in the simulator. One callback per scope took 44.12 ns before and 47.09 ns after the change. For 16 callbacks per scope it took 27.50 and 27.48 ns per event. The median within-pair changes were +2.75% and +1.23% respectively. Each of the 32 measured processes verified 262,144 events. These times include entering and leaving the scope and enqueueing the event. Dequeue, event-field verification and OS discovery were outside the timer. Physical-device discovery remains unverified. [Individual simulator timings and live probe identities](2026-11-18-ios-bonjour-callbacks.json)

## Outgoing Apple connection resolution

At `e46993d`, the native connection resolver supplies a fresh operation number for each service and address lookup. The serial callback queue finds a weak connection reference by number and checks the active stage. Successful callbacks also have to match the SDK reference. Retirement removes the number before deallocating the reference. A late callback cannot act on a replacement, including when the SDK reuses the same reference address. Service-to-address handoff preserves the route fallback timer. The resolver still reads SDK-owned hostnames and addresses and uses native DNS-SD and Network.framework.

On macOS the optimized and sanitizer fixtures covered failed creation, failed queue assignment, stale callbacks, reused references, cancellation and exhaustion. Independent checks exercised 4,096 callbacks during deallocation and eight concurrent lookups for the local host through the real DNS service. The app passed all four live Nearby networking regressions. They include a real Bonjour and QUIC library request, route selection and concurrent stream retirement. On the iOS simulator, the probe resolved a temporary service to its bound private address and port, rejected an unexpected hostname, advanced past an unpublished route and found a late registration. It withdrew its temporary records. The simulator probe stopped before TLS and QUIC.

| Measurement | Previous | With operation numbers | Coverage |
|---|---:|---:|---|
| Mac callback owner lookup, one active operation | 14.61 ns | 14.47 ns | 12 pairs, 32,768 lookups per process |
| Mac callback owner lookup, 32 active operations | 10.57 ns | 10.86 ns | Same local fixture |
| Mac library request and route test | 1,209 ms | 1,267 ms | Whole process, including publisher setup, real QUIC and cleanup |
| iOS simulator DNS route selection | 4.99 ms | 4.66 ms | Pooled median of 96 checked selections per version |

The Mac library test used ten alternating fresh-process pairs after two warm-up pairs. Its median within-pair elapsed time increased 3.8%. The simulator used six alternating process pairs, each with one discarded warm-up and 16 measured selections. Its median within-pair change was −8.6%. These measure different parts of a request. Other workloads shared the Mac, and physical radio performance remains open. [Individual timings and binary identities](2026-11-18-apple-route-queries.json)
