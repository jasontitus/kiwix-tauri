# Our implementation versus existing Kiwix apps — updated 2026-11-05

This replaces the earlier, wrong-scope comparison of our own platform targets. The baselines are **Kiwix Apple 3.18.0**, **Kiwix Android 3.14.1 standalone ARM64**, and **Qt Kiwix Desktop 2.5.1**. Our binaries are optimized local release builds, not published store releases. Source feature coverage and exercised behavior remain separate.

**Current result:** the latest controlled Android artifact (`e74be9a5…`) is **4.56% smaller** than official ARM64 Kiwix. Cold-reader results are workload-dependent: the larger real Wikipedia uses **250.54 versus 238.39 MiB total PSS (+5.09%)**, while the larger California StreetZIM uses **304.89 versus 311.51 MiB (−2.13%)**. After 20 larger-Wikipedia title-query/article-viewing cycles, ours uses **321.72 / 318.66 MiB** versus official's **404.15 / 385.27 MiB** in two paired runs, with lower growth from each prepared baseline. Ours still grows with activity; fixed overhead is not established. Four map sessions also ended lower for ours, but their incremental-growth comparisons reversed sign. This test artifact includes reviewed but uncommitted PDF/preview changes and retains known PDF layout/recovery failures; it is not release acceptance. Earlier source-backing, APK, emulator, search and Mac results retain their separate scopes below; no overall memory/speed win or completed iPhone comparison is claimed.

The controlled artifact freezes `d49a472928da2b43dde2f3e5fa47fe78ab255c20` plus five reviewed reader-export/preview overlays, with exact source hashes in the [larger-archive receipt](benchmarks/2026-11-05-android-larger-archives.json). The preceding default idle-client/storage-capture APK (`7d45d21e…`) freezes `8002bff8971b4ceec74772e11d84e63b02d3fb5e` plus `storage-bridge.js`, `acquisition.rs` and `maps.rs`. Profiler commits are not product source identities. Artifacts and protocols remain separate: no pooling or individual-change causal saving is inferred.

## Current app-level measurements — separate frozen artifacts

### Larger real Wikipedia — cold-reader memory, not activity growth

Both Android apps opened the same **2,223,060,407-byte Best of Wikipedia**
archive, dated 2026-03-18, on the actual Animal article. Host and phone
SHA-256 matched: `3eaaa2765ad11d0f6fcd424f9bc14c5489b572b488564d75cdced9410fb7e0cd`.
The archive contains 923,593 entries; its Animal HTML is 494,043 bytes.
Each app had one reader tab. The unchanged `8217a32a…` profiler completed
eight ABBAABBA process-cold launches, 32 samples total, in **369.3 seconds**,
with the existing 30-second settling and all-owned-process accounting.

| Independent median, MiB | Ours | Official ARM64 |
| --- | ---: | ---: |
| Main + owned renderer TOTAL PSS | **250.54** | **238.39** |
| Main process PSS | 148.55 | 157.75 |
| Renderer PSS | 101.98 | 80.39 |
| Native heap PSS | 40.93 | 47.27 |

The total gap is **+12.15 MiB (+5.09%)**; adjacent paired block differences
are **+9.90, +12.17, +11.37 and +14.02 MiB**. All favor official.
Component medians are independent and are not additive.

A preceding separate run of the same APK on the 333,429,922-byte Wikipedia
100 Animal article completed 32 samples in 376.8 seconds: **241.93 versus
233.99 MiB**, a **+7.94 MiB (+3.39%)** gap. Do not attribute the wider gap
solely to archive size: the later library contains four rather than two
archives, and the editions/resources differ; the larger edition is nopic.
These observations do not establish a fixed overhead, a leak, peak memory,
or how memory scales with repeated reading/searching.

The [receipt](benchmarks/2026-11-05-android-larger-archives.json) retains all
64 Wikipedia samples, process-generation ownership, raw meminfo hashes, boundary-image
agreement and installed APK identities. No samples were excluded.

### Larger real Wikipedia — lower memory after repeated title-query/article viewing

The same APKs and larger archive completed **four uninterrupted sessions**,
in **ours → official → official → ours** order. Each session counted 20
predefined title-query / first-displayed-result / article-top viewing cycles.
After rendering was admitted, each article had a two-second dwell and a
second readiness check. There was no vertical scrolling. The apps returned
to Animal after cycles 5, 10 and 20, then settled for 30 seconds before each
four-sample checkpoint.

Checkpoint zero is **after archive selection and Animal warmup**, not
untouched cold-start memory. The Animal warmup and three return searches
are additional to the 20 counted primary cycles. Each app had one reader
tab and the same four-archive library inventory.

| Pair | Completed primary cycles | Ours total PSS, MiB | Official total PSS, MiB |
| --- | ---: | ---: | ---: |
| 1 | 0 | 254.72 | 272.43 |
| 1 | 5 | 305.93 | 315.91 |
| 1 | 10 | 302.66 | 319.18 |
| 1 | 20 | **321.72** | **404.15** |
| 2 | 0 | 254.17 | 259.64 |
| 2 | 5 | 295.44 | 323.31 |
| 2 | 10 | 303.19 | 339.75 |
| 2 | 20 | **318.66** | **385.27** |

At cycle 20, ours used **82.44 / 66.62 MiB less**. Growth from each app's
own prepared baseline was **67.00 / 64.48 MiB ours** versus **131.72 /
125.63 MiB official**. Thus ours-minus-official growth favored ours by
**64.72 / 61.15 MiB** at the endpoint. These are two paired observations;
the four adjacent samples at each checkpoint are not independent sessions.

This is a genuine bounded workload advantage, **not fixed overhead or a
plateau**: ours added another **15.79 / 23.22 MiB between cycles 5 and 20**.
Our renderer PSS grew **58.69 / 57.10 MiB** from checkpoint zero. Official's
main-process PSS grew **133.78 / 112.13 MiB**, while its native-heap PSS
grew **112.70 / 112.79 MiB**. Component medians are independent and not
additive; PSS does not identify retained JavaScript objects or prove leaks.
No source optimization occurred between these runs.

Both apps opened the same observed headings for all 20 queries, but this
was not exact-title retrieval: `Computer` selected the first `.computer`
result, rendering **List of Internet top-level domains**, and `Mathematics`
rendered **Exponentiation**. Ours explicitly submits Enter; official uses
displayed title suggestions. The comparison therefore matches a user
workload, not identical search-engine calls. It establishes neither search
latency nor full-article reading performance.

The [receipt](benchmarks/2026-11-05-android-wikipedia-activity.json) retains
all 64 samples, actual query-to-heading mappings, visually admitted reference
hashes, per-process evidence and paired growth calculations. All four
sessions completed with continuous main/renderer generations, unchanged
APK identities and successful stop-both/Home cleanup. An earlier preparation
selected the smaller Wikipedia100 article and failed the unchanged reference
gate before any admitted sample; it remains excluded. Four seconds of
startup dwell were then applied to both corrected plans. Historical
`UNVERIFIED` reference filenames are accompanied by the parent's subsequent
visual-admission record, not treated as unreviewed proof.

Official's temporary All Files Access was restored to **OFF** through
Android Settings after measurement; read-only appops inspection confirmed
the original UID-deny/default package state. No iPhone or speech services
were used.

### Larger California StreetZIM — lower cold-map PSS in this cohort

Both apps opened the same **2,280,435,024-byte Kalifornien** archive, dated
2026-10-13, with 542,818 entries. Host and phone SHA-256 matched:
`d3fcd070f15e2259a95a629362b243fd96d26b9da2adcb8745feff3edd8ca579`.
The actual embedded map rendered the San Francisco Bay / Sacramento region.
Each app had one reader tab and the same four-archive library inventory;
native and web-shell map viewport geometry differs.

The unchanged profiler completed **eight ABBAABBA process-cold launches,
32 samples total, in 371.6 seconds**, with the same 30-second settling.

| Independent median, MiB | Ours | Official ARM64 |
| --- | ---: | ---: |
| Main + owned renderer TOTAL PSS | **304.89** | **311.51** |
| Main process PSS | 196.36 | 211.55 |
| Renderer PSS | 108.73 | 101.26 |
| Native heap PSS | 49.58 | 51.53 |

The total difference is **−6.63 MiB (−2.13%)**, favoring ours. Adjacent
paired block differences are **−7.45, −6.47, −1.04 and −14.61 MiB**.
All four favor ours, but their spread and context-sensitive shared mappings
remain visible. This is not an overall app advantage or repeated-activity
stability result.

An initial preparation run was rejected after four ours samples and before
any official sample: the native cold-restored map differed from its earlier
live-navigation reference, **MAD 8.420 > 5.0**. The replacement reference was
the visually inspected actual cold-restored native map. The threshold was
not relaxed. The [receipt](benchmarks/2026-11-05-android-larger-archives.json)
keeps that rejected run separate from the complete 32-sample cohort; no
cross-run pooling or selective sample deletion was used.

### Larger StreetZIM — four uninterrupted activity sessions

The same frozen APKs completed two matched pairs, in **official → ours →
ours → official** order, without restarting either app between its own
checkpoints. Each session performed 20 prescribed horizontal map cycles;
all 64 samples were admitted. Main and renderer process generations remained
continuous, installed APK identities were unchanged, and all sessions
finished with both apps stopped and the launcher foreground.

| Pair | Completed cycles | Ours total PSS, MiB | Official total PSS, MiB |
| --- | ---: | ---: | ---: |
| 1 | 0 | 326.78 | 329.95 |
| 1 | 5 | 361.95 | 363.72 |
| 1 | 10 | 343.81 | 366.34 |
| 1 | 20 | **343.59** | **366.17** |
| 2 | 0 | 317.60 | 346.77 |
| 2 | 5 | 354.00 | 369.17 |
| 2 | 10 | 354.53 | 368.78 |
| 2 | 20 | **354.55** | **368.61** |

Each cell is the median of four samples from one session checkpoint, not
four independent trials. At cycle 20, ours was **22.58 / 14.06 MiB lower**
in the two pairs. Growth relative to each app's own checkpoint zero tells
a less favorable story: ours minus official growth was **−19.41 / +15.11
MiB**. The sign reversal does not establish consistently lower incremental
memory use.

In our first session, the large cycle-5-to-10 PSS drop coincided with
mapped APK/shared-library PSS falling from **99.48 to 83.12 MiB**, while
native heap and Unknown PSS were nearly unchanged. Do not present that
shared-mapping change as an app optimization or heap saving.

Between cycles 5 and 20, ours changed **−18.36 / +0.55 MiB** and official
changed **+2.45 / −0.56 MiB**. No large continuing late rise was observed
over this bounded workload; this is not proof against a leak, a larger
working set, or accumulation from visiting distinct regions/articles.

The [activity receipt](benchmarks/2026-11-05-android-map-activity.json)
retains all checkpoint samples, generation checks, raw-evidence hashes,
both paired growth calculations and the separately excluded exploratory
and rejected protocols. The first pair used the reviewed private helper;
the second used the checked-in portable helper with unchanged accounting
and equivalent actions. Both used theme Auto. Map viewport geometry differs;
metric units were observed in ours and imperial units in official during
preparation and were not normalized or rechecked at each checkpoint.
This is a prescribed horizontal-pan/Home comparison, not identical tile
coverage, vertical toolbar behavior, search speed or rendering throughput.

### Repeating uninterrupted activity measurements

[`profile-android-activity-memory.py`](../scripts/profile-android-activity-memory.py)
uses the existing profiler's accounting but launches only once per session.
It measures after 0, 5, 10 and 20 prescribed cycles, returning to the trusted
starting view and settling 30 seconds before each four-sample checkpoint.
It records process-generation changes rather than treating a restart as a
memory saving. A rejected/incomplete session is not a completed trajectory.

The checked-in California [ours](benchmarks/2026-11-05-california-plan-ours.json)
and [official](benchmarks/2026-11-05-california-plan-official.json) plans are
**1080×2340 Samsung fixtures**, not a general UI recognizer. They perform
three zoom increments, an alternating horizontal pan of increasing distance,
and Home. They do not exercise search or native vertical toolbar hiding.
Only Home endpoints are admitted; equal tile counts or 20 distinct regions
are not established. Other geometry/app versions require an observed plan
and locally verified references, not a relaxed readiness threshold.

