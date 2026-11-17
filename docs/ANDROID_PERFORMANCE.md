# Can older Android phones assemble and verify multilingual ZIM files?

**Yes: a physical 2021 Moto e20 with 2GB RAM and Android Go successfully assembled and verified the tested multilingual ZIMs.** A 201MB multilingual test file took about **30 seconds**; a real 49.5MB Bahrain map took **9.3 seconds**. The phone ran the actual native assembly code in a 32-bit ARMv7 process. All 26 operations passed, including corruption rejection and cancellation cleanup.

This answers the basic low-end feasibility question for these workloads—not for every map size or the full app under memory pressure. The largest file tested on this 2GB phone was 201MB. Earlier tests on four 3–6GB phones covered files up to 4.43GB; those results are a separate build/workload cohort below.

## New result: a genuinely low-end Android Go phone

Measured on 30 October 2026 through Firebase Test Lab. Android reported **1.90GB physical RAM**, low-RAM mode, Android 11 and a **32-bit ARMv7** process on the Moto e20. Its native library loaded successfully before accessing the test inputs.

| Workload on Moto e20 / 2GB | Output size | Assembly, cold policy | Assembly, warm policy | Separate whole-file SHA-256 | Separate ZIM MD5 | Separate structural validation |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Small multilingual test | 2.16MB | 0.51 s | 0.51 s | 0.25 s | 0.068 s | <0.001 s |
| City-sized multilingual test | 201MB | 30.01 s | 30.37 s | 22.87–22.88 s | 6.33 s | <0.001 s |
| Real Bahrain map, German recipe | 49.5MB | 9.30 s | 9.32 s | 5.62 s | 1.55 s | 0.068 s |

Assembly times are medians of three repetitions per policy and already include the production composer's integrity checks, synchronization and publication. The separate SHA-256, MD5 and structural columns are independent diagnostic passes **after** assembly, not times to add to every normal app download. “Cold/warm” describes prepared source/cache policy, not a controlled empty/warm OS page cache. Downloading and preparing inputs were outside the assembly timer.

**What passed:** 26 operations, zero failures and zero storage skips. All 22 successful output validations matched the expected SHA-256 and passed retained-checksum, structural, article and search assertions. The smaller multilingual and Bahrain cases also exercised existing-output preservation, corruption rejection, cancellation cleanup and boundary checks; those negatives were not repeated on the 201MB case.

**Memory and storage:** the largest observed sampled worker PSS was **98MiB**; sampled RSS was **168MiB**. This includes the JNI worker, Java runtime and instrumentation—not the interactive reader/WebView—and sampling can miss brief peaks. The test retained 256MB of delivered source files; the largest measured private-run logical footprint before cleanup was 604MB. That footprint is not a claimed total-device peak or a production space multiplier.

**What this changes:** low-end 2GB/32-bit feasibility is now demonstrated. Hashing remains noticeable: a separate SHA-256 scan of 201MB took about 23 seconds on this phone. The tiny structural timings for synthetic files must not be extrapolated to complex real maps.

**What remains unproven:** 1GB devices, large maps on a 2GB phone, full-app memory pressure, battery/thermal behavior and the newer raw-range bounded-cache workflow on physical Go hardware. The TECNO POP 7 inventory independently confirmed a low-RAM 2GB/32-bit device, but its workload was rejected by Test Lab's free quota before execution; it has **no performance result**. No paid retry or quota bypass was made.

Exact Moto evidence: [per-device summary](../.native/evaluation/moto-e20-physical-20261030/per-device-summary.json), [phase CSV](../.native/evaluation/moto-e20-physical-20261030/phases.csv), [integrity results](../.native/evaluation/moto-e20-physical-20261030/integrity-results.json), [raw provenance and hashes](../.native/evaluation/moto-e20-physical-20261030/evidence-sha256.json). [TECNO inventory and quota denial](../.native/evaluation/pop7-physical-20261030/per-device-summary.json).

## Earlier results: four 3–6GB phones

Four physical phones were measured on 29 October 2026. These ARM64/pre-SDK2 results remain separate from the new SDK2/ARMv7 Moto results; they do not establish a speedup or slowdown between builds.

## 1. How long does assembling a file take?

“Assembly” means the app's real StreetZim code reconstructing a ZIM from a recipe, an existing base and language-pack pieces. It includes checking source pieces with SHA-256, writing the result, generating/checking its ZIM MD5 checksum, syncing and publishing the output. It does **not** include downloading the inputs, copying test fixtures, or all the separate validation scans measured in the next section.

| Phone / tested RAM | Real 271MB map: assembly | Synthetic 1.07GB multilingual file: assembly | Real 4.43GB multilingual map: assembly | Largest sampled worker RAM in these cases |
| --- | ---: | ---: | ---: | ---: |
| **Galaxy A03s (2021), 3GB** | **6.3–6.4 s** | **31–41 s** | **Not run: insufficient free storage** | **103 MiB** |
| Galaxy A06 (2024), 4GB | 5.1–5.4 s | 20–27 s | 98–103 s | 141 MiB |
| Infinix Smart 8 / X6525 (2023), 4GB | 6.0–6.2 s | 20–21 s | 120–121 s | 132 MiB |
| Galaxy A16 LTE (2024), 6GB — comparison device | 4.2–4.3 s | 13–15 s | 69.5–70.3 s | 145 MiB |

The real maps are East Polynesia (270,675,451 bytes) and Austria/Czechia/Slovakia (4,426,552,705 bytes). They contain actual map/article content and language/search packs. The 1,073,803,454-byte synthetic file contains English, Arabic and Hindi test articles and search indexes plus highly compressible filler; it is a large-byte-count test, **not a substitute for a real large map**.

Times are the range between the cold-policy and warm-policy medians, rounded for readability—not a best/worst guarantee. The smaller two cases had three runs per policy; the 4.43GB case had only one per policy. OS caches were not controlled. “Worker RAM” is process PSS sampled every 100ms, including the benchmark's Java/native runtime but **not the full interactive app/WebView**; samples can miss brief peaks.

## 2. How expensive are the hashes and other integrity checks?

The experiment measured three different kinds of checking:

- **Piece SHA-256:** checks that source chunks match the recipe while assembling. Its time is **already inside** the assembly column above.
- **Standalone whole-output SHA-256 and ZIM MD5:** separately timed scans of the completed file to confirm its expected bytes and built-in checksum.
- **Standalone structural validation:** checks internal entry pointers, path/title ordering, cluster pointers/offsets and MIME-type references. Unlike hashing, this parses tables and follows references; it is not simply a sequential pass over all bytes. Article reads and title/full-text search assertions were checked separately.

| Phone / workload | Piece SHA-256 — already included in assembly | Standalone whole-file SHA-256 | Standalone ZIM MD5 | Standalone structural validation |
| --- | ---: | ---: | ---: | ---: |
| **A03s, real 271MB map** | **2.4 s** | **2.1 s** | **1.1 s** | **23.3–23.4 s** |
| **A03s, synthetic 1.07GB file** | **10.3–13.8 s** | **12.0 s** | **4.3 s** | **<0.01 s** |
| A06, real 4.43GB map | 40.3–40.7 s | 44.3–44.4 s | 17.4–17.7 s | 24.3–25.0 s |
| Smart 8 / X6525, real 4.43GB map | 48.5 s | 55.7–57.3 s | 27.8–28.2 s | 26.6–27.3 s |
| A16, real 4.43GB map | 22.9–23.2 s | 24.9–25.0 s | 14.5–14.6 s | 9.7 s |

**The important finding: hashing is feasible, but real-archive structural checks can be a significant part of the wait.** On the oldest phone, the real-map structural scan took longer than assembly. The almost-instant structural check of the synthetic file would have hidden that cost in a synthetic-only benchmark.

**“Standalone” means separately timed, not a mandatory additional app step for every column.** The benchmark deliberately ran these independent checks, then a further combined MD5/structural check, to validate correctness and measure their cost. Do not add every column into a supposed measured “download to ready” time: that end-to-end interactive workflow was not measured. In the app, assembly calculates MD5 while writing; acquisition import subsequently checks the saved file's checksum and structure once before making it available. The standalone whole-file SHA-256 and repeated standalone-plus-combined sequence are benchmark checks, not additional normal map-import stages.

**These are hashes/checksums, not digital signatures.** They detect mismatched or damaged bytes; they do not prove who published the file. Public-key signature verification was not benchmarked.