The reference PNGs mask every pixel outside the comparison crop, removing
status/notification/navigation regions without changing any compared pixels.
Map screenshots contain OpenStreetMap/OpenMapTiles content:
© [OpenStreetMap contributors](https://www.openstreetmap.org/copyright)
and [OpenMapTiles](https://openmaptiles.org/).

With the exact archive already open in one reader tab, run one session:

```sh
python3 scripts/profile-android-activity-memory.py \
  --adb "$ANDROID_HOME/platform-tools/adb" --serial "$ANDROID_SERIAL" \
  --model SM-A356U --alias ours \
  --plan docs/benchmarks/2026-11-05-california-plan-ours.json \
  --reference docs/benchmarks/2026-11-05-california-reference-ours.png \
  --output /tmp/kiwix-activity-ours-NEW
```

Choose a new output directory. Run the matching official session, then reverse
the order for a second pair. Compare each app's growth from its own checkpoint
zero, then subtract official growth from ours. The nearby four samples are
not four independent sessions. No cache clearing, forced GC, app replacement
or process restart is permitted between checkpoints.

### Build-cache cleanup

At the owner's request, seven obsolete generated project Cargo targets and
twelve verified assistant-owned temporary Cargo targets were removed. Actual
filesystem free space increased **53.27 GiB**, from **28.40 to 81.67 GiB**.
The project measured **112.40 GiB** after the project-target cleanup.
APFS clones share extents, so summed `du` sizes are not independent physical
allocations. Source, ZIMs, installed SDKs, benchmark evidence and comparison
APKs were preserved; this cleanup is not an app-size or memory optimization.


### Application-package size experiment — smaller artifact, no runtime win

The application package alone was rebuilt with Rust `opt-level=s`, one codegen
unit and explicit `strip=symbols`; its local C++ archive bridge used `-Os`.
Dependencies retained optimization level 3 and the native search SDK was
unchanged. Sources, features, locales, frontend and Android manifest matched.
The [controlled receipt](benchmarks/2026-11-05-android-root-size.json) retains
both measured cohorts and the separate first confounded artifact.

| Artifact | Signed APK bytes | Disposition |
| --- | ---: | --- |
| Default `7d45d21e…` | 67,145,771 | Retained and restored |
| First trial `aac90c65…` | 82,137,131 | Rejected before installation: stripping unexpectedly changed to `debuginfo` |
| Strip-controlled trial `55de4378…` | 66,400,299 | Size saving verified; default profile not changed |

The corrected trial saves **745,472 APK bytes** and **749,344 native ELF bytes**,
placing it 5.635903% below the official APK. The first trial retained `.symtab`
and `.strtab`; its extra bytes are not evidence that size optimization enlarged
runtime code. The corrected build explicitly rejected those static tables and
preserved dynamic JNI exports, signer and 16 KiB alignment.

The unchanged profiler ran separate **eight-block ABBAABBA startup-enabled**
cohorts: 406.333 seconds for the default and 408.513 seconds for the corrected
trial. Each retained 32 PSS samples and four startup observations per app.
PSS settling starts after admitted crop readiness and launch completion, unlike
the standard-mode run below. No cohort or artifact is pooled.

| Startup-enabled cohort | Ours PSS, MiB | Official PSS, MiB | Ours median readiness bounds, s | Official median readiness bounds, s |
| --- | ---: | ---: | --- | --- |
| Default `7d45d21e…` | 247.788574 | 246.806641 | [0.931030, 2.213825] | [0.356023, 2.135596] |
| Strip-controlled `55de4378…` | 250.085938 | 245.116211 | [1.087150, 2.374054] | [0.011142, 1.983636] |

Total PSS is unfavorable by **0.397855%** and **2.027498%**, respectively;
all four adjacent pairs favor official in each run. Every sample, including
the default run's larger fourth paired difference, remains included.
These are descriptive separate-cohort results, **not proof of a causal
profile regression**. Startup intervals overlap; no midpoint or resolved
winner is inferred. Bounds describe host-observed crop readiness, not Android
frame timing, search speed or guaranteed interactivity.

Actual Animal images/classification, submitted article-title results and
Hawaii/Hilo detailed maps passed on the corrected trial. Full-text, routing
and search-latency acceptance do not follow. The default already meets the
APK-size goal; without a demonstrated runtime advantage, its existing
optimization policy is retained rather than promoting the experimental
package profile. No repository Cargo-profile change was made. The default APK
was restored in place with exact installed hash and actual one-tab English
Animal rendering checked. Official All Files Access was restored Off through
normal Settings, both benchmark apps stopped and Home restored.

### Idle transport and bootstrap lifetime corrections — total memory still worse

The installed ARM64 APK is **67,145,771 bytes**, SHA-256
`7d45d21ed9480c5aefcb54f641ecb6d851a28b1ce7156fb397c06edc0e71b398`.
It is 8,192 bytes above the preceding ASCII build. Features, locales, native
search SDK, signing identity, manifest and 16 KiB alignment are retained.
The [complete receipt](benchmarks/2026-11-05-android-idle-memory.json)
binds source, artifact, functional checks and every memory sample.

The unchanged `8217a32a…` profiler ran **373.646 seconds**, standard mode,
**ABBAABBA, eight cold launches, 32 samples total/16 per app**. That is four
launches per app, not 16 independent launches. The same prepared real Animal
references were used; all 16 before/after screenshots matched with MAD 0.0
and independently showed Animal content. All 64 meminfos and 72 ownership
observations were admitted, without exclusions. Settling remains 30 seconds
after launch completion; **no startup timing was measured**.

| Independent pooled median, MiB | Ours | Official ARM64 |
| --- | ---: | ---: |
| Main + owned renderer TOTAL PSS | **257.025879** | **247.258301** |
| Main process PSS | 148.740234 | 161.948242 |
| Renderer PSS | 108.285645 | 86.002930 |
| Native heap PSS | 33.926758 | 44.736328 |
| Native heap allocated, not PSS | 58.554688 | 84.900391 |
| APK + SO mmap PSS | 104.122070 | 87.656250 |

The pooled total difference is **+9.767578 MiB (+3.950354%)**. Adjacent pair
differences are **+7.107422, +10.519531, +9.770996, +8.377930 MiB**; their median
is **+9.074463 MiB**, a different statistic. Component medians are not additive;
graphics/detailed categories overlap and shared-page attribution varies.
Do not interpret differences from earlier cohorts as causal savings.

The changes are retained for narrower demonstrated/source-established benefits:
callbacks no longer retain the complete initial archive-storage bootstrap,
and empty/history-only managers do not build two unused HTTP/TLS clients.
A fresh-context, explicit-GC diagnostic changed bootstrap reachability from
retained to collectible while preserving clear/set sequencing. This is **not**
quantified payload-byte or app-PSS saving. Existing transport builders,
policy/cancellation checks and offline journal/recovery initialization remain.
Current native real-Wikipedia recovery and real HTTPS Hawaii composition passed;
the physical production APK rendered actual Animal and Hawaii/Hilo detail.
Those functional checks are not latency comparisons.

The owned map tab was closed, one English Animal tab restored, both benchmark
apps stopped, and the official app's temporary All Files Access restored Off
through normal Settings (UID mode deny).

### Automated article-ready startup calibration — no faster app established

The profiler now supports **`--measure-startup`**. It confirms the resolved
Home activity and unlocked state, rejects an already-matching prelaunch image,
launches the selected app, and records timed screenshots until the prepared
article crop matches its reference. It retains the preceding mismatch, first
match, acquisition start/end times and launch-process cleanup. This is reference
agreement, not independent semantic recognition or guaranteed interactivity.
The `am start -W` activity-draw logs remain separate from article readiness.

The actual frozen `8217a32ad87aaaf20c9ced8643d8033fa895302887b219e6316251ffa57235d5`
tool ran **208.1 seconds**, unattended ABBA, on the same installed ASCII APK
and official app. All four Home/mismatch/match transitions were visually
inspected: the mismatches show Home, splash or loading; first matches show the
actual Animal heading, montage and classification. Each app has **two launches**,
not eight independent startup observations.

| Prepared-article startup, median endpoint bounds | Ours | Official |
| --- | ---: | ---: |
| Lower bound, seconds | 1.311 | 0.010 |
| Upper bound, seconds | 2.626 | 2.133 |

Each interval is `[last mismatch acquisition start, first match acquisition
end]` relative to host-monotonic launch start. Screenshot acquisition is slow
enough to make these intervals coarse; no midpoint, frame-accurate timing,
significance claim or startup winner is inferred. The lower native endpoint
near zero reflects an early screenshot, not a near-zero launch.

The separate memory observations in this same calibration run settle for
30 seconds **after admitted readiness and launch completion**, unlike the
standard tool mode's launch-completion settle. Its 16 total samples/eight per
app yield pooled PSS medians **235.92 versus 238.43 MiB**, but adjacent
ours-minus-official block differences disagree: **−7.16 and +7.69 MiB**.
This does not establish reliably lower memory and is not pooled with preceding
cohorts. The [sanitized calibration receipt](benchmarks/2026-11-05-android-startup-calibration.json)
retains timing frames, all samples, owned process generations and raw hashes.

Append `--measure-startup` to the reproduction command below, with a fresh output
directory. The same operator-prepared article and per-app references are required;
no app data, permission or device-display settings are changed by the tool.


### Rejected Android 8 MiB cache experiment — no useful heap benefit

A separately reviewed release changed only Android's runtime libzim cache-cost
budget from 16 to 8 MiB, once before archive construction. It retained active
blob ownership and the library's oversized-entry behavior: this was never an
8 MiB RSS/PSS cap. The candidate APK was the same size as the ASCII baseline;
its exact source/artifact identities and full run remain in the
[rejected-experiment receipt](benchmarks/2026-11-05-android-cache8-rejected.json).

The same startup-enabled profiler completed another unattended four-launch,
16-sample ABBA run in 205.7 seconds. Against the preceding same-tool ASCII
baseline, independently computed native-heap medians were:

| Our native heap, both owned processes | 16 MiB baseline | 8 MiB experiment |
| --- | ---: | ---: |
| Allocated, KiB | 60,551 | 60,473 |
| PSS, KiB | 35,380 | 35,855.5 |

The 78 KiB allocation difference is not a useful demonstrated saving; native
heap PSS did not improve. Candidate-versus-official total PSS was unfavorable
in both adjacent pairs. Startup intervals overlap and establish neither a
speedup nor a material regression. Separate cohorts and changing shared-page
attribution prevent interpreting their differences as a causal policy effect.

**Rejected:** no earlier-eviction/redecompression tradeoff was retained without
a measured benefit. The repository's native bridge was restored byte-for-byte
to the prior source, and the approved ASCII APK was reinstalled with its exact
installed hash verified. The temporary official all-files grant was restored
Off through normal Settings (UID mode deny); the diagnostic helper is absent,
owned forwards are removed, and both benchmark apps were stopped. Unrelated
device forwards and the shared real archive sources were left alone.

### Latest physical Samsung: ASCII source backings shrink; app PSS remains worse

The installed ASCII-encoding release is **67,137,579 bytes**, SHA-256
`0f04390127c81e9d8ce23965fa595876298fbb2ac722daa3f88a154485932a33`.
It is **4.588125% smaller** than official Kiwix **3.14.1 standalone ARM64**
(70,366,062 bytes, `62a9910be260030e642465c17d642537e6ee4ee5f2d4596c248267c2766a88b8`),
but **151,552 bytes larger** than the preceding `089c7d49…` APK. Its frozen
product source is `c4603dd52502ef2fceb099754e30285ef583d10f` plus the Vite
`charset: 'ascii'` and reader-bridge Unicode-escape overlays, not the newer
profiler-tool commit. The [combined sanitized receipt](benchmarks/2026-11-05-android-ascii-memory.json)
records every frozen overlay/frontend digest, SDK/native/APK identities,
signature, manifest/JNI exports and 16 KiB alignment evidence. All **16
recorded build/inspection commands** returned zero; that is not 16 functional
tests. The installed APK identity matched before and after measurement.

#### Separate source-backing diagnostic, not acceptance PSS

Physical WebView heap snapshots from the preceding and ASCII APKs trace each
selected `system / Script` node's `source` edge to its native
`system / ExternalStringData` backing stores. These are **GC-capable DevTools
diagnostics**, collected separately from the uninstrumented memory run below.
They are not whole-heap, allocation-rate or app-PSS acceptance measurements.

| Selected retained source | Previous characters / above Latin-1 | ASCII characters / above Latin-1 | Previous backing bytes | ASCII backing bytes |
| --- | ---: | ---: | ---: | ---: |
| Main frontend entry | 688,893 / 521 | 691,723 / 0 | 152 + 1,377,798 | 152 + 691,735 |
| Reader bridge | 124,291 / 4 | 124,311 / 0 | 152 + 248,594 | 152 + 124,323 |
| **Sum of these backing stores only** | | | **1,626,696** | **816,362** |

The observed difference is **810,334 bytes (0.772795 MiB)**. Both new sources
also contain zero characters above ASCII; the unchanged 56-byte source-string
nodes are not part of that backing-only sum. The main source remains retained
by live functions: encoding makes the backing one-byte rather than two-byte,
not unreachable. Escaped source preserves translated runtime values; it does
not remove locales. **Not every shipped JavaScript file is ASCII**:
the unchanged public `boot-guard.js` still contains literal non-ASCII.
Independent source/heap review supports the bounded backing-store claim,
not a causal total-memory gain.

#### Separate automated Animal cohort: 3.15% more attributed PSS

The same frozen profiler `874cc01df6ffb45187e65f5aa579ce7bc365ad15f63ff37316715f3aa7faf2a2`
completed **189.431676 seconds** of unattended **ABBA, four process-cold
launches, 16 samples total/eight per app** on Samsung **SM-A356U / Android 16 /
API 36 / WebView 156.0.8078.49**. That is **two launches per app**, not eight
independent launches. Operator-prepared actual Animal states and the same
reference-crop method/defaults documented below were retained. Each launch
settles for the requested 30 seconds **after launch completion**, before the
reference screenshot gate; this differs from earlier manually
article-readiness-gated cohorts. All eight before/after crops matched with
mean absolute RGB difference **0.0**; independent raw review also inspected
actual Animal heading/montage/classification screenshots.

| ASCII APK Animal: independent pooled medians | Ours | Official |
| --- | ---: | ---: |
| Main + exclusively owned renderer TOTAL PSS, KiB | **259,378.5** | **251,456** |
| Same pooled median, MiB | **253.299316** | **245.562500** |
| Observed TOTAL PSS range, KiB | 253,783–264,928 | 245,457–254,955 |
| Main PSS, MiB | 150.60 | 164.18 |
| Owned renderer PSS, MiB | 102.23 | 81.38 |
| Native heap PSS, MiB | 34.86 | 44.83 |
| .apk + .so mmap PSS, MiB | 102.94 | 89.69 |
| Unknown detailed-row PSS, MiB | 54.78 | 43.49 |
| Graphics summary PSS, MiB | 31.69 | 31.90 |

**Still unfavorable: +7,922.5 KiB (+7.736816 MiB, +3.150651%)** between the
pooled medians. Block medians in order are **257,895.5; 251,448; 254,752;
262,463 KiB**. Adjacent ours-minus-official paired differences are
**+6,447.5 and +7,711 KiB (+6.296387 and +7.530273 MiB)**, both unfavorable.
Their median **+6.913330 MiB** is not the pooled-median difference.
All **16 samples, 32 process meminfos and 36 ownership observations** were
independently admitted without exclusions; the receipt preserves their
metrics, generations, monotonic timing and raw-evidence digests. Separate
component medians are not additive; graphics can overlap detailed categories,
mapped PSS depends on shared attribution, and Unknown is not identified JS heap.

This sums sequential `dumpsys meminfo` TOTAL PSS (including reported graphics
mtrack) for stable, exclusively owned main/renderer generations, not unrestricted
whole-system memory. Shared/unassigned services are excluded and swap is
separate. There was no video, DevTools/CDP, forced GC or filesystem-cache purge
**during this PSS run**. No lower app memory, ASCII-specific causal PSS saving,
startup/search, installed-storage ratio or matched-map performance is inferred
from comparison with previous cohorts.

The parent observed actual Persian RTL Settings, restored English and returned
to a ready actual Animal tab. The current APK also rendered the real Hawaii
offline map, returned Hilo place-search results and selected Hilo with detailed
roads/buildings at the displayed 300 m scale. Private screenshots are linked
by digest. All **17 existing live-export host tests passed** in the parent's
observed command result (237.107541 ms); no separate raw test stdout was saved.
Their audio coverage is **synthetic inert snapshot representation only, not
audio playback**. These observations are neither complete locale/accessibility/
feature-parity certification nor matched-map memory/FPS evidence. The memory
tool stopped both packages and returned Home. The later diagnostic helper was
uninstalled and its owned forwards removed without changing the product APK;
an unrelated emulator forward was preserved. Official all-files access was
temporarily On during collection and the following performance cycles. It was
subsequently restored Off after the rejected cache experiment above; that
restoration is separate from profiler cleanup. The iPhone, VoiceOver and audio
playback were not accessed for this batch.

### Preceding frozen physical Samsung: previous APK memory is 6.19% worse

The frozen final installed APK is **66,986,027 bytes**, SHA-256
`089c7d4906162e69ff9c757e9be104af197f94c25902d7d679ee2687c72d2500`.
Official Kiwix **3.14.1 standalone ARM64** is **70,366,062 bytes**, SHA-256
`62a9910be260030e642465c17d642537e6ee4ee5f2d4596c248267c2766a88b8`.
Our APK remains **4.80% smaller**, with the same byte count as the earlier
annotation-release APK but a different identity. Both installed `base.apk`
hashes were verified. The [final release/runtime receipt](benchmarks/2026-11-04-final-android-release-runtime.json)
records the exact pre-collection source/SDK/artifact freeze and bounded runtime
acceptance; the earlier `adcd191…` pilot is not this artifact.

All **eight process-cold blocks and 32 samples** were independently admitted,
without exclusions or numerical-outlier pruning. The fixed **ABBAABBA** order
has **four launches per app**, with four samples within each launch—not 16
independent launches. On the Samsung **SM-A356U / Android 16 / API 36 /
WebView 156.0.8078.49**, both apps retain prepared profiles, the same two real
shared archives and one fresh index-to-actual-**Animal** tab, with no saved
highlights. No additional app-data reset was performed.

Appearance remains controlled: ours Light/archive-original font and colors/
16 px/100% zoom; official Dark mode Off/100% text zoom. This does not prove
identical computed typography or equal chrome/viewport. The other app stays
stopped. Sampling follows actual heading/montage/classification readiness and
a measured **30.0005–30.0054-second** settle. The requested two-second pause is
sleep between completed acquisitions; actual sample starts are approximately
**7.818–8.768 seconds** apart, with all actual timestamps retained. There was
no video, DevTools/CDP, forced GC or filesystem-cache purge during collection.

| Frozen final Animal memory | Ours | Official Android |
| --- | ---: | ---: |
| Pooled median main + owned renderer TOTAL PSS, KiB | **264,599** | **249,175.5** |
| Same pooled median, MiB | **258.397461** | **243.335449** |
| Observed TOTAL PSS range, KiB | 241,827–269,040 | 231,203–260,799 |
| Median swap PSS, KiB, separate from resident PSS | 455 | 452 |

**Ours is +15,423.5 KiB (+15.062012 MiB), or +6.189814%, higher.** The block
medians in actual order are **264,599; 256,909.5; 246,219; 267,802; 268,008;
258,974; 236,870.5; 247,241.5 KiB**. All four adjacent ours-minus-official
paired-block differences are unfavorable: **+7,689.5; +21,583; +9,034;
+10,371 KiB** (**+7.509277; +21.077148; +8.822266; +10.127930 MiB**).
Every completed sample remains included.

The unchanged reviewed collector sums sequential `dumpsys meminfo` **TOTAL
PSS including reported graphics mtrack** for the stable, exclusively owned
main process and WebView renderer. Shared/unassigned OS/GPU/display services
are excluded; swap is separate. This is not unrestricted whole-system memory
or pooled emulator `smaps` PSS. At all four pair starts, thermal status was 0,
USB power connected and battery 100%; temperatures were **27.3, 27.3, 27.5,
27.5 °C**, not a continuous thermal/energy result.

The [comprehensive final memory receipt](benchmarks/2026-11-04-physical-android-final-memory.json)
contains **all 32 totals, 64 process measurements and 72 ownership
observations**, selected PID/name/start-tick/package attribution, actual
settling/acquisition timestamps, component medians, paired signs, environment
and private raw-evidence basenames/digests. Raw activity/process dumps,
screenshots, personal paths, device identifiers and wall clocks are not
published. Independent raw review reproduces every summed total and stable
generation guard; the parent inspected all 16 before/after Animal screenshots.

**The completed result is worse than official for this workload and metric.**
The combined source changes do not isolate any individual causal saving; the
earlier APK cohorts, pilot pair and diagnostics remain separate. Earlier
startup/search videos and the Settings-storage table are not measurements of
this final APK. No final-APK startup/search latency, installed-storage ratio
or matched-map memory/FPS result is inferred.

After this batch, normal Samsung Settings showed the temporary official
all-files switch **Off**, with UID `MANAGE_EXTERNAL_STORAGE` mode **deny**.
Both benchmark packages were stopped and the parent returned Home. The
sanitized memory receipt links the private restoration proof by digest.

### Single-command automated repeat — 2026-11-05 (separate four-block cohort)

The reviewed `scripts/profile-android-memory.py` (SHA-256
`874cc01df6ffb45187e65f5aa579ce7bc365ad15f63ff37316715f3aa7faf2a2`)
completed an unattended **188.196-second** run on the same Samsung model/API/
WebView, with **ABBA, four process-cold launches, 16 samples total (eight per
app)**. This is a separate diagnostic repeat, **not a replacement for or pooled
with the eight-block, +6.19% frozen result above**. Installed APK bytes and
versions matched before/after: ours `089c7d4906162e69ff9c757e9be104af197f94c25902d7d679ee2687c72d2500`
(66,986,027 bytes), official 3.14.1 standalone ARM64
`62a9910be260030e642465c17d642537e6ee4ee5f2d4596c248267c2766a88b8`
(70,366,062 bytes). No subsequent source optimization is measured here.

| Automated Animal repeat: independent pooled medians, MiB | Ours | Official |
| --- | ---: | ---: |
| Main + exclusively owned renderer TOTAL PSS | **237.203125** | **225.247559** |
| Main PSS | 133.52 | 148.72 |
| Owned WebView renderer PSS | 103.41 | 76.53 |
| Native heap PSS | 34.46 | 42.00 |
| .apk mmap PSS | 75.80 | 65.94 |
| .so mmap PSS | 12.51 | 11.20 |
| .apk + .so mmap PSS | 88.32 | 77.15 |
| Unknown detailed-row PSS | 54.84 | 41.95 |
| Graphics summary PSS | 31.69 | 31.90 |

**Still worse: +12,242.5 KiB (+11.955566 MiB, +5.307745%)** between
the pooled total medians. Adjacent four-sample block-median differences are
**+1,875 and +18,629 KiB (+1.831055 and +18.192383 MiB)**, both unfavorable.
The generated summary's +10.01 MiB is the median of those two paired
differences, **not** the difference of pooled medians. All 16 samples remain
included. These independently computed component medians are **not additive**;
graphics summary can overlap detailed categories. Mapped PSS depends on shared
mapping attribution; Unknown is not a demonstrated JavaScript/retained heap,
and these observations establish no causal allocation saving.

Reproduction requires Python 3 and Pillow, an authorized physical Android
target, and **operator-prepared actual Animal article states and trusted,
full-size PNG references for each app** at the desired scroll position.
Reading state must survive relaunch; the tool does not navigate or prepare
articles. Set the placeholders explicitly; the output directory must not exist:

```sh
python3 scripts/profile-android-memory.py \
  --adb "$ADB" --serial "$SERIAL" --model "$MODEL" \
  --reference-ours "$REFERENCE_OURS" \
  --reference-official "$REFERENCE_OFFICIAL" \
  --output "$NEW_OUTPUT_DIRECTORY"
```

Default `--blocks 4` gives 16 total samples/eight per app; use `--blocks 8`
for full ABBAABBA, 32 total/16 per app. Each guarded launch has a requested
30-second settle before screenshot/reference admission and four acquisitions,
with two seconds sleep between completed samples (not a two-second sample
period). Before/after screenshots agree with the supplied reference crops
(normalized 0.08, 0.08, 0.92, 0.80; maximum mean absolute RGB difference 5/255).
This is **pixel-crop agreement, not OCR or independent semantic article proof**.
The run retained stable, exclusive package ownership and PID/start-tick
generations across 36 ownership observations and 32 per-process measurements;
all selected samples had exactly one main and one renderer, no other owned PSS.
It excludes shared/unassigned OS/GPU/display services and reports swap separately.
Process-cold does not mean cold filesystem cache.

The tool installs nothing, wipes no app data, writes no grants or Settings,
and uses no video, DevTools/CDP, forced GC or cache purge. Its final cleanup
successfully stopped both named benchmark packages and pressed Home. For
this cycle the parent separately enabled official all-files access temporarily;
the retained receipt records restoration as pending at collection. It was
subsequently restored Off after the rejected cache experiment above. Profiler
cleanup itself does not change permissions.

The [sanitized automated receipt](benchmarks/2026-11-05-android-automated-memory.json)
retains all 16 sample totals, 32 process readings, generation/ownership guards,
crop admission, monotonic timings, package identities and private evidence
basenames/digests. Serial, host/install paths, wall clocks, raw dumps and app
data are omitted. Startup/search, installed-storage and matched-map limits
remain unchanged: this memory repeat measures none of them.

### Earlier physical Samsung — `8ccbe629…`: 5.47% more Wikipedia PSS

The actual rebuilt ARM64 APK is **66,986,027 bytes**, SHA-256
`8ccbe6298cda9ec062a18c5544eb0fdfd62f0d8d80ac387c5a043893f03bd267`.
It is **4,096 bytes larger** than the earlier `5eec1e4` APK and **4.80% smaller**
than official 3.14.1 standalone ARM64 (**70,366,062 bytes**, SHA-256
`62a9910be260030e642465c17d642537e6ee4ee5f2d4596c248267c2766a88b8`).
The [release receipt](benchmarks/2026-11-04-annotation-android-release.json)
identifies `cbb13b3` plus the reviewed annotation bridge/regression overlays,
excluding paused Library edits, and records signature, ARM64/JNI and 16 KiB
alignment gates plus exact embedded production-bridge identity. The installed
device `base.apk` hash was observed to match before this repeat.

All **eight blocks and 32 samples** were independently admitted, with no
exclusions. On the same **Samsung SM-A356U / Android 16 / API 36 / WebView
156.0.8078.49**, prepared profiles were **retained, not reset again**. Both apps
closed the map tab and followed a freshly opened Wikipedia index's actual
**Animal** link: two same shared archives and one article tab each. Controlled
appearance remains ours Light/archive-original font and colors/16 px/100% zoom
versus official Dark mode Off/100% text zoom, not proven identical typography
or equal chrome/viewport.

The fixed process-cold **ABBAABBA** order has four blocks per app and four
samples per block, with the other app stopped. Each 30-second settle begins
only after observed actual article readiness; measured spans are
**30.010065–30.010121 seconds**. Actual sample-start intervals are
**6.763–7.744 seconds**; the unchanged two seconds is requested sleep between
acquisitions, not the sampling period. No video, DevTools/CDP, forced GC or
filesystem-cache purge was used during collection.

| Current physical Animal memory | Ours | Official Android |
| --- | ---: | ---: |
| Pooled median main + owned renderer TOTAL PSS, KiB | **248,535.5** | **235,655.5** |
| Same pooled median, MiB | **242.71** | **230.13** |
| Observed TOTAL PSS range, KiB | 236,200–250,414 | 228,457–248,518 |
| Median swap PSS, KiB, reported separately | 449.5 | 451.5 |
| Main-process median PSS, KiB | 140,480.5 | 154,320.5 |
| Owned-renderer median PSS, KiB | 107,338 | 81,382.5 |

**Ours is 5.47% higher: +12,880 KiB (+12.578125 MiB).** Block medians in
run order are **249,978; 248,078.5; 234,333; 249,192; 248,535.5; 235,346;
237,039; 241,765.5 KiB**. All four adjacent ours-minus-official paired-block
differences are unfavorable: **+1,899.5; +14,859; +13,189.5; +4,726.5 KiB**.
The higher official b1 and lower ours b7 remain included. Every first sample
is lower than its remaining three in both apps; no causal explanation or
numerical pruning is applied.

Each process-component median is calculated independently across 16 readings;
**component medians do not add to the pooled total median**. The renderer's
median is **25,955.5 KiB (25.35 MiB) higher**, offsetting our lower main-process
median by scope. This makes renderer attribution a bounded next investigation
target, **not a demonstrated heap, GC or allocation cause**.

The metric sums sequential `dumpsys meminfo` **TOTAL PSS including reported
graphics mtrack** for stable, exclusively attributed main/owned-renderer
generations. Swap remains separate; shared/unassigned OS/GPU/display services
are excluded. It is not unrestricted whole-system memory or pooled emulator
PSS. At all four pair starts, thermal status was 0, USB power connected and
battery 100%; temperatures were **27.7, 27.8, 27.7, 27.7 °C**.

The [comprehensive current physical receipt](benchmarks/2026-11-04-physical-android-annotation-memory.json)
preserves all 32 totals, per-process PSS/swap and generation/ownership
observations, actual timing, component medians, pair signs, environment and
private raw-evidence basenames/digests. **No physical memory improvement or
annotation-specific causal effect is established**: this release includes
previously published incoming-intake UI as well as annotation changes, and
the prepared session differs from the earlier cohort. The isolated 4.48%
emulator result is described separately below.

Startup/search videos and the Settings-storage table below belong to the
**earlier `5eec1e4` APK**, not this rebuilt release. No new-release
startup/search or matched installed-storage comparison was collected. The
earlier rounded official 107 MB storage observation is retained as historical,
not asserted unchanged after the repeat.

After that earlier finite physical batch, the parent restored the temporary official
all-files permission **Off** through Samsung Settings (UID broad-grant mode
deny), restored both pointer/touch indicators to their original absent state,
force-stopped both benchmark apps and returned Home. Fifteen owned device MP4
copies (**13,672,800 bytes**) were removed only after local/remote hash
verification; all host originals and shared source archives were retained.
The private cleanup receipt is linked by digest in the sanitized memory
receipt. The iPhone and VoiceOver were not accessed for this batch.

### Earlier physical Samsung baseline — `5eec1e4`: 4.11% more Wikipedia PSS

This is the accepted **physical Samsung SM-A356U**, Android **16 / API 36**,
1080 × 2340, Google WebView **156.0.8078.49** comparison, not an emulator result.
The frozen `5eec1e4` APK is **66,981,931 bytes**, SHA-256
`5d28b3c6c2fdc9d08879e8e0b5b2edf27ae82990f2898dac71ab4cdb5efeaa03`;
official Kiwix **3.14.1 standalone ARM64** is **70,366,062 bytes**, SHA-256
`62a9910be260030e642465c17d642537e6ee4ee5f2d4596c248267c2766a88b8`.
Installed APK and on-device real archive bytes were hash-verified.

The owner authorized clearing **both benchmark apps' data**. This cohort uses
fresh profiles, exactly two real archives and one **Animal** tab reached through
the Wikipedia index's actual link. It supersedes the earlier preserved-profile
four-block **apparent approximately 4.2% win**; that series and the earlier
official warm observation are excluded, not pooled or silently discarded.

This is **controlled appearance, not untouched product defaults or proven
identical computed typography**: ours uses light theme, archive-original font
and colors, 16 px text and 100% page zoom; official uses Dark mode Off and 100%
text zoom. Matching article landmarks do not establish equal chrome or viewport.

Eight process-cold blocks use **ABBAABBA** order
(ours, official, official, ours, ours, official, official, ours), with the other
app stopped. Each starts sampling after actual article-visible readiness and
30 seconds settling; measured settle spans are **30.001–30.010 seconds**.
Four samples per block give **16 per app**. The requested two-second pause is
**sleep between acquisitions, not a two-second sampling period**. Actual
sample-start intervals are **5.866–7.219 seconds**; the receipt preserves every
start, end and completed-observation timestamp. No filesystem-cache purge or
concurrent video recording was used during memory collection.

| Physical Animal attributed memory | Ours | Official Android |
| --- | ---: | ---: |
| Pooled median main + owned renderer TOTAL PSS, KiB | **246,477.5** | **236,757** |
| Same pooled median, MiB | **240.70** | **231.21** |
| Observed TOTAL PSS range, KiB | 239,438–246,774 | 229,526–237,214 |
| Median swap PSS, KiB, separate from resident PSS | 428 | 427 |

**Ours is 4.11% worse: +9,720.5 KiB (+9.49 MiB).** The eight block medians,
in run order, are 246,076; 237,004; 236,958; 246,499; 246,684.5; 235,207;
236,757; and 246,474 KiB. All four adjacent ours-minus-official paired-block
differences are unfavorable: **+9,072; +9,541; +11,477.5; +9,717 KiB**.
Every block's first sample is lower than its remaining three in **both apps**.
All samples are retained; no causal explanation or outlier pruning is applied.

The metric sums sequential `dumpsys meminfo` **TOTAL PSS, including reported
graphics mtrack**, for the main process and all exclusively attributed owned
renderers (one renderer was selected in every observation). PID, process start
ticks and exclusive `ProcessRecord.packageList` ownership remain stable before
and after every sample. Shared/unassigned OS/GPU/display services are excluded;
this is not unrestricted whole-system memory and is **not pooled with emulator
`smaps_rollup` PSS**. Swap is separate. At each adjacent pair start, thermal
status was 0, USB power connected and battery 100%; battery temperatures were
28.6, 28.4, 28.3 and 28.3 °C. These are not energy or continuous thermal results.

The [sanitized physical receipt](benchmarks/2026-11-04-physical-android-clean.json)
contains all 32 totals, per-process PSS/swap, selected PID generations and
ownership observations, actual timing, paired comparisons, settings and exact
private-evidence basenames/digests. Raw ownership dumps and original screenshots
remain private: device identifiers, command paths and status bars are not
published. Independent memory admission and publication-review boundaries are
explicit. **Matched physical map performance remains unmeasured in accepted
results**. The separate physical startup, prepared-session title-autocomplete
and map-functionality observations follow below.
This earlier frozen APK predates the annotation change. Its measurements remain
separate from the rebuilt-release repeat above; no cross-cohort causal gain is inferred.

### Earlier physical Samsung — `5eec1e4`: process-cold Open-marker → Animal

For the same frozen physical-phone apps and real Wikipedia article described
above, silent on-device video bounds the **observable pointer-marker transition
on Android App-info's Open button → actual Animal heading, complete visible
montage and visible classification content** after process-cold preparation.
This is not a launcher-icon benchmark, physical-tap/electrical-input latency,
first paint, app-only CPU time, document-load completion, interactivity or
offscreen readiness.

The final result is an **explicitly post-hoc, uniform semantic adjudication of
every original frame in all nine chronological clips**, not a retuned
pixel-template result. Eight clips are admitted: **four observations per app**.
The original eight captures followed ABBAABBA, but original **b4 (ours)** has
the required content only in its final encoded frame and lacks the required
later encoded confirmation at least 100 ms apart. It remains rejected; a
container's static final-frame hold does not create another encoded frame.
One prospective later replacement was authorized for that admission failure,
not its timing. It supplies the fourth semantic observation for ours, but this
is **not an uninterrupted eight-run ABBAABBA cohort** and does not create a
fourth original adjacent pair. All nine attempts and both dispositions remain.

| Descriptive observable startup interval, ms | Ours | Official Android |
| --- | ---: | ---: |
| Median bounds, four admitted semantic observations per app | **1,273.6165–1,555.994** | **1,110.961–1,176.250** |

The median interval is bounded by the medians of lower and upper observation
endpoints, not by selecting a point timestamp. The ours-minus-official median
difference is **+97.3665 to +445.033 ms**, unfavorable to ours. Ours is also
slower in **each of the three original complete adjacent pairs**, whose
difference intervals are **+317.723–676.679**, **+65.245–351.344** and
**+120.422–420.011 ms**. These are descriptive small-cohort bounds, **not
population percentiles or a general startup ranking**.

The unchanged frozen pixel classifier admits **seven of the original eight**
and **seven of the amended nine** clips: the later replacement also fails its
exact-pixel thresholds. Independent semantic frame review nevertheless finds
the required content in two replacement frames at least 100 ms apart. The
global pixel-color/appearance difference is retained and its **cause is
unknown**; no normalization, template/threshold adjustment or further retry is
used. Some official ready frames retain chrome loading progress, which does not
invalidate this required-visible-content endpoint or establish completed loading.
Native variable-frame-rate PTS and gaps remain; there is no interpolated
cadence or claim of unseen continuous readiness. Untimed calibrations are excluded.

The [original-eight frozen receipt](benchmarks/2026-11-04-physical-android-startup-original.json)
retains its **7 admitted / 1 rejected** disposition and no pooled ranking.
The [amended frozen-pixel receipt](benchmarks/2026-11-04-physical-android-startup-frozen-pixel-amended.json)
retains **7 admitted / 2 rejected**, including its historical pending-review
metadata rather than rewriting that record.
The independently reviewed [final semantic receipt](benchmarks/2026-11-04-physical-android-startup-semantic.json)
retains both frozen and semantic dispositions, all nine clip identities,
original-frame marker/readiness/confirmation PTS, eight admitted intervals and
the descriptive summaries above. Its amended-receipt reference resolves to the
published frozen copy. These sanitized receipts contain no raw video/screens,
device identifiers or private command paths. **No physical annotation-change
gain has been measured**. The separately admitted physical title-autocomplete
observations follow below; they are not inferred from the emulator results.

### Earlier physical Samsung — `5eec1e4`: title autocomplete has no resolved winner

All four warmed, prepared-session recordings are independently admitted in
**BAAB** order (official, ours, ours, official), with **two observations per
app** and warm-ups excluded. Each starts from a visible nonmatching `zzzzqx`
query, absent target rows and the visible keyboard, then enters **Animal**.
The endpoint bounds the **first complete visible exact query → both actual
Animal and Animal Characteristics title rows legible above the keyboard**.
This is title autocomplete, not an Enter/Search submission, full-text query,
cold search, title-to-reader navigation, physical-keystroke latency or CPU timing.

The entry surfaces differ: ours uses **Library → Search inside your archives**,
Wiki-only scope and the title-autocomplete dropdown; official uses the reader's
native **SearchFragment** for the current Wikipedia archive. Preparation
establishes the Wiki-only scope. Our recorded selected-archive name is offscreen,
while returned rows carry Wikipedia 100 labels; no equivalent entry-UI claim
is made.

| Descriptive complete-visible-query → title-row interval, ms | Ours | Official Android |
| --- | ---: | ---: |
| Median bounds, two admitted observations per app | **0–269.756** | **125.411–226.639** |

The ours-minus-official descriptive median-difference interval is
**−226.639 to +144.344 ms**. Both adjacent-pair difference intervals also
overlap zero: **−168.389 to +252.589 ms** and **−284.889 to +36.100 ms**.
**These observations do not resolve a faster app**, nor establish equivalence.
Our zero lower bounds reflect overlapping observation intervals, **not
instantaneous query processing**.

Original native variable-frame-rate observations are retained. Official list
crossfades overlap old and new text; the intervals include ambiguous frames
between the last definitely non-ready and first definitely ready frame rather
than assigning a favorable exact onset. Later encoded frames confirm the
definitely-ready result at least 100 ms apart. No interpolation, resampling,
clamping, new capture or retry supplies additional precision.

**No debounce was subtracted**, but input processing and debounce may begin
before the complete query becomes visible. These bounds therefore do not
establish that the full configured debounce duration is included. Two
observations per app do not establish population percentiles or a general
product ranking.

The independently reviewed [physical title-autocomplete receipt](benchmarks/2026-11-04-physical-android-search.json)
retains all four clips' identities, original integer PTS and stream time bases,
exact rational bounds, crossfade uncertainty, confirmation and private
full-frame hashes. The table rounds only for presentation; the rational
intervals are authoritative. Raw frames, private command paths and device
identifiers are not published.

### Earlier physical Samsung — `5eec1e4`: Hawaii map functionality only

Both apps' normal **Home** paths rendered actual Hawaii map content.
Independent instrumentation review accepts this as **functionality only**, not
a matched map-memory or FPS comparison. Canvas heights and Honolulu's visible
vertical position differ; official reader controls clip the map footer/scale.
Home's configured camera, together with the archive's `maxBounds`, does not
prove equivalent resulting camera geometry. No identical-camera, equal-tile,
equal-viewport or physical map-performance claim follows.

The private originals `clean-map-official-home.png` and
`clean-map-ours-home.png` retain the rendered-map evidence; raw screenshots are
not published here.

### Earlier physical Samsung — `5eec1e4`: observed Settings storage

Normal **Samsung Settings → App info → Storage** shows the following literal
displayed values for the frozen `5eec1e4` and official 3.14.1 baseline apps,
**not the new annotation-optimization APK**:

| Rounded Samsung Settings category | Ours | Official Android |
| --- | ---: | ---: |
| App | 45.92 MB | 77.60 MB |
| Data | 8.96 MB | 28.52 MB |
| Cache | 16.71 MB | 471 kB |
| Total | **71.59 MB** | **107 MB** |

This is a **smaller observed Settings-reported footprint in the exercised
profile**, not exact filesystem bytes or a fresh-install/default-size
comparison. Both clean profiles had two archive references and one Hawaii tab
after earlier Animal benchmarks; data/cache contents reflect each actual
workflow, not identical cache contents. No data or cache was cleared for this
observation. The same shared source ZIM files are outside these app-private
totals. Keep the displayed units/rounding: these categories are not the signed
APK byte counts or the historical emulator code-root census, and they do not
establish an OEM compression or accounting mechanism.

The independently reviewed [physical Settings storage receipt](benchmarks/2026-11-04-physical-android-installed-storage.json)
preserves the exact displayed strings and private Storage-detail XML/PNG
basenames, sizes and digests. Raw Settings screenshots remain private.


### Android EMULATOR: accepted Wikipedia memory regression

The earlier frozen `5eec1e4` optimized APK is SHA-256
`5d28b3c6c2fdc9d08879e8e0b5b2edf27ae82990f2898dac71ab4cdb5efeaa03`;
the official 3.14.1 ARM64 APK is
`62a9910be260030e642465c17d642537e6ee4ee5f2d4596c248267c2766a88b8`.
Both run on the same newly owned API 33 ARM64 emulator with two library
references and one restored **Animal** tab from the real Wikipedia 100 archive.
The eight process-cold blocks use **ABBAABBA** order, with the other app stopped,
30 seconds settling and four samples per block: 16 samples per app. Timed
acceptance blocks exclude the concurrent Mac compilation/setup cohort.

| App-attributed memory | Ours | Official Android |
| --- | ---: | ---: |
| Median main + exclusively attributed WebView PSS, MiB | **204.7** | **191.5** |
| Observed PSS range, MiB | 202.9–205.6 | 190.8–193.6 |
| Median swap PSS, MiB, reported separately | 1.57 | 1.59 |

**Ours is 6.87% worse on this workload.** Each of the four adjacent paired-block
differences is positive, ranging from 4.85% to 7.44%. This is a measured current
regression, not an unresolved claim about whether memory was measured.
PSS includes the main process and exclusively attributed WebView renderer;
unassigned OS/GPU/display services are outside this scope. Swap is not added
to resident PSS.

Per-process profiling places the excess in the renderer: our lower main-process
footprint does not offset its higher private anonymous/code memory. The observed
JavaScript heap and decoded images do not account for that entire difference.
The tested mobile browser-speech initialization change was **not retained**:
its controlled ABBA probe showed no memory reduction. Its Library-to-reader
workflow and CDP/Fetch instrumentation differ from the accepted restored-reader
cohort, so their absolute totals are not pooled.
The [rejected-experiment receipt](benchmarks/2026-11-04-current-browser-speech-experiment.json)
preserves the negative result; no rebuilt memory or speed gain is claimed.

The [complete Android receipt](benchmarks/2026-11-04-current-app-comparison.json)
includes APK/archive/SDK/WebView identities, every accepted PSS sample and
per-process generation, raw rollup fields, video PTS, bounds, target identities
and exclusions. Original videos, screenshots, detailed `meminfo` and command
receipts remain in `/private/tmp/kiwix-current-android.c1yx6zuv/`.
Preliminary post-import memory is excluded: it gave a misleading favorable
result before process-cold controls.

### Android EMULATOR: rendered StreetZIM memory is effectively tied

The real Hawaii map was fully loaded in both apps, with the same zoom, bearing
and pitch and centers within one device pixel. Both used Home followed by a
real pan to persist the camera. App chrome gives them different canvas heights;
this is not an equal-viewport tile/FPS benchmark. The eight counterbalanced
blocks contain 16 samples per app, with real rendered-map evidence before and
after sampling.

| App-attributed map memory | Ours | Official Android |
| --- | ---: | ---: |
| Median main + attributed renderer PSS, MiB | 193.69 | 193.20 |
| Observed PSS range, MiB | 192.98–195.03 | 192.36–202.77 |
| Median swap PSS, MiB, reported separately | 1.69 | 1.72 |

The point difference is 0.26% worse for ours, but paired-block directions
reverse and observed variation is much larger. Treat this as a **tie within
this experiment's variation**, not a demonstrated map-memory advantage.

### Android EMULATOR: measured title-search responsiveness

Silent device video shows actual **Animal / Animal Characteristics** result
rows after the same `Animal` query, scoped to Wikipedia. The final quiet
**BAAB** series has two recordings per app and excludes the earlier
compiler-overlapping series. Values below are medians of first-ready-frame
upper endpoints; the receipt retains each last-negative/first-ready interval.

| Visible query → actual result rows, ms | Ours | Official Android |
| --- | ---: | ---: |
| First visible input → rows, median | **132.66** | **1,146.71** |
| First visible input → rows, observed range | 123.44–141.87 | 985.30–1,308.12 |
| Complete query visible → rows, median upper endpoint | 112.27 | 646.03 |
| Complete query visible → rows, observed upper endpoints | 82.68–141.87 | 578.99–713.08 |

**Ours is faster in this measured title-search cohort.** These are
user-visible frame boundaries, not native-ABI timings or CPU-only execution.
No debounce was subtracted; however, input handling and some debounce may
precede the frame where the complete query becomes visible. The intervals
therefore do not measure physical-keystroke latency or the full debounce
duration. Two recordings per app do not establish a general percentile or
physical-phone result.

Our real `multicellular` full-text query also displayed about ten results,
including Eukaryote and Virus with snippets. Official Android 3.14.1 has no
equivalent native indexed full-text submission surface in the inspected path;
there is no invented counterpart full-text latency.

### Android EMULATOR: real article-ready startup, one valid paired observation

The valid recordings start at an unambiguous on-device pointer-location tap
marker on the actual launcher icon and end at **Animal's body, montage and
classification content**, confirmed by later matching frames.

| Visible launch tap → article-ready frame bound, seconds | Ours | Official Android |
| --- | ---: | ---: |
| Accepted matched pair | **2.516–2.966** | **3.782–3.838** |

Ours was faster in this pair; this is **not a launch-time distribution**.
The API 33 ARM64 emulator uses SwiftShader and on-device video recording, so
these are instrumented emulator observations, not shipping-phone startup
times or first paint. Original variable-frame-rate PTS and durations are
retained. Earlier white-on-white ambiguous touch markers, censored clips and
multi-second host-PNG-observer bounds are excluded from this comparison.

### macOS: lower measured Wikipedia footprint

The current optimized `5eec1e4` app and an official **3.18.0 DMG-derived,
identity-resigned clone** both opened the real Wikipedia archive's index and
followed its Animal link through the UI. Each had one reader tab and a verified
1100 × 680 window. Actual article text was verified before and after each
cohort. After at least 30 seconds foreground settling, ten samples per app
span approximately 18 seconds. The host reported 83% free memory; observed
compressed/swapped process ledger points were zero.

| Median summed process footprint, MiB | Ours | Official-release-derived |
| --- | ---: | ---: |
| Main + all attributed WebKit processes | **191.93** | **208.53** |
| All observed coalition members, including auxiliary services | 217.08 | 254.97 |

**Ours is 7.96% lower for main plus WebKit in this cohort.** The broader sum is
14.86% lower, but includes auxiliary OS services and is not the same scope.
These are summed per-process `physFootprint` ledgers, **not PSS or a
deduplicated whole-system physical-memory total**. This is one matched cohort,
not repeated counterbalanced memory blocks or proof across all archives.

The baseline clone preserves all 94 compared file-backed non-LINKEDIT
sections across both architectures; its bundle identity and ad-hoc signature
changed for isolation. This is not unmodified shipping-identity acceptance.
Normal user app profiles were not used.

Four process-cold AX-observed launches completed actual Animal article
readiness. Their last-negative/first-positive intervals overlap and include
observer overhead, so no precise first-paint or startup-speed ranking is
claimed. Both apps also produced an actual Eukaryote search result. Their
search modes differ, so those observations establish functionality, not
comparable search latency. Earlier unmatched navigation/history cohorts and
locked-screen observation failures are excluded.

The [Mac receipt](benchmarks/2026-11-04-current-mac-comparison.json) records
artifact/archive identities, process generations, metric scopes, observation
bounds and exclusions. Original private evidence remains in
`/private/tmp/kiwix-apple-current.qkn6YWC1/`. Both owned measurement apps were
normally quit and the Mac released after the finite batch.

### iOS: mixed physical memory results; simulator launch lower, app size larger

#### Simulator main-process memory: ours lower, whole-reader total unavailable

Both installed release apps completed the real Wikipedia → Animal search/body
→ Hawaii map Home/zoom/pan workflow. The matched camera-reset run passed UI,
state and preservation checks in **268.61 seconds**, including cleanup.
Screenshots show actual corresponding Hawaiian geography in both apps:
Home at 50 mi scale, zoom at 30 mi, then Oahu/Maui shifted by the same pan.
This is workflow proof, not an app-speed measurement.

After normal foreground activation, each app received a 30-second settling
sleep and three generation-stable host `proc_pid_rusage` captures:

| Host-side iOS Simulator metric | Ours | Upstream Apple 3.18.0 |
|---|---:|---:|
| Main-process physical footprint | **61.38 MiB** | **90.58 MiB** |
| Main-process resident size, separately diagnostic | 394.91 MiB | 526.84 MiB |

Our **main-process** physical footprint was **32.24% lower** in this one
simulator session. It is not evidence of physical-iPhone or whole-reader
memory savings. Both app roots share the simulator's host resource and
jetsam coalitions with system services; summing them would count the other
app and OS processes. No WebKit-service allocation or aggregate is claimed.
The corrected census generation-validates that competing root; the earlier
non-main census is excluded from affirmative ownership proof.

Actual activation-to-first-capture delays differed: approximately 37.66 and
55.78 seconds after equal 30-second sleeps. All three main-footprint samples
were identical for each app. This is idle-state diagnostic evidence, not
peak memory or an independent replication series. Our profile had two books
and two tabs; native had the same books and three tabs, including its normal
empty tab.

The [memory receipt](benchmarks/2026-11-05-iphone-simulator-memory.json)
retains exact bytes, generations, source versions, timing caveats and raw
evidence hashes. `scripts/profile-ios-simulator-memory.py` is a read-only
collector; the caller owns activation and settling. Standard `footprint`
auxiliary `phys_footprint` independently matched the upstream main value.

#### Simulator first-responsive-frame launch: ours shorter in both batches

Official `XCTApplicationLaunchMetric(waitUntilResponsive: true)` produced
three retained process-cold launches per app, after discarding the first:

| Median first-frame-plus-responsive duration | Ours | Upstream Apple 3.18.0 |
|---|---:|---:|
| Original batch | **0.884 s** | **1.575 s** |
| Separate fresh-tool-preparation repeat | **0.855 s** | **1.569 s** |

The original median interval is 43.9% shorter for ours; original means were
0.900 and 1.575 seconds. Do not pool the two batches. Each used grouped
ours-then-upstream order and uncontrolled OS caches. Every iteration required
the app to be terminated before launch. This is **first displayed frame plus
main-thread responsiveness**, not WebView/archive content readiness, storage-cold
startup, search speed or physical-iPhone performance.

The [original launch receipt](benchmarks/2026-11-05-iphone-simulator-launch.json)
and [separate repetition](benchmarks/2026-11-05-iphone-simulator-launch-repeat.json)
retain each batch's samples and exact metric boundary. The reusable
`scripts/profile-ios-simulator-launch.py` prepares its dedicated runner from
tracked sources and executes it with bounded cleanup; no app rebuild or reset
is required. Fresh preparation and a second actual run both completed.

#### Source-built device app size: ours larger

Measured the actual signed release device bundles installed in the last
physical attempt, then packaged both with identical unencrypted developer
IPA ZIP DEFLATE-level-6 settings:

| Device-build storage metric | Ours | Upstream Apple 3.18.0 |
|---|---:|---:|
| Regular-file logical `.app` size | **48.91 MiB** | **26.61 MiB** |
| Developer IPA container | **21.93 MiB** | **8.17 MiB** |

Ours is **83.82% larger** as a logical app bundle and **168.45% larger**
under that developer-IPA compression method. These are not App Store
encrypted/thinned downloads or physical installed app-plus-data sizes.
The upstream baseline is source-built, developer signed, with the approved
Apple Pay merchant-entitlement omission; reader sources are unchanged.

Simulator data-container totals are not a fair storage winner: the harness
retains incoming ZIMs, an abandoned Wikipedia staging file and our managed
copies, while native opens its staged originals. The
[storage receipt](benchmarks/2026-11-05-iphone-storage-comparison.json)
separates those payload categories, non-ZIM data, allocated/logical sizes,
artifact identities and the verified developer-IPA byte counts.

Recorded XCTest search/body intervals contain expensive AX work and
different search-trigger boundaries. They are **not** calibrated app launch
or search-speed benchmarks. Simulator metrics cannot replace physical
memory/performance measurements.

The [simulator verification receipt](benchmarks/2026-11-05-iphone-simulator-verification.json)
records the exercised workflow, exact source/artifact bindings, rendered-map
admission and preservation/cleanup results.

#### Physical iPhone: paired Wikipedia memory is mixed

On the same iPhone 17 Pro Max running iOS27.0.1, the baseline-first run
captured three stable observed-coalition samples per checkpoint after a
30-second settling interval:

| Physical checkpoint | Ours | Upstream Apple 3.18.0 | Ours minus upstream |
|---|---:|---:|---:|
| Wikipedia100 home | **223.71 MiB** | **166.98 MiB** | **+56.73 MiB / +33.98%** |
| Animal body after search | **310.98 MiB** | **333.56 MiB** | **−22.58 MiB / −6.77%** |
| Hawaii after Home/zoom/pan | Unavailable | **458.11 MiB** | No pair |

These are medians of **observed kernel-coalition process footprint-ledger
sums**, including main, WebContent, GPU, Networking and other observed helpers.
They are not main-only measurements, PSS, deduplicated RAM, peak memory or
proof of every shared OS service. Compressed memory is separate, not added
again. One session with three repeated samples is not independent replication.

Actual screenshots admit Wikipedia home and Animal in both apps, and rendered
Hawaiian geography in native. Our existing profile began with four books and
two tabs; isolated native began with zero books and one empty tab. Real
archive/workload matching therefore does **not** establish identical
library/history/tab state or a causal framework-overhead comparison.
Existing own Wikipedia admission checked UUID/size/path, not a fresh
phone-side byte hash. Host fixture hashes and exact component totals are in
the [physical comparison receipt](benchmarks/2026-11-05-iphone-physical-comparison.json).

Our map intake then failed. The saved failure hierarchy contains the actual
app alert **“The archive could not finish closing its document.”** The absent
Reader navigation control is a consequence of that blocked state, not proof
of a wrong selector. No retry or alert suppression was used. Automation
stopped and the phone was immediately released after **410.28 seconds**,
including confirmed cleanup. Final whole-workload preservation validation
and our map-memory checkpoint were not reached.

Conclusion: ours was worse at home and lower on Animal in this session;
there is no overall winner. Our WebContent component was about 63 MiB higher
at home and 58 MiB higher on Animal, an attribution target rather than a
proven avoidable allocation. Calibrated physical launch/search latency remains
unmeasured; AX observation intervals are deliberately excluded.

#### Earlier physical iPhone attempt: our first checkpoint retained

The new filtered sampler passed readiness and captured three stable samples
of our actual Wikipedia home on the iPhone. The observed kernel-coalition
footprint-ledger median was **235.10 MiB**, including main, WebContent, GPU,
networking and the other observed helper—not merely our main process.
Compressed memory was recorded separately, not added again. This sum is not
PSS, deduplicated RAM or proof of every shared OS service.

The native reader's 333 MB fixture-copy command then timed out at 35 seconds
over the recorded local-network connection. No progress evidence distinguishes
slow transfer from a stall. The phone was immediately released after
**147.00 seconds**. The [physical checkpoint receipt](benchmarks/2026-11-05-iphone-physical-first-memory.json)
retains its scope, components and evidence hashes. That attempt alone supplied
no paired winner; the later paired measurements above supersede that gap.

The next explicitly authorized, baseline-first USB attempt completed fixture
staging but exposed a CoreDevice argument-ordering bug: `--device` followed
the positional app bundle and was consumed as an application argument.
It stopped and immediately released the phone after **42.22 seconds**.
The command builder now puts device/output/timeout options before positional
arguments; an offline regression exercises the actual vendor parser without
device transport. Simulator success had not covered that physical CLI path.

#### Earlier physical paired attempt: first memory sample timed out

The independently reviewed prepared command installed both our exact
`3cce5af…` release and the separate source-built upstream Apple 3.18.0
baseline, plus the comparison runner. Installation commands took **4.474**
and **3.254 seconds** respectively. Our existing real Wikipedia tab was
selected through ordinary UI, and the saved checkpoint screenshot visibly
shows the actual Wikipedia home article grid.

The **first memory command timed out after 12 seconds**, leaving an empty
sample file. The run stopped and confirmed host/test-runner cleanup; the
phone was immediately released after a **90.53-second total invocation**.
**No memory sample, native Wikipedia checkpoint, Animal search or Hawaii
map interaction completed.** This is not a paired performance result.
The [paired-attempt receipt](benchmarks/2026-11-05-iphone-paired-attempt.json)
retains exact app/runner identities and evidence hashes.

The stalled operation is unresolved: the original command wrapper discarded
partial timeout diagnostics. PMD 9.36 creates its output after resolving its
service provider, and its single-snapshot path performs sequential
executable-name lookups for all processes before writing JSON.
**[INFERENCE]** DVT startup or Wi-Fi enrichment cost could exceed the
12-second command budget; the empty file does not establish either cause,
nor prove that USB selection caused this failure.

The owner requested simulator-first verification before another phone
window. Simulator UI/fixture/checkpoint proof will remain separate from
physical-device profiling-transport and memory proof.

#### Earlier bounded installation/reader attempt

The new `npm run iphone` command installed the independently reviewed,
development-signed ARM64 release from exact committed source
`3cce5af5677350996301b6cc335b30a5a00d4938`, without dirty working-tree
overlays. Its main executable SHA-256 was
`d2948ecb419b37a9495892d600df4c70b002d4545c90c95958d8ada5ebd3050b`.
The in-place installation command took **2.687 seconds**. The entire
wrapper invocation took **71.67 seconds**, including preparation validation,
installation, the failed UI attempt and cleanup—not app startup time.

The signed host app contains **51,280,691 regular-file logical bytes**;
**50,710,848 bytes** are its main executable. This excludes archives,
mutable data, filesystem overhead and OS WebKit; it is not a phone's
installed-storage or thinned App Store size. The
[bounded-attempt receipt](benchmarks/2026-11-05-iphone-bounded-attempt.json)
binds the artifact, timing observations, failure and private-evidence hashes.

The real **Wikipedia 100** main page rendered; the recovered test recording
visibly contains its article grid, including Animal, Bird and the White
House. Automation then failed because an exact **Search books** query
restricted to XCTest's button role returned no match, despite a visible
search icon. The cause is not established. This is a test-observation
failure, not evidence that manual search is broken or that accessibility
passes.

The run stopped at that first unexpected state. Its matching-device receipt
confirmed both host automation cleanup and absence of the remote test
runner, and the phone was released. **No Animal search, Hawaii map rendering
or memory measurement completed.** Hawaii's source was staged, not admitted.
The observed **5.025-second** activation-to-Library-button interval is an
instrumented upper bound, not calibrated cold-start latency.

Private evidence is retained as
`acceptance-4840eec9f24d477c9640ef2ddd2e4e59/` under the tool's ignored
run directory. Its original XCTest result bundle was incomplete after the
failure; the rendering observation comes from the recovered raw recording,
not a successfully exported complete test report. Existing library/settings
preservation was verified **before launch**, with only the verified managed
container-path rebase allowed; no post-workload exact-state claim is made.

A clean **source-built Kiwix Apple 3.18.0** baseline has been built and signed
from official revision `09a5ff1c9c3ffeda258ad593bbe16da612cfb149` for the same-device
comparison. It uses a separate developer identity and, with owner approval,
omits only the Apple Pay merchant entitlement that the available signing
profile cannot authorize. It is **not an App Store binary**, and no new
paired iPhone memory or timing result is available.

#### Earlier admission and selector attempts

The owner deleted our app for a clean test. The current reviewed build was
reinstalled, and the same real Wikipedia file was staged for it and the
installed Apple **Maps Beta 3.18.0 / 259**. That beta is a developer baseline,
not the shipping App Store app.

The actual intake attempt failed at **Read originals**: a developer-supplied
file URL does not carry UIKit's required open-in-place attestation. Native
refusal preserved the permission boundary, but the UI incorrectly highlighted
an unusable choice and displayed a raw provider error. No matched reader
cohort completed. Installation, file staging, consent-dialog memory and
background-app memory are **not** iPhone performance results.

The reviewed correction was then installed in place: a file-specific
**Add to library / Cancel** flow, plain storage consequences and supported
secondary storage choices. Captured existing book records, reader tabs,
bookmarks and preferences were preserved; the managed-download directory
rebased to the new OS container UUID. A cold developer handoff did not display
the dialog. A warm handoff did, and actual XCTest tapped **Add to library**.
The stored library then contained the real Wikipedia UUID and a managed file
of 333,429,922 bytes; no archive-byte readback hash is claimed.

The automation subsequently failed at its incorrect **Open Wikipedia**
selector: the actual button was **Open Wikipedia 100**. This is successful
admission but not an article-ready or performance result. The phone was
released rather than held idle during host-side correction. Both apps'
reader tabs may now be closed for the matched test with owner authorization;
archives, bookmarks and settings must remain intact. No matched iPhone
memory, startup or search-time comparison has completed.


## Findings

- **Android, original EMULATOR comparison cohort:** our corrected release APK was **83,808,299 bytes**, versus **70,366,062 bytes** upstream: **19.1% larger**. Observed installed code/resources were **85,635,655 versus 77,055,894 bytes**. Neither includes the shared test ZIM. The current efficiency result supersedes this size status, not this runtime cohort.
- **macOS:** our ARM64 release `.app` is **97,929,512 regular-file bytes**, versus **19,352,045** for the official universal Apple DMG app: **5.06× larger**, despite our single architecture. This remains a substantial size gap.
- **Android EMULATOR runtime, real archives:** on Wikipedia's **Animal** article, our Activity Manager launch median was **287 versus 543 ms**, while main-plus-attributed-renderer PSS was **10.0% higher**. A separately accepted rendered Hawaii StreetZim overview used **18.8% higher** attributed PSS. These are not first-paint, unrestricted whole-app-memory or physical-phone measurements.
- **iOS:** the historical unsigned ARM64 archive and current development-signed bundle/IPA are separate measured forms. Official Apple download metadata is not an official IPA or matched installed-code measurement; no official size ratio is inferred.
- **Windows:** the official Qt ZIP and its expanded contents are measured. Our Windows package and runtime remain unavailable.

All MB in explanatory prose are decimal; memory tables explicitly use KiB. Download containers, expanded bundles, installed code, mutable data and OS-provided engines are not interchangeable measurements.

## Android package and installed storage

The upstream [official ARM64 APK](https://github.com/kiwix/kiwix-android/releases/download/v3.14.1/v3.14.1-arm64-v8a-standalone.apk) was downloaded and its actual SHA-256 matched the [release asset metadata](https://github.com/kiwix/kiwix-android/releases/tag/v3.14.1). Both artifacts contain **ARM64 only**. Comparing against upstream's multi-architecture universal APK would conceal our size gap.

| Measurement | Ours, corrected release | Upstream 3.14.1 standalone ARM64 |
| --- | ---: | ---: |
| Actual signed APK bytes | 83,808,299 | 70,366,062 |
| Installed code/resources, logical file bytes | 85,635,655 | 77,055,894 |
| Mutable credential-encrypted data after representative workflows, logical bytes | 23,105,902 | 16,365,575 |
| Device-encrypted data file bytes | 0 | 0 |
| App-specific external file bytes | Directory absent | 0 |
| Current/reference ART profile file bytes | 4,235 | 8,842 |
| Sum of the enumerated app roots, logical bytes | 108,745,792 | 93,430,311 |
| Sum of the enumerated app roots, allocated KiB | 110,684 | 92,204 |

**Installed-code scope:** the selected `/data/app/.../` directory contains `base.apk`, ARM64 `base.vdex` and `base.odex`. Native libraries remain in the APK; there is no separately counted extracted-library copy. ART output depends on device/OS/compiler state. Data rows are final snapshots after real Wikipedia/StreetZim reading, search, restart and recovery interactions—not immutable installation requirements or a controlled cache-growth ranking. Ours and upstream did not perform identical search/recovery work. An empty directory can consume allocated blocks despite containing zero regular-file bytes.

Storage inventories separately cover `/data/user/0/PACKAGE`, `/data/user_de/0/PACKAGE`, app-specific external storage and current/reference ART profiles. They exclude shared OS/WebView packages, package-manager/system-wide accounting, filesystem metadata outside those roots and the **484,034,937 combined archive bytes in public Downloads**, referenced rather than copied into app-specific roots. They are not the Android Settings storage panel's unrestricted total.

Both representative-cohort installations used ordinary **Push Install**, explicitly disabling incremental and streaming installation. Installed APK pulls matched the frozen source APK SHA-256. Earlier small-archive mutable-data and incremental/sparse-install snapshots are not carried into this table.

### What was wrong with the enormous APK

The previous **554,758,953-byte APK was debug**, not a shipping-size baseline. Its native library contained **366,726,522 bytes of debug sections** plus **64,378,807 bytes of symbol/string tables**. These are not all resident memory. An earlier malformed/incrementally enlarged debug ZIP was separately excluded; it is not the optimized result.

The first current optimized APK was still **102,420,523 bytes**. Its native ELF retained **18,611,674 bytes** of `.symtab` and `.strtab`, although debug sections were gone. `src-tauri/Cargo.toml` now sets:

```toml
[profile.release]
strip = "symbols"
```

The rebuilt, signed APK is **18,612,224 bytes smaller**. The final library has no `.symtab`/`.strtab`; `.dynsym`/`.dynstr` and required JNI exports remain. Actual native launch, local article rendering, replay navigation, Back and saved-reader restart passed. APK signatures and 16 KiB ZIP alignment passed. This is a measured rebuild, not subtraction used to predict a release size.

**Tradeoff:** release backtraces/profilers lose local symbol names. Debug builds retain them. Earlier binaries are not matching offline symbols for a new release; retain exact-build symbol artifacts when establishing a production crash-symbolication pipeline. No optimization-level, LTO, panic or runtime validation setting changed. This is a workspace-wide release policy, not an Android-only setting.

Our final APK is optimized/non-debuggable but signed with the local **Android Debug development certificate** for this isolated installation. Signature identity does not make it a debug build; it is also not a production-signed/Play-distributed release. No physical phone was updated in these benchmark runs.

## Apple and Qt desktop size baselines

| Platform/form | Our measured artifact | Existing official app |
| --- | --- | --- |
| macOS expanded `.app`, regular-file logical bytes | **97,929,512**, ARM64, local ad-hoc release | **19,352,045**, Apple 3.18.0 Developer-ID distribution, universal ARM64+x86_64 |
| macOS compressed distribution | No current shipping DMG/download measured | **6,190,489-byte** [official 3.18.0 DMG](https://download.kiwix.org/release/kiwix-macos/kiwix-macos_3.18.0.dmg) |
| iOS expanded app bundle, regular-file logical bytes | **56,965,578**, ARM64 optimized **unsigned archive**; not installed by this run | No official IPA/installed bundle measured |
| iOS download metadata | No App Store/thinned download measured | **19,074,048 bytes**, [Apple lookup metadata](https://itunes.apple.com/lookup?id=997079563&country=us), not a downloaded IPA or device-specific installed size |
| Windows x64 release distribution | No our Windows package | **157,265,310-byte** [Qt 2.5.1 ZIP](https://download.kiwix.org/release/kiwix-desktop/kiwix-desktop_windows_x64_2.5.1.zip); **336,453,400** expanded regular-file bytes |
| Linux ARM64 application-only package | No matched optimized Linux package measured here | **538,946-byte** [Noble 2.5.1 DEB](https://ppa.launchpadcontent.net/kiwixteam/release/ubuntu/pool/main/k/kiwix/kiwix_2.5.1~noble_arm64.deb); **1,695,121** own payload bytes, **excluding runtime dependencies** |

The Mac DMG's actual MD5 matched its published sidecar. Its app was inspected through an owned read-only/no-auto-open mount, then detached. The existing `/Applications/Kiwix.app` App Store variant was inspected without launch: **3.18.0 / 20261101.0205**, **19,355,498 regular-file bytes**, distinct from the DMG's **20261031.2304** build. Both have universal executables; their whole-app sizes are not architecture-thinned ARM64 sizes.

Our final Mac executable is **56,238,256 bytes**; the seven bundled dylibs include **33,177,136 bytes of ICU data**. The official DMG's entire universal executable is **17,562,640 bytes**. These observations identify substantial components of our footprint; they do not establish a safe language-data reduction or attribute every byte to Rust. Dropping locale/search data without compatibility evidence is not a size fix.

Bundle sums exclude symlinks, directory/filesystem overhead, app-created data and archives. Mac/iOS use OS WebKit, outside these bundles. The Windows ZIP includes bundled Qt6/WebEngine, resources and `vc_redist.x64.exe`; its expanded total is not a clean Windows machine's installed footprint. Its MD5 matched the official sidecar and member CRCs were checked. The Noble ARM64 DEB actually declares **Qt5** dependencies, unlike the Windows Qt6 package; its tiny own payload must not be compared with a self-contained app as if it included that dependency closure.

Qt Kiwix Desktop officially targets Linux and Windows. The official macOS app is **kiwix-apple**, not a Qt macOS release. Our iOS unsigned archive is build/size evidence only: no store compression, thinning, signing overhead, installation or current-release phone runtime is inferred. The phone still has the separately recorded development build.

## Representative Android EMULATOR runtime — historical cohort

### Environment and workloads

- Host: **M2 Max / Mac14,5, 64 GiB RAM, macOS 27.0.1 (26A434)**.
- One fresh owned **API 33 Google APIs ARM64** emulator, Pixel 6 definition, **2 virtual cores / 2,048 MB RAM / 1080×2400 / SwiftShader**. Google WebView **109.0.5414.123**, multiprocess enabled.
- The same frozen optimized release APKs used for the package comparison; installed identities were checked again. Separate package IDs, default appearance/reader settings, ordinary DocumentsUI selection. Upstream's normal all-files permission UI was exercised; its enabled switch was separately verified during the final title-search workflow.
- Airplane mode enabled and Wi-Fi disabled inside this emulator only. Existing emulator, Samsung and iPhone were untouched. Archive originals were preserved.

| Real archive | Actual bytes | Entries / articles | Indexes and workload |
| --- | ---: | --- | --- |
| [Wikipedia 100, English, 2026-08](https://download.kiwix.org/zim/wikipedia/wikipedia_en_100_2026-08.zim) | 333,429,922 | 9,320 / 5,056 | Title and full text; `mainPage` → `index`; **Animal**, image montage/classification table, Eukaryote navigation and indexed queries |
| Production Hawaii StreetZim, `osm_en_hawaii_2026-10-26.zim` | 150,605,015 | 724,178 / 10,329 | Title and full text; `mainPage` → `index.html`; real MapLibre map, **Honolulu** place search, selection, zoom and pan |

Wikipedia's actual bytes/hash match its official [Metalink](https://download.kiwix.org/zim/wikipedia/wikipedia_en_100_2026-08.zim.meta4). The existing production StreetZim was hashed before use. Archive hashes, accepted sample values, process generations, storage snapshots and method boundaries are retained in [the machine-readable receipt](benchmarks/2026-11-04-android-real-archives.json).

### Wikipedia: process-cold restored startup

Both apps restored **one tab on Animal**, not just library chrome. After a warm-up, five force-stop/`am start -W` runs per app, upstream group first, then ours; the opposite test package was force-stopped before each cohort. OS/filesystem caches were **not** purged. Final native screenshots visibly show the actual article and images.

| Activity Manager `TotalTime` | Ours | Upstream Android |
| --- | ---: | ---: |
| Median, ms | 287 | 543 |
| Observed min–max, ms | 271–385 | 516–586 |
| Successful recorded runs | 5 | 5 |

This is **Activity Manager's launch boundary**, not first paint, article-ready time or time until the user can interact. Our WebView can continue loading afterward. Grouped VM runs do not prove a physical-phone speedup, percentile distribution or storage-cold performance. Preliminary runs, first-run/onboarding and the interrupted AX-observation attempt are excluded.

### Main process plus exclusively attributed WebView renderer memory

After at least **30 seconds settling**, collect **10 samples**. Each sample sums sequential `/proc/PID/smaps_rollup` PSS reads for the main process and renderer whose `ProcessRecord.packageList` exclusively names that app. Main PID and process start ticks are checked; the selected process generations stayed stable before/after every sample.

| Accepted metric | Ours | Upstream Android |
| --- | ---: | ---: |
| Wikipedia Animal: median attributed PSS, KiB | 212,863 | 193,508 |
| Wikipedia PSS min–max, KiB | 212,695–213,018 | 193,442–193,584 |
| Wikipedia median swap PSS, KiB | 884 | 879 |
| Wikipedia first-to-last sample span, seconds | 35.01 | 26.71 |
| Rendered Hawaii overview: median attributed PSS, KiB | 230,543 | 194,048 |
| Hawaii PSS min–max, KiB | 230,074–230,984 | 194,026–194,072 |
| Hawaii median swap PSS, KiB | 1,484 | 1,491 |
| Hawaii first-to-last sample span, seconds | 52.69 | 58.00 |

Ours used **10.0% more attributed PSS for Wikipedia** and **18.8% more for the rendered Hawaii overview**. Main/renderer PIDs were **8978/9018 ours, 7428/7532 upstream** for Wikipedia; **15107/15148 ours, 16991/17100 upstream** for Hawaii. These are not unrestricted whole-app or system memory totals: shared/unassigned OS, GPU and display services are excluded. RSS is not summed as physical memory. Controller/ownership overhead means samples are not 1 Hz. This method is not pooled with the earlier main-only `dumpsys` series.

**Map boundary:** accepted Hawaii memory follows equivalent Home/real-user-pan overview interactions, with one map tab and two library references per app. Viewport heights/default chrome differ. Upstream's native reader Home recovered a blank restored canvas before its accepted rendered-memory cohort. This is **post-interaction rendered overview memory**, not matched restored-reader memory or restoration-speed evidence. Ours has pre/post-memory screenshots; upstream has a pre-memory rendered screenshot and a complete stable ten-sample receipt, but no post-memory screenshot: the controller deadline interrupted the later observation after memory/storage were written.

### Reading, indexed search and map interactions

Both apps visibly rendered **Animal** and **Eukaryote**, including real images and article tables. Both displayed actual `Animal` title suggestions and opened the result offline. Our `multicellular` full-text query returned **about 10 results**, including **Eukaryote**, with actual snippets; opening it rendered the article. These establish functionality, **not a navigation/search-paint latency ranking**.

Upstream's exact native [search generator](https://github.com/kiwix/kiwix-android/blob/v3.14.1/core/src/main/java/org/kiwix/kiwixmobile/core/search/viewmodel/SearchResultGenerator.kt) uses `SuggestionSearcher`, and [submission](https://github.com/kiwix/kiwix-android/blob/v3.14.1/core/src/main/java/org/kiwix/kiwixmobile/core/search/SearchFragment.kt#L243-L298) only opens a complete matching suggestion title. **FIND IN PAGE is not archive full text.** There is no indexed full-text submission surface in that native 3.14.1 path; see upstream [#1324](https://github.com/kiwix/kiwix-android/issues/1324). We do not invent a counterpart full-text timing. Apple already has combined search; Qt already has indexed full text.

Both apps rendered actual Hawaii vector-map content offline, returned **Honolulu city** place results, centered a selected pin and visibly changed map scale/position after zoom/pan. Earlier restored-map series had different saved cameras and blank/partial upstream canvases. Those receipts remain diagnostic evidence, **excluded from restored-startup rankings**. The archive intentionally saves user camera moves, not programmatic Home moves; a Home tap alone does not establish the next restored camera. No StreetZim first-tile, FPS or matched-restoration speed claim follows.

Native UIAutomator returned 137 once despite reporting an XML dump on the large Wikipedia article; earlier AX trees also omitted visibly rendered results. Screenshots supply independent surface evidence; failed AX/controller observations are not app failures or app-only latency measurements.

### Historical small-archive regression only

The earlier **330,278-byte Go by Example** workload remains replay-link regression evidence, not the primary performance baseline. Its five-run **276/514 ms** Activity Manager medians and **120,547.5/113,820 KiB main-only PSS** use different content and memory scope. They are not combined with Wikipedia/StreetZim results.

Large-library throughput, physical-phone/Apple/Windows runtime, first paint, search-ready latency, scroll FPS, CPU and energy remain unmeasured here.

## Replay parity defect found and corrected

A real Android tap on the archived **Hello World** link initially produced an external-browser confirmation instead of offline navigation. Upstream opened it offline. Real Chrome against our native Service reproduced the problem: Wombat replaces the DOM's `href` getter with the original web URL, while the browser's underlying target remains local.

The archive bridge now captures native `HTMLAnchorElement` and `HTMLAreaElement` URL accessors **before archive scripts execute**. Primary/middle click, link context, preview and descendant containment resolve the actual browser target, retaining `<base>` semantics. Shell origin checks, external confirmation, CSP and network denial were not weakened.

After correction: real Chrome and the rebuilt Android release rendered Hello World's unique body offline; native Back restored the main article; Chrome preview returned its actual excerpt; a genuine `https://go.dev/` link still required confirmation. Context-menu runtime proof was not obtained in this pass. **211 JavaScript tests and TypeScript checking passed**; the six affected suites account for 60 of those tests. Mac/Android release builds and the iOS unsigned release archive built successfully. Existing VM harness dependencies were updated; they do not replace the real replay-engine proof. The native/browser failure and passing screenshots remain the regression evidence, not a new permanent browser CI test.

## Feature comparison

The table is **pinned release-source coverage versus our current documented implementation**, not a blanket certified superset. Native acceptance for ours is scenario-specific in [FEATURES](FEATURES.md), [PARITY](PARITY.md) and [RELIABILITY](RELIABILITY.md).

| Area | Apple 3.18.0 | Android 3.14.1 | Qt Desktop 2.5.1 | Our meaningful distinction/boundary |
| --- | --- | --- | --- | --- |
| Offline reader/navigation/tabs | WKWebView, outline/Find/home/random, previews, opaque WebKit interaction-state persistence; native Mac windows/session | WebView, TOC/Find/home/random, tabs, saved WebView history/scroll | QtWebEngine, TOC/Find/home/random, tabs; optional URL-based session restoration | Same broad primitives; explicit portable history/workspaces and inert previews. Opaque native-state fidelity is not identical. |
| Archive search | Combined title/full-text across included archives | Native title suggestions; Find searches the current page | Title and indexed full text; inspected selection UI is single-book | Explicit title/full-text/combined modes and multi-archive search. Full text itself is not new over Apple/Qt. |
| Bookmarks/notes | Simple article bookmark schema/list | Bookmarks **and Take notes**, XML interchange | Stars/reading list with XML interchange; no note/tag/sync controls in inspected bookmark surface | Rich notes/tags, anchored highlights/comments, JSON/XML extensions and explicitly enrolled private reading-data sync. |
| Catalog/downloads | OPDS/Core Data, conditional refresh, OS-background HTTP pause/resume | Catalog/downloads; standalone versus Play storage paths differ | OPDS, libkiwix downloader/pause/resume | Paged/configurable catalogs, verified edition cutover/rollback, Metalink and explicit torrent/seeding workflows. No claim that Qt's backend cannot handle any torrents. |
| Archive sharing | Selected-library HTTP hotspot server/address/QR | Wi-Fi Direct transfer **and** HTTP hosting/address/QR | Embedded HTTP server | Discovery/inventory-driven encrypted QUIC acquisition plus HTTP/OPDS serving. Same-LAN Mac↔Android and Android/Mac↔iPhone discovery, catalog and download are verified; routerless phone-pair acceptance remains open. |
| Export/resources/print | Current text-page PDF, original resource sharing, native share sheets/Mac print | Image Save Media; existing PDF/EPUB Save/Open | Print/PDF and non-HTML resource download | Additional original HTML/MHTML/resource and native PDF/print paths; not first-ever export. Windows native runtime remains unverified. |
| Read aloud/media | No app-owned TTS control in inspected reader/menu/settings; OS services remain available; inline/PiP/background declarations exist | Article/selection Android TTS including pause/resume; fullscreen media | Optional Qt TextToSpeech, build/platform dependent | Integrated speech/media controls; installed voices, physical audibility/headset/lock-screen acceptance remain separate. |
| Maps | Can read prebuilt map ZIMs and bridge location; release `geo:` opens Apple Maps | Archive/WebView content, not evidence of our recipe compositor | Generic archive reader | StreetZim recipe composition, reuse and acquisition. Ordinary offline map reading is not our exclusive feature. |
| Localization/accessibility/distribution | Released native controls and translated resources; official App Store/DMG | Released native controls/resources and distribution | Released desktop distribution | Our 73 non-English catalogs remain partial; AT/provider/media and production-signing/store acceptance are incomplete. Locale count is not quality parity. |

Primary source anchors: Apple [search](https://github.com/kiwix/kiwix-apple/blob/3.18.0/SwiftUI/Model/SearchOperation/SearchOperation.mm), [bookmarks schema](https://github.com/kiwix/kiwix-apple/blob/3.18.0/SwiftUI/Model/DataModel.xcdatamodeld/DataModel.xcdatamodel/contents), [export](https://github.com/kiwix/kiwix-apple/blob/3.18.0/Views/Buttons/ExportButton.swift), [hotspot](https://github.com/kiwix/kiwix-apple/blob/3.18.0/Model/Hotspot/KiwixHotspot.mm), [reader menu](https://github.com/kiwix/kiwix-apple/blob/3.18.0/Views/Buttons/TabbarMoreMenu.swift); Android [reader](https://github.com/kiwix/kiwix-android/blob/v3.14.1/core/src/main/java/org/kiwix/kiwixmobile/core/main/CoreReaderFragment.kt), [resource actions](https://github.com/kiwix/kiwix-android/blob/v3.14.1/core/src/main/java/org/kiwix/kiwixmobile/core/main/KiwixWebView.kt), [TTS](https://github.com/kiwix/kiwix-android/blob/v3.14.1/core/src/main/java/org/kiwix/kiwixmobile/core/main/KiwixTextToSpeech.kt), [transfer](https://github.com/kiwix/kiwix-android/blob/v3.14.1/app/src/main/java/org/kiwix/kiwixmobile/localFileTransfer/LocalFileTransferFragment.kt); Qt [search](https://github.com/kiwix/kiwix-desktop/blob/2.5.1/src/suggestionlistworker.cpp), [selection](https://github.com/kiwix/kiwix-desktop/blob/2.5.1/src/multizimbutton.cpp), [reading list](https://github.com/kiwix/kiwix-desktop/blob/2.5.1/src/readinglistbar.cpp), [exports](https://github.com/kiwix/kiwix-desktop/blob/2.5.1/src/webview.cpp), [server](https://github.com/kiwix/kiwix-desktop/blob/2.5.1/src/localkiwixserver.cpp).

Official Apple already has background URLSession, relinking, integrity checks, unlink/delete separation, native share sheets and Mac window groups. Android already has notes, saved tab state and offline sharing. Those are not counted as unique additions. Local upstream checkouts contain later modifications; feature findings use the exact release tags/downloaded Qt source rather than treating those modifications as shipped.

### Same-LAN sharing acceptance — 2026-11-06

The physical Mac, Android and the dedicated iPhone 13 Pro test device were on the same
`192.168.0.0/22` LAN. Senders used **Share all**; receivers opened **Nearby**,
selected the sender, and downloaded through the shared catalog. This is
observed behavior, not a source-only claim:

| Flow | Observed result |
| --- | --- |
| Mac → Android | Android discovered **Mac · Shared library · 3 archives** over DNS-SD, opened the catalog, and completed `StreetZim Demo` (`osm_en_testland.zim`, 36,196 bytes). |
| Android → iPhone | iPhone discovered the Android library, opened the catalog, and the Android reported **6 archives · 4 connected devices · 362.5 MB sent**. `osm_de_california.zim` failed only after the Android share was stopped mid-transfer; the final app now explains that interruption instead of showing the raw QUIC text. |
| Mac → iPhone | iPhone discovered the Mac, opened the catalog, and completed `osm_en_testland.zim` (36.2 kB sent by the Mac). |
| Cancel | `testNearbyCancel` passed on the dedicated iPhone 13 Pro test device: opening a shared catalog and cancelling left no duplicate peer and released the action gate immediately. |

The explicit share snapshot remains: archives received through Nearby are
**not** added to an already-running share. Session-specific DNS-SD instances
and the optional `gen` TXT let the receiver keep older sessions selectable
under a disclosure instead of silently hiding the real sender; unauthenticated
`gen` is never used to revoke the only selectable row.

**Remaining acceptance boundaries:** a full routerless Android-hotspot →
iPhone transfer was not recorded through XCTest because the iPhone leaves the
Mac's LAN when it joins the AP (the runner loses its control link); the
Android **Use temporary Wi-Fi** path, AP route binding and iOS join prompt are
implemented and were exercised up to the system join. Android hotspot
join/cancel/route-loss, reverse-direction AP-only transfer, phone-to-phone
throughput, Mac/desktop AP joining and iOS↔iOS cross-network sharing remain
open. A peer on a different subnet (including a VPN source) is refused by the
Android library listener (`local=false`) and needs the temporary-Wi-Fi path.
The Mac remains LAN/Bonjour-only; it cannot host an app-owned hotspot.

Adversarial review controls now include a persisted monotonic library
generation (not wall-clock), a two-connection per-remote-IP cap plus 5 s
pre-request setup bound, a 120 s Android hotspot-start timeout that releases
the reservation, and a sender-side temporary-network card with SSID,
passphrase and Wi-Fi QR. The remaining acceptance work is the full routerless
Android-hotspot → iPhone transfer and Android hotspot join/cancel/route-loss,
reverse-direction AP-only transfer, phone-to-phone throughput, Mac/desktop AP
joining and iOS↔iOS cross-network sharing; the Mac remains LAN/Bonjour-only.

## Functionality-preserving improvement criteria

These measurements are engineering baselines, not acceptable reasons to remove
languages, indexes, offline assets, sharing, background work or safety checks.
An optimization must preserve the same observable capability and ownership
contract, then demonstrate its claimed benefit against a frozen artifact.

| Priority / area | Concrete work | Acceptance and tradeoff |
| --- | --- | --- |
| Release payload | Keep the measured symbol-stripping correction; investigate duplicate frontend/native-data payloads and release link optimization before pruning anything. | Compare actual same-ABI APK/app bytes and installed code; preserve JNI exports, all locales, indexed multilingual search, native/headless readers and 16 KiB alignment. Do not call an experimental linker setting a saving before measuring its output. |
| Mac native dependencies | Audit the 33.18 MB ICU data library, duplicate versions and SDK/link composition. | Preserve Unicode normalization, collation, case/diacritic search and every supported language. No locale-data removal or unverified reliance on private OS ICU APIs. |
| Startup | Trace optional-service initialization, restoration and frontend work; defer only work that is genuinely unnecessary until first use. | Retest fresh and restored readers, visible search/article readiness, pending-transfer recovery and cold external intake. Faster Activity Manager completion alone is insufficient. |
| Memory | First attribute renderer/GPU/utility processes; inspect eager services and archive/index/DOM retention by owner and lifetime. | Report whole-app PSS where attributable, main PSS/RSS and swap separately. Release inactive resources only when live tabs, providers, downloads, search cancellation and background media retain their required authority. |
| Frontend/catalog data | Measure duplicated embedded assets and selected-locale parsing. Existing locale chunks are already loaded lazily; do not reimplement that as a new optimization. | Keep all offline catalogs, deterministic fallback/provenance and native locale support. Account for decompression/CPU/temporary-memory costs, not download size alone. |
| Continuous comparison | Freeze release identity, archive hashes, device/WebView version, process state, sample spans and readiness boundary. | Reject stale-AX/controller-overhead timings; retain failed-before/passing-after evidence and run the actual changed surface. No benefit claim from source inspection or unlike debug/release packages. |

The next feature work remains the concrete gaps/acceptance gates below, not
relabeling implemented upstream features as novel or disabling difficult paths.

## Measured follow-on optimization: defer full-text initialization

`src-tauri/native/archive.cpp` now opens ordinary paths, retained descriptors and
Windows leased paths with `zim::OpenConfig().preloadXapianDb(false)`. Full-text
indexes remain available; the patched SDK initializes the database on first
full-text use under its existing synchronization. Title search, snippets,
validation, resource bounds and ownership/destruction order are unchanged.

A separate macOS native-ABI probe ran **24 independent processes**: three
repetitions per real archive/opening mode/before-or-after variant, reversing
variant order in the middle repetition. Both index flags, metadata, full-text
results/snippets and complete article byte hashes matched. Retained-mode queries
worked after closing the caller descriptor; article hashes also matched an
independent libzim read of the original archives.

| Native probe: median current RSS immediately after open, KiB | Before | Deferred |
| --- | ---: | ---: |
| Wikipedia, pathname | 17,424 | 16,512 |
| Wikipedia, retained descriptor | 17,344 | 16,640 |
| StreetZim, pathname | 22,768 | 21,824 |
| StreetZim, retained descriptor | 22,704 | 22,224 |

Median probe open-operation times were approximately **0.12–0.21 ms shorter**.
These are small-sample **host probe-process** observations, not Android app-PSS
or startup improvements. Query timing includes ABI return/copy/free/JSON handling;
retained open includes Python descriptor/stat setup. First full-text timing and
post-search RSS were mixed; Wikipedia/path post-search RSS increased. No
post-search saving or statistically established phone speedup is claimed.
The first real query now bears the deferred initialization cost, and the database
remains retained afterward.

The rebuilt optimized ARM64 Android APK is still **83,808,299 bytes**,
SHA-256 `f285a5244a8095ec8d9501ecda05b0f0335b2d28d37bfffe99fde30ee921259c`.
Its locally development-signed v2/v3 signatures and 16 KiB ZIP alignment passed;
the installed APK hash matched. Actual offline title search, first/repeated
`multicellular` full-text snippets, Eukaryote navigation/restart, and Hawaii
Honolulu selection/zoom/pan passed in the owned emulator. No normal phone update
or new Mac/iOS packaged acceptance is implied. Raw probe-operation values and
the separate smoke identity are in [the optimization receipt](benchmarks/2026-11-04-deferred-index.json).

## Measured follow-on optimization: one release codegen unit

An isolated source snapshot of `468b44f` changed only
`profile.release.codegen-units = 1`. The actual root Rust compiler invocation
included `-C codegen-units=1`; optimization level, unwinding, crate types,
features, locales and native SDK inputs were preserved. This trades release
compilation parallelism for cross-unit optimization. Debug builds are unchanged.

| Matched ARM64 artifact | Before | One codegen unit | Reduction |
| --- | ---: | ---: | ---: |
| Signed APK, bytes | 83,808,299 | 77,041,707 | 6,766,592 (8.07%) |
| Native ELF, bytes | 81,849,928 | 75,082,184 | 6,767,744 (8.27%) |

The isolated APK SHA-256 is
`cb41c64ed15aa79e81f3e124296406cb700eedfdcd0840a7ed1677f36b10aa52`.
All 27 defined dynamic exports, including JNI/service/PDF entry points, and
the same five OS-library dependencies remained. ELF load segments and ZIP
alignment passed 16 KiB checks; v2/v3 signatures verified with the local Android
Debug development certificate. The installed APK hash matched. Apart from
signing metadata and the native library, the packaged configuration changed
only the absolute path of the frozen frontend input.

In the owned offline API 33 ARM64 emulator, the isolated candidate passed real
Wikipedia title search, first/repeated `multicellular` full-text snippets,
Animal/Eukaryote navigation, native Back, an actual internal article link and
cold tab restoration. The production Hawaii StreetZim passed place search,
Honolulu selection, rendered map tiles/labels, zoom and pan. Physical phones
and VoiceOver were untouched.

Exploratory grouped launch/PSS samples are not a causal performance result:
host compilation overlapped the baseline group, sampling order was not
counterbalanced, and swap differed. No startup, whole-app memory or
physical-device speedup is claimed. The isolated APK remains 9.49% larger than
the pinned official ARM64 Android APK. New Mac/iOS package-size savings have
not been measured.

ThinLTO was rejected as a no-op in this build: the mixed staticlib/cdylib/rlib
root did not receive an LTO compiler flag despite the requested profile setting.
This is a known [Cargo mixed-output limitation](https://github.com/rust-lang/cargo/issues/14612),
not evidence of an LTO benefit. No ineffective LTO configuration was retained.

The final integrated source, including Clippy remediation, separately produced
a **77,029,419-byte APK**: **6,778,880 bytes (8.09%) smaller** than the preceding
optimized build, and **9.47% larger** than official Android. Its installed code
root totaled **78,856,775 logical bytes / 77,068 allocated KiB**, including the
APK and generated ODEX/VDEX—not mutable caches, public archives or shared OS
WebView. This is a separate acceptance artifact, not a replacement of the
earlier upstream runtime cohort. It passed signature/alignment/export checks,
installed-hash matching, real title/full-text queries, Eukaryote/Back/Animal
cold restoration and Honolulu **city** map selection/zoom/pan. Wikipedia had
three tabs and the final map had four; no matched runtime ranking is inferred.
Its identity, native-library hash, storage scope and screenshots are in
[the codegen/Clippy receipt](benchmarks/2026-11-04-codegen-clippy.json).

## Measured follow-on optimization: reader-efficiency batch

This batch follows the integrated codegen/Clippy artifact, not the original
upstream runtime cohort. [The reader-efficiency receipt](benchmarks/2026-11-04-reader-efficiency.json)
records identities, bounded samples, native compatibility probes and deployment
acceptance; evidence is rooted at `/private/tmp/kiwix-efficiency.z5otfQ8l`.

### Current package and installed-code comparison

| Comparable Android ARM64 form | Current reader-efficiency build | Official 3.14.1 standalone ARM64 |
| --- | ---: | ---: |
| Signed APK, bytes | 66,981,931 | 70,366,062 |
| Owned API 33 installed code root, logical bytes | 68,809,287 | 77,055,894 |

The current APK is **4.81% smaller than official Android**, and **13.04% smaller
than our preceding 77,029,419-byte integrated artifact**. Installed code means
APK plus generated ODEX/VDEX from normal push installation, not mutable data,
shared archives, OS/WebView or filesystem overhead; it is not an unrestricted
installed-storage total or a physical-phone code measurement.

The accepted Android-targeted frontend APK SHA-256 is
`c42c1fc678a0cc415b62bd4f6f2cc2719bbe893a0107898eb0716d45824f6c14`.
The same-sized generic-frontend candidate with SHA-256
`f7b32548f0c84f6b4fef260828dbaa413dce41b1d28669d468ffd193b48f2617`
is **excluded**: it omitted compile-time Android document-source capture, and
native UI correctly refused missing document authority. Its equal byte count
does not establish equivalence. The accepted build retains optimization level 3,
one codegen unit, unwind behavior, required exports and system dependencies;
symbols are stripped, debugging is disabled, and v2/v3 signatures and 16 KiB
ZIP/ELF alignment passed. Signing remains local development signing, not
production distribution.

Android's `reader-services-v1` ICU policy removes currency, time-zone, unit and
rule-based-number formatting service trees (`curr`, `zone`, `unit`, `rbnf`)
from the **libzim reader/search closure**, not from a general-purpose formatting
SDK. It prunes no locales; retained members are byte-identical. A fresh ARM64
recipe build and native probes compared multiscript normalization, locale-language
resolution, English title/full-text results, pathname/retained-descriptor access
and expected article bytes/hashes against the full SDK. ARMv7 was not rebuilt;
iOS does not use this filtered policy.
This is not evidence that Mac ICU can safely be reduced the same way.

| Current iOS development artifact form | Measured bytes | Official counterpart |
| --- | ---: | --- |
| Signed expanded ARM64 app, regular files | 51,116,419 | No official bundle measured |
| Actual locally zipped development IPA | 22,770,533 | No official IPA obtained |

These are not App Store compression/thinning measurements. Neither the older
unsigned archive nor Apple lookup download metadata is a matched official IPA;
no Apple size ratio or matched Apple runtime result is claimed.

### Bounded runtime and compatibility conclusions

The app changes defer six optional surfaces (Catalog, Downloads,
LocalSwarmControls, Maps, Settings and Collections) while keeping reader/library
eager; use narrow Store getters; allocate buffers after locking; single-flight
concurrent cache misses with invalidation correctness; and cache the native
search end iterator. The JSON formatting contract is unchanged. Exercised
surface availability does not certify network workflows: offline registry
warnings and initial Nearby permission UI remain explicit limits.

| Current bounded experiment | Result and limit |
| --- | --- |
| Restored Wikipedia Animal memory | Candidate versus preceding integrated baseline differs by about 0.1%; adjacent-block direction reverses and candidate swap is higher. No reliable memory improvement. |
| Rendered Hawaii overview memory | Candidate versus preceding baseline differs by about 0.3%; direction reverses. Official-app blocks are included, but variation exceeds differences: no robust superiority. |
| Rendered Animal readiness | Observer-inclusive sampled visual bounds after process-cold starts, without OS cache purge; no demonstrated readiness-speed win over our baseline. Not first paint or confidence intervals. |
| Warm native query ABI | Same full SDK and NDK flags isolate old/current cached-end bridge; workload-dependent changes include regressions. No consistent search-speed win. Not final filtered-SDK performance or whole-app latency. |

Memory uses separate bounded cohorts: two library references, one reader/map
tab, settling, and main plus exclusively attributed WebView renderer PSS.
It excludes unassigned GPU/utility/system memory. Readiness-cohort memory with
growing observer logs was rejected in favor of bounded sampling, not pooled.
Earlier upstream comparisons, small-archive runs and native probes retain
their own content, process, SDK and measurement boundaries; none establish a
current robust whole-app memory or speed advantage over official apps. Raw
native batch tables remain in the receipt rather than duplicating them here.

### In-place phone deployment, not physical runtime certification

Both phones were updated in place without uninstalling the target or clearing
its data. Android's captured prelaunch profile/preferences, URI grants and
runtime permission records were preserved; archive evidence was size/mtime,
not content hashing. iOS captured small profile files and native preferences
were byte-exact, with archive reported metadata preserved but archive bytes
unhashed. iOS acceptance explicitly permits the changed data-container UUID
and five JSON files gaining only OS data-protection attribution xattrs; it is
**not strict identity/metadata equality**.

Launch-command success does not prove physical reader rendering: the Samsung
capture shows the notification shade, and the iPhone later disconnected,
preventing updated reader-surface/postlaunch migration acceptance. No phone
performance ranking follows. VoiceOver remained off and untouched.

## Measured follow-on optimization: shared HTML responses

After `b7012ee`, ordinary validated HTML shares its already-read article buffer
around the injected scripts rather than allocating/copying the complete response.
The bounded head scan now compares ASCII case-insensitively without a lowercase
temporary; HEAD does not construct a discarded GET body. UTF-8 validation,
response length, CSP, archive ownership and retirement remain unchanged.

The [response-path receipt](benchmarks/2026-11-04-response-path.json) uses actual
Wikipedia **Animal**, the production Hawaii map's **1 MiB `index.html`**, and its
**Honolulu** article. These are additional **requested Rust allocation bytes
during response-body construction**, not total memory. The original article
buffer and a controlled 280-byte injection already exist outside this boundary.

| Real HTML workload | Previous requested bytes | Shared-response requested bytes |
| --- | ---: | ---: |
| Wikipedia Animal | 677,196 | 136 |
| Hawaii map document | 1,114,424 | 136 |
| Honolulu article | 80,128 | 136 |

Allocation calls fell from three to two. A syntax-aware probe compiles the
actual current helpers and the frozen preceding scanner. Four process runs,
two per variant, validated all **10,800 measured bodies** byte-for-byte through
every response frame. Boundary/mixed-case/no-head scan equivalence checks passed.

| Workload | Previous constructor, µs | Shared constructor, µs | Previous debug HTTP, ms | Shared debug HTTP, ms |
| --- | ---: | ---: | ---: | ---: |
| Wikipedia Animal | 11.690 | 0.476 | 1.009 | 0.464 |
| Hawaii map document | 17.337 | 0.167 | 1.279 | 0.769 |
| Honolulu article | 1.877 | 0.098 | 0.554 | 0.254 |

Constructor columns are equal-weight means of two process medians of batch
means, from a release-profile probe. They include head scanning and allocator
counter overhead, but exclude archive I/O, UTF-8 validation, injection preparation,
body consumption, network and UI. They are not percentile estimates.
HTTP columns are equal-weight means of four block medians per variant across
**1,200 actual GETs**: two live **debug headless applications**, warm persistent
loopback connections and the same native SDK. Validation was outside timing.
An unoptimized old scan can amplify the debug difference; these are not release
or mobile delivery/readiness measurements, nor comparison with official Kiwix.

Actual final-app browser internal navigation rendered Animal/images and the
map; map verification used an explicit owned SwiftShader browser after the
default browser honestly reported unavailable WebGL. The final Android APK
also rendered real title results, Animal/images and Hawaii map tiles offline.
GET source bytes/security assertions and HEAD status/empty-body/length passed;
HEAD security-header parity was not separately checked.

That shared-response cohort's APK remains **66,981,931 bytes**, SHA-256
`5d28b3c6c2fdc9d08879e8e0b5b2edf27ae82990f2898dac71ab4cdb5efeaa03`.
It retains the prior signature/export/dependency/16 KiB checks and size goal;
the preceding `c42c…` APK is a separate artifact. Current signed iOS regular-file
bytes are **51,116,323**, and the actual development IPA is **22,771,864 bytes**.
No official IPA or matched Apple runtime ratio is implied.

Both final phone updates were installed in place and normal launches succeeded.
Android's captured profile/native preferences/document grants/runtime permissions
were exact. iPhone prelaunch content was exact with the separately accepted
container/five-attribute changes; postlaunch acceptance allows only three managed
book paths, the download-directory relocation, corresponding root bookkeeping,
and one observed history-title change. Raw strict-failure reports remain.
Archive bytes were not rehashed, and fresh physical-reader visual or performance
certification is not claimed. VoiceOver remained off and untouched.

## Measured follow-on optimization: demand-driven annotation resources

Annotation observation and paint resources now follow the actual annotation
lifecycle rather than eagerly observing parser mutations in every article.
A completed, nonempty current-document highlight stream activates lazy
mutation/resize observers; empty/new-document streams and page teardown
disconnect them. Text indexing is skipped when there are no annotation items.
Highlight style/fallback layers are created on demand and removed when no
ranges remain. Mutation still clears stale paint immediately before bounded
quote re-resolution; current-document identity and the trusted root annotation
producer remain authoritative.

### Isolated EMULATOR bridge experiment, not rebuilt-phone proof

The [frozen emulator receipt](benchmarks/2026-11-04-annotation-emulator-memory.json)
records one accepted **ABBA** on our same earlier `5eec1e4` APK: original and
candidate bridges supplied through identical owned CDP Fetch transport, with
strict zero-iframe attachment, current-document marker and exactly-one
acknowledged fulfillment. Every block process-cold restored actual Animal,
settled 30 seconds and collected four samples: **eight per condition**.
The article used **18 px / zoom 1**, distinct from the physical 16 px cohort.

| Isolated emulator main + owned renderer PSS, KiB | Original bridge | Candidate override |
| --- | ---: | ---: |
| Pooled median | **208,389** | **199,050** |
| Observed range | 207,930–208,851 | 198,234–199,815 |
| Median swap PSS, separate | 1,695 | 1,644 |

This is **4.48% lower PSS (−9,339 KiB)** in that controlled override experiment.
Both adjacent comparisons favor the candidate (−4.63% and −4.34%). Rejected
pre-PSS attachments remain disclosed in the receipt. Allocator categories
overlap; allocated-object bytes are not proven reachable/live bytes. This
result is neither a comparison with official Kiwix nor rebuilt-APK/physical
acceptance, and **does not transfer into a phone memory-gain claim**.

### Actual highlight consumers and lifecycle coverage

The [functional receipt](benchmarks/2026-11-04-annotation-functional.json)
preserves the parent's 14 passing regression checks and separate actual
consumer coverage. On the owned Android emulator, normal **Bookmark and notes
→ Highlight selected passage** UI created a real stored Animal highlight.
Mutating its actual quote cleared stale paint and reported unavailable;
restoring it recovered found/paint. Normal **Remove bookmark** removed paint
and owned nodes without direct store/API deletion.

Actual fallback rendering was exercised by temporarily shadowing the Highlight
constructor after PSS collection, then restoring it; this is compatibility-path
coverage, **not an older-engine performance result**. Genuine navigation/back
in isolated desktop Chrome 156 produced `pageshow.persisted`, retained highlight
paint and mutation/recovery afterward. That BFCache proof uses the production
bridge with a **fixture parent/highlight**, not Android WebView BFCache or
end-to-end app-shell stored-bookmark BFCache certification. Synthetic stream
attempts superseded by the authoritative empty root producer are explicitly
excluded rather than counted as product failures or feature proof.

The three public summaries are frozen records of their own scope; their
original release-boundary text is not rewritten to pretend later acceptance.
The earlier annotation-release APK identity and **+5.47% physical result**
remain in their separate receipts above. Neither that result nor the later
final combined cohort isolates an annotation-caused phone saving.

## Final combined Android release: source and bounded runtime evidence

The already reviewed [Zstd accounting receipt](benchmarks/2026-11-04-android-zstd-cache-accounting.json)
corrects which existing stream is queried for decoder/encoder memory cost.
The [decoded-cluster retirement receipt](benchmarks/2026-11-04-android-decoded-cluster-retirement.json)
records retiring a compressed decoder only after all logical blobs have
successfully materialized, retaining independent decoded buffers for random
and range reads without additional reads/decompression to trigger retirement.
The nominal **16 MiB** cache budget remains unchanged; insertion-time costing
is still an estimate, not a hard resident-memory ceiling. Those host/source
proofs are linked unchanged, not relabeled as isolated phone savings.

The [renderer diagnostic receipt](benchmarks/2026-11-04-android-renderer-memory-diagnostic.json)
retains instrumented heap/allocator/compositor observations and the bounded
Android compact-navigation overflow correction. Categories overlap and are
not summed as PSS or collectible savings. The final build includes the
reviewed CSS without removing any of the seven navigation destinations.

The [final release/runtime receipt](benchmarks/2026-11-04-final-android-release-runtime.json)
binds these reviewed overlays to the exact final installed APK. Actual
**Animal** heading/montage/classification and **Hawaii Home**, **Hilo search
rows** and a selected detailed map with buildings/roads/300 m scale were
observed. Map functionality does not establish matched camera, memory, FPS or
tile-ready latency.

Root Back was independently admitted for the **empty-jobs case only**:
the same observed app PID remained before and after a **35.003-second**
Activity-free wait and relaunch, with actual Animal and no visible crash
report. This is not paused/active/completed-download notification acceptance
or a general process-generation certificate. The diagnostic helper package
was absent and owned diagnostic forwards were removed before final collection.

The accepted complete-CSS browser matrix has **20 observations**:
320/384/540 CSS px, LTR/RTL and 1×/2× CSS scaling retain all seven center-hit
targets; 541/760 CSS px retain automatic vertical-rail scrolling. The first
matrix injected a shortened CSS read and remains rejected. This is **browser
CSS-scale coverage**, not maximum Android OS font/display settings or
physical-phone accessibility certification. The final combined physical
memory result above remains **6.19% worse**, without individual-change
causality.

## Remaining parity and measurement work

- **Size:** the frozen final combined ARM64 APK is 4.80% smaller than official and the same byte count as the earlier annotation-release artifact, but has its own verified SHA-256. Earlier emulator code-root and Samsung Settings measurements retain their artifact/profile scopes; no final-APK installed-storage comparison is claimed. Mac remains 5.06× larger by the measured expanded app; no new Mac artifact or matched official iOS IPA establishes a size result there.
- **Comparable runtime:** unmodified Apple shipping apps need a genuinely isolated account/VM or authorized isolated device workflow; `HOME` alone does not isolate Core Data, standard defaults, sandbox Documents/session/cookies and WebKit. The installed `self.Kiwix` app was not launched against the owner's data. Re-signing/changing identity would create a modified-binary baseline, not unmodified shipping acceptance. iOS has no public official release IPA in the inspected GitHub assets; no authentication/DRM bypass was attempted. Windows still needs our native artifact and runner. The ARM64 Linux package is available, but its matched optimized-runtime/dependency experiment was not run here.
- **Memory/readiness:** the completed frozen final physical Wikipedia comparison is 6.19% worse than official, with all four paired directions unfavorable—not an overall memory improvement. Earlier +5.47%/+4.11% physical cohorts and the isolated emulator 4.48% bridge reduction remain separate; no individual-change causal saving is established. Earlier-APK startup observations favor official and warmed title autocomplete resolves no faster app; neither measures final-APK timing. Actual final-APK maps establish functionality only, not matched-camera memory/FPS. Broader archives, long sessions, other devices and cold/full-text search distributions remain unmeasured. Activity Manager startup and warm native-query timings are not substitutes.
- **Product parity:** competent localization, remaining AT/provider/media/lifecycle acceptance, physical BLE/hotspot **join/cancel/route-loss** phone pairs (same-LAN Mac↔Android and Android/Mac↔iPhone discovery, catalog and download are now verified), production distribution and automatic predecessor-profile migration remain open. Migration remains owner-deprioritized. Neither this report nor completed scoped fixes close the broader parity goal.

The iPhone bottom-edge correction was separately installed and inspected after owner unlock: the portrait Japan/Tokyo reader's dark navigation reaches the screen bottom. Prelaunch preservation passed; post-launch title and migration-source differences remain explicit, not an exact State-match claim. That development build is not the unsigned release archive measured above. **VoiceOver was untouched throughout this work.**

## Identities, receipts and reproducibility boundaries

Representative runtime evidence root: `/private/tmp/kiwix-real-archives.MJz5D8fN/`. Frozen release/static-upstream and historical replay evidence root: `/private/tmp/kiwix-upstream-release.DDIvhEIK/`.

| Artifact | SHA-256 |
| --- | --- |
| Our frozen comparison Android APK, `ours-release-arm64-replay-fixed.apk` | `09da3ccb6910ce51b5dc66c4d82be27153749a26442f6fa7092e1eb4d01600ec` |
| Official Android APK | `62a9910be260030e642465c17d642537e6ee4ee5f2d4596c248267c2766a88b8` |
| Our final Mac executable, `ours-final-macos.app/Contents/MacOS/kiwix-workbench` | `36186fa76dbb352187cdef6eeac96a31f8a05f68f00b44fdb73113215b21b09b` |
| Our final unsigned iOS executable, `ours-final-ios-unsigned.app/Kiwix` | `a590b960ad3c40f834b7cb28a2cef6172c8d3a5827f65d620cd28b6b81c3e4c1` |
| Official Apple DMG | `58744c6301d7ad6cb24c288e04d46c32fa0f032ba66deed1d75f7d2279f946bd` |
| Official Windows ZIP | `56ad5b3a86010c2c74db63a717d087cfbdcdbe71a0d7fca0655f90ef1d80593f` |
| Official Noble ARM64 DEB | `d772ce28a16cc274251e902b0d48c919b556244a11ea9d1291f5877f43d07cde` |

- Representative runtime: `representative-summary.json`, `startup-runs.json` accepted Wikipedia keys, `*-memory-wikipedia-animal-accepted.json`, `*-memory-streetzim-rendered-overview.json`, corresponding storage inventories, both installed identities, `workload.json`, raw commands and `device.json`. The committed machine-readable receipt records the accepted cohort separately from preliminary/interrupted/nonmatching runs.
- Representative native surfaces: `*-wikipedia-accepted-restored-5.png`, `ours-wikipedia-title-animal-returned.png`, `upstream-wikipedia-title-animal-accepted.png`, `ours-wikipedia-fulltext-multicellular.png`, both apps' Eukaryote destinations, `*-hawaii-place-honolulu.png`, `*-hawaii-honolulu-rendered.png`, `*-hawaii-after-zoom-pan.png`, and the accepted map-memory screenshots described above. Historical replay before/after remains `033-ours-final-hello-world.png` / `036-ours-replay-fixed-hello-world.png`.
- Browser: `replay-browser-before.json/png`, `replay-browser-after.json/png`, `replay-external-still-gated.png`. The after JSON explicitly says context-menu proof was not obtained.
- Static upstream evidence: `apple/` complete manifests, slice/signing-metadata/size/download receipts; `desktop/static-distribution-measurements.json` and pinned source archive. Signature metadata inspection of Apple is not cryptographic publisher verification. Android APK signature verification and our Mac deep/strict local signature verification were separately exercised.
- Selected native inputs: schema-2 retained-descriptor SDKs, Rust 1.99 mobile compiler, Java 21 / Android SDK 36 / NDK 28.2.13676358; macOS/iOS SDK 27.2. These are local development toolchains, not the official apps' release toolchains or a bit-reproducibility attestation.
- A Rust `rust-objcopy` host-tool launch warning exposed a missing matching `llvm-tools` component. Installing it restored actual `rust-objcopy --version` execution; final target ELF stripping was independently inspected rather than inferred from warning suppression. Initial iOS compiler selection also used the wrong sysroot; the explicit rustup-toolchain path built the final archive. The later Clippy remediation passed strict host, Android, iOS and Android-evaluation checks; vendored/native warnings remain. See [the scoped verification](RELIABILITY.md#release-code-generation-and-rust-diagnostic-remediation--2026-11-04), not a warning-free certification.
- The historical representative owned emulator process exited 0; its private AVD and executable probe/build scaffolding were removed after acceptance. Reviewed helper sources remain inert as `*.py.source.txt`; raw receipts, original archives, SDKs and frozen APK/app artifacts remain. Historical disposable emulators/replay profiles were also stopped/removed. Failed admission/disk-headroom/AX observations remain separate receipts. Normal phones and Kiwix libraries/settings were untouched by that historical emulator cohort; the later explicitly authorized physical Android reset is described above. VoiceOver was untouched.