Hashing proves that bytes match an expected digest; it does not prove that their internal layout is valid. Structural checking therefore catches a different class of problems, including malformed recipes/archives whose hashes still match. The current libzim implementation traverses entries in separate path-order, title-index and MIME-type checks. These are candidates for reducing repeated parsing without dropping validation; their individual costs and any resulting speedup have **not** been measured on the physical Android phones.

## 3. What did we learn?

- **An older budget phone can do the work.** The A03s completed the executed multilingual cases correctly; the real 4.43GB case also worked on the tested 4GB phones. A premium phone was not required for those workloads.
- **The assembly worker streams the work rather than loading the entire ZIM into RAM.** The observed worker footprints support this design. They are not a promise that the entire app will fit every low-memory device.
- **Integrity checking is real work, not a negligible final step.** Show progress through checking as well as through assembly. Real content matters: structural-check cost was very different from the large synthetic test.
- **Free storage was a demonstrated obstacle.** The A03s's largest cases were skipped before assembly, not crashed or timed out. They provide no large-merge speed or memory result for that phone.
- **A physical 2GB Android Go phone has now passed the smaller workload.** See the new Moto e20 results above. Large files on that RAM class, 1GB devices, battery use, long-term thermal behavior and the full app under memory pressure remain unmeasured.

All executed cold/warm outputs matched the expected size, whole-file SHA-256, ZIM MD5 and structural checks, and passed their article/search assertions. All four phones also passed the smaller corruption, cancellation, existing-output collision and boundary scenarios. Those negative scenarios were not repeated on every large archive.

This is recipe-based assembly of prepared content and indexes—not a general merger for arbitrary unrelated ZIMs, on-device translation, or rebuilding all search indexes from scratch.

## 4. Why couldn't the oldest phone run the largest case?

The test retained **10.38GB of source files for the whole test set**. At the largest real case, the A03s had **8.73GB free**, while the test required **13.46GB additional headroom** for staging, cache, output and reserve. It therefore refused to start that case.

That shows a storage limit of this test setup. It does **not** prove that the A03s cannot assemble a large ZIM using a smaller set of inputs. Existing recipe selection can deliver only the chosen large recipe's inputs—about **4.49GB**, rather than every test's inputs—but staging/output space is still required. We have not rerun that case on the A03s or demonstrated that it fits a 32GB phone.

## 5. Which phones were these, and roughly what do they cost?

Prices are approximate US-dollar or euro amounts checked on 29–30 October 2026. Launch and current examples differ by market/configuration, so this is a rough buying guide, **not a price-history comparison**.

| Phone | Released | Tested RAM | Approx. launch price | Approx. price now |
| --- | --- | ---: | ---: | ---: |
| Moto e20 — new Android Go result | October 2021 | 2GB | **€100** | Current used price not confirmed |
| Galaxy A03s | August 2021; US January 2022 | 3GB | $160 at US launch | **$80–100 used** |
| Infinix Smart 8 / X6525 | November 2023 | 4GB | $105 | **$70 new** |
| Galaxy A06 | August 2024 | 4GB | $160 | **$120–135 new** |
| Galaxy A16 LTE | November 2024 | 6GB | $230 at Brazilian launch | **$155–170 new** |

Current A16 prices refer to a 4GB model, not the tested 6GB model; the Smart 8 quote is 3GB/64GB rather than the tested 4GB configuration. These are regional examples, not guaranteed worldwide prices.
The Moto e20 [launched at €100](https://www.gsmarena.com/motorola_moto_e20_announced_with_android_11_go_and_affordable_pricing_-news-50958.php). Accessible current results did not establish an available used/refurbished offer, so no current price is invented.

## 6. What needs testing next?

The next useful physical run is a larger real map on the Moto e20, followed by the same workload on the 2GB TECNO POP 7 and, if available, a 1GB device. The POP 7 hardware inventory passed, but Test Lab rejected its workload with `TEST_QUOTA_EXCEEDED`; no execution or timings exist. No further cloud submissions or paid testing are currently authorized.

Separately, the newer raw-range bounded-cache workflow needs physical low-end measurements. Neither the original four-phone preseeded-cache runs nor the Moto prepared-input runs measure its cache eviction/download path.

## Measurement details and evidence

Results use the app's production native StreetZim assembly/archive-validation code through a JNI benchmark, not a Java reimplementation. Both cohorts had debug assertions enabled. The earlier ARM64 cohort used optimized development builds; the Moto's SDK2/ARMv7 application used an **unoptimized development build**. Its timings are therefore not a release-performance estimate. The original four phones ran ARM64/pre-SDK2; the new Moto ran SDK2/ARMv7. Actual RAM was approximately 2.72/3.56/3.68/5.50 GiB for the A03s/A06/X6525/A16 respectively; the tables use approximate marketed RAM classes. The Moto reported 1,900,457,984 physical RAM bytes during its workload; its earlier inventory was a separate physical allocation, not proof of the same individual unit.

These physical results used the original **preseeded map-cache workload**: masked source chunks were staged before the composition timer, then rehashed by assembly. They do not measure cache creation/eviction inside that timer or the newer bounded-cache/raw-range evaluation policy. The two workload policies must not be combined into one performance comparison.

The measurements above were calculated from the original `report/phases.csv` files. The archives below retain exact timings, samples, storage decisions, recipes, hashes, device fingerprints and raw Test Lab output; those original bundles have not been rewritten.

| Phone | Exact measurements | Shareable raw evidence ZIP |
| --- | --- | --- |
| A03s | [Phase CSV](../.native/evaluation-combined/physical-a03su-report-be08e80e-f08e-41c2-af3b-72b69d0cb7f2/report/phases.csv) | [A03s bundle](../.native/evaluation-combined/physical-a03su-report-be08e80e-f08e-41c2-af3b-72b69d0cb7f2.zip) |
| A06 | [Phase CSV](../.native/evaluation-combined/physical-a06-report-528596c4-7482-4636-b02f-692734d9ac13/report/phases.csv) | [A06 bundle](../.native/evaluation-combined/kiwix-galaxy-a06-zim-performance.zip) |
| Smart 8 / X6525 | [Phase CSV](../.native/evaluation-combined/physical-infinix-x6525-report-97746eed-1bbf-44b2-b45c-e883c3bb220e/report/phases.csv) | [Infinix bundle](../.native/evaluation-combined/physical-infinix-x6525-report-97746eed-1bbf-44b2-b45c-e883c3bb220e.zip) |
| A16 | [Phase CSV](../.native/evaluation-combined/physical-a16-report-a3893a90-c555-49f3-9b40-ca4225ade604/report/phases.csv) | [A16 bundle](../.native/evaluation-combined/physical-a16-report-a3893a90-c555-49f3-9b40-ca4225ade604.zip) |

Build identities: app APK `b0d0882866deb957624e9c98d359408de1060d57fcc995bcac2f32c7519fe6dd`; test APK `ab2e492203ab17a826b43c34c98e1d42d4840f2325b8510a70d14e7e2c0d2f02`; manifest `5a1f5d10de01e0ecf3137084ccea1caeeff7866edfc86977f613553dbf535e16`.

Pricing sources: [A03s launch](https://www.theverge.com/2021/12/1/22810665/samsung-galaxy-a13-5g-a03s-android), [current used A03s](https://swappa.com/listings/samsung-galaxy-a03s?carrier=unlocked&storage=32gb); [Smart 8 launch](https://www.gsmarena.com/infinix_smart_8_brings_90hz_refresh_rate_and_5000_mah_battery_-news-60529.php), [current Smart 8](https://gshoppe.co.ke/product/infinix-smart-8/); [A06 launch](https://news.samsung.com/br/samsung-apresenta-galaxy-a06-no-brasil), [current 64GB](https://a1techdeals.com/product/samsung-galaxy-a06-64gb-gold)/[128GB](https://azelectronics.uk/products/samsung-galaxy-galaxy-a06-4gb-ram-128gb); [A16 launch](https://news.samsung.com/br/samsung-apresenta-galaxy-a16-e-a16-5g-no-brasil-com-ate-seis-atualizacoes-de-sistema-android-e-seguranca), [UK release](https://news.samsung.com/uk/samsung-introduces-the-galaxy-a16-series-featuring-two-new-devices), [current black](https://a1techdeals.com/product/samsung-galaxy-a16-128gb-gray-black)/[grey](https://a1techdeals.com/product/samsung-galaxy-a16-128-gb-grey/refurbished) (the grey page labels its displayed offer new). [Current conversion rates](https://open.er-api.com/v6/latest/USD): USD1 = GBP0.751538 / KES129.832372; Brazilian launch conversions used BRL5.622/USD for A06 and BRL6.0712/USD for A16.
