# Video containers in the Kiwix catalog

**Question.** How much of the catalog's video is WebM-only versus MP4, so we can judge whether native VP8/VP9 decoding on iOS or an upstream ZIM-recipe change is the better investment?

**Answer.** 608 of the 3,656 catalog ZIMs (16.6%) contain video. **546 of those 608 (89.8%) are WebM-only**; 49 (8.1%) are MP4-only, 10 (1.6%) contain both, and 3 (0.5%) use neither container (HLS/MPEG-TS or ASF). By bytes, WebM-only archives account for **1.50 TB of the 1.75 TB** of video-bearing ZIM payload (85.7%). TED (357/357) and youtube2zim (170/170) are WebM-only in every archive; Khan Academy, Crash Course, mdwiki and the remaining large video collections follow the same pattern.

The classification below comes from an exhaustive MIME-list scan of **all 3,656** catalog archives (8 KiB HTTP range requests, no full downloads), plus a 19-archive / 432 MiB download sample inspected with `zimdump` and `ffprobe` to verify entry counts, page references and codecs.

---

## Why this matters

iOS WKWebView has no built-in VP8/VP9 decoder, so a `<video><source type="video/webm">` element fails unless the page ships a JavaScript/WASM decoder. MP4/H.264 plays natively. Android and desktop are unaffected.

Two details complicate a simple "WebM bad, MP4 good" reading:

1. **The WebM-only ZIMs already ship a WASM fallback.** TED article pages load `ogv.js`/`ogv-support.js` and configure video.js with `techOrder: ["html5", "ogvjs"]`; youtube2zim archives bundle `ogv-decoder-video-vp8/vp9-wasm` assets. On iOS these can decode WebM in JavaScript, at a performance and battery cost, rather than failing outright.
2. **MIME presence is not the same as a referenced, usable video.** One sampled Open edX archive contains an MP4 entry that no content page references; 114 PhET archives only appear to contain video because a TypeScript source file is named `index.ts` and is therefore labelled `video/mp2t`. Those corrections are applied below.

---

## Methodology

### Catalog snapshot

| Item | Value |
| --- | --- |
| Source | `https://library.kiwix.org/catalog/v2/entries?count=5000` |
| Snapshot | 2026-11-01T14:00:22Z |
| Entries | 3,656 (`totalResults` = 3,656) |
| Total payload | 5,499.5 GB (catalog-reported lengths) |
| `_videos:yes` tag | 1,173 entries |

The feed is cached at `.native/video-survey/catalog-entries-2026-11-01.xml` and parsed to `.native/video-survey/catalog-entries.json`. Each entry keeps its OPDS name, title, language, tags, category, article/media counts, size and `.meta4` acquisition URL.

### Tier 1 — exhaustive MIME-list scan (all 3,656 archives)

A ZIM stores its MIME list near the start of the file: the 80-byte header gives `mimeListPos`, and the list is a sequence of NUL-terminated strings ending in an empty string. An 8 KiB HTTP `Range` request against a Kiwix mirror is therefore enough to read every MIME type in an archive, including a 91 GiB TED bundle, without downloading it.

- 3,656 of 3,656 archives scanned; 0 errors.
- Requests went to `https://mirror.download.kiwix.org` with `https://lb.download.kiwix.org` as fallback; both answered `206 Partial Content`.
- If the list were not terminated inside 8 KiB, the scan refetched 64 KiB (never needed in practice).
- The MIME list is authoritative for **presence** of a type, not for how many entries use it or whether pages reference it.

Classification used for the headline:

| Class | Rule |
| --- | --- |
| `webm-only` | `video/webm` present, `video/mp4` absent |
| `mp4-only` | `video/mp4` present, `video/webm` absent |
| `mixed` | both present |
| `other-only` | some other `video/*` type, neither of the two |
| `none` | no `video/*` type |

### Tier 2 — reproducible download sample (19 archives, 432 MiB)

Sampling was stratified by **download family** — the `/zim/<family>/` path segment, a proxy for the scraper recipe (`ted`, `videos`, `zimit`, `mooc`, `freecodecamp`, ...). Within each family, video-tagged entries were sorted by name, then two candidates were drawn:

- the **smallest** archive with size ≤ 150 MB;
- one **seeded-random** archive (`random.Random(20261101)`) from entries ≤ 100 MB.

Candidates were added round-robin across families (largest family first) under a **380 MB** budget. Three candidates were skipped when the budget ran out: `avanti_hi_metallurgy` (74 MiB), `freecodecamp_sw_rosetta-code` (11 MiB), `phzh_core-english-one_en` (97 MiB). Two non-video-tagged controls were added to test tag specificity. Three **targeted probes** cover MIME-list edge cases rather than the random draw:

| Probe | Why |
| --- | --- |
| `phet_oc_all` | `video/mp2t` in 114 PhET archives — is it real video? |
| `nhs.uk_en_medicines` | HLS/MPEG-TS (`video/MP2T`) content |
| `lost-stats.github.io_en_all` | smallest MP4-only zimit archive |

The manifest with every selected/skipped archive and reason is `.native/video-survey/sample.json`. The analyzed set totals 432.4 MiB (16 sampled + 3 probes); the largest archive downloaded was 82.5 MiB. Two additional ~4.2 MiB PhET archives were downloaded solely to confirm the `index.ts` mislabel. Multi-gigabyte video ZIMs (TED `all` is 91 GiB; Khan Academy English is 180 GiB) were **not** downloaded; their MIME lists came from Tier 1.

### Tier 3 — archive inspection

For each downloaded archive (inspection used `zimdump` from zim-tools 3.6.0 / libzim 9.8.2 and the local `ffprobe`):

- `zimdump list --details` enumerated every entry and its MIME type; counts are exact entry counts, not estimates.
- Up to 10 HTML pages and 20 JSON/HLS entries per archive were read with `zimdump show` and scanned for `<video>`, `<source type="video/webm|mp4|ogg">`, `.webm`/`.mp4` paths, `.m3u8` playlists, `ogv.js`, and JSON `videoPath`/`mimeType` fields. Pages were ranked toward media (same directory tree as a video entry, shared filename token, main page) rather than taken alphabetically.
- `ffprobe` identified the actual codecs of representative media entries.
- For two disputed archives the whole ZIM was dumped and searched (`grep`) to check whether a container is referenced anywhere, not just in the sampled pages.

Raw evidence:

| Path | Content |
| --- | --- |
| `.native/video-survey/mime-scan.jsonl` | One JSON record per archive: header, MIME list, family, size, tags |
| `.native/video-survey/sample.json` | Selection rule, seed, selected/skipped archives, probes |
| `.native/video-survey/archive-analysis.jsonl` | Exact MIME counts and page-scan hits per downloaded archive |
| `.native/video-survey/survey-summary.json` | Aggregated counts, percentages, per-family table, tag accuracy |
| `.native/video-survey/table.md` | Generated per-archive table reproduced below |
| `.native/video-survey/archives/` | The 19 analyzed ZIMs (432.4 MiB) plus the two extra PhET archives (8.3 MiB) |

### Reproduce

```bash
python3 scripts/video-format-survey.py catalog        # cache the catalog snapshot
python3 scripts/video-format-survey.py mime-scan --all-archives
python3 scripts/video-format-survey.py download --budget-mb 380
python3 scripts/video-format-survey.py report
```

The MIME scan is cached per archive and resumes; the download stage reuses complete archives and re-analyses locally.

---

## Results

### Catalog-wide container classes (all 3,656 archives)

Counts and sizes are catalog archives, not video files. Percentages are of the 608 video-bearing archives.

| Class | Archives | Share of video-bearing | Payload | Share of video-bearing bytes |
| --- | ---: | ---: | ---: | ---: |
| WebM-only | 546 | 89.8% | 1,502.6 GB | 85.7% |
| MP4-only | 49 | 8.1% | 159.6 GB | 9.1% |
| Mixed (WebM + MP4) | 10 | 1.6% | 91.7 GB | 5.2% |
| Other-only (`video/mp2t`, `video/x-ms-asf`) | 3 | 0.5% | 0.4 GB | <0.1% |
| **Video-bearing total** | **608** | **100%** | **1,754.3 GB** | **100%** |

- WebM present in 556 archives (91.4% of video-bearing); MP4 present in 59 (9.7%).
- 3,048 archives (83.4%) contain no video at all.
- The 114 PhET archives with `video/mp2t` are excluded from these totals; see [MIME false positives](#mime-false-positives-and-tag-accuracy).

### By download family

Only families with at least one video-bearing archive are shown. `none` counts are included to show tag noise.

| Family | Archives | WebM-only | MP4-only | Mixed | Other | No video |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `ted` | 357 | 357 | 0 | 0 | 0 | 0 |
| `videos` (youtube2zim) | 170 | 170 | 0 | 0 | 0 | 0 |
| `other` (Khan Academy, Crash Course, mdwiki, Diksha, maitre_lucas, ...) | 211 | 15 | 26 | 2 | 1 | 167 |
| `zimit` | 101 | 0 | 22 | 3 | 2 | 74 |
| `mooc` (Open edX) | 5 | 0 | 0 | 5 | 0 | 0 |
| `wikipedia` | 1,227 | 2 | 0 | 0 | 0 | 1,225 |
| `wikiquote` | 135 | 1 | 0 | 0 | 0 | 134 |
| `wiktionary` | 148 | 1 | 0 | 0 | 0 | 147 |
| `libretexts` | 13 | 0 | 1 | 0 | 0 | 12 |
| `phet` | 114 | 0 | 0 | 0 | 114\* | 0 |

\* PhET's 114 `video/mp2t` hits are the `index.ts` false positive described below and are not real video.

**WebM-only is the dominant recipe.** TED (357), youtube2zim (170), Khan Academy (`khanacademy_en_all` 180 GB, `_es` 161 GB, `_fr` 100 GB), Crash Course (23 GB), mdwiki (10.7 GB), mediawiki (6.8 GB), Diksha and the other large `other`-family channels all contain WebM and no MP4. The 19 WebM-only archives outside `ted`/`videos` (15 in `other`, 2 Wikipedia, 1 Wikiquote, 1 Wiktionary) include the three Khan Academy bundles, Crash Course, mdwiki, mediawiki, Diksha, marefa, dicoado, wikipedia_en_100 and wikipedia_uk_medicine.

**MP4-capable is a minority and is recipe-specific.** Open edX (`mooc`) is the only family that is uniformly mixed (5/5); zimit contributes 22 MP4-only + 3 mixed archives; the `other` family contributes 26 MP4-only + 2 mixed (mostly youtube2zim channels that captured MP4 instead of WebM); libretexts contributes one 2 GB MP4-only archive.

**Other containers are rare.** `eleda.education_fr_fo-offline` and `nhs.uk_en_medicines` use MPEG-TS/HLS; `lua.org_en_all` uses ASF. Neither native VP8/VP9 nor MP4 fixes those three.

### Download sample: exact counts and page references

The table is generated by the script (`.native/video-survey/table.md`). “References found” is a bounded scan of up to 10 HTML pages and 20 JSON/HLS entries per archive.

| archive | family | size | video/webm | video/mp4 | video/ogg | references found |
|---|---|---:|---:|---:|---:|---|
| ted_mul_street-art | ted | 40.0 MiB | 5 | 0 | 0 | `<source type=video/webm>`×5, ogv.js×35 |
| devdocs_en_sanctuary-type-classes | devdocs | 0.3 MiB | 0 | 0 | 0 | none in sampled pages |
| unine.ch_fr_premiers-pas-avec-python | videos | 22.7 MiB | 6 | 0 | 0 | JSON videoPath .webm×6 |
| maitre_lucas_enigmes_fr | other | 5.9 MiB | 0 | 0 | 0 | ogv.js×8 |
| gutenberg_ale_all | gutenberg | 2.1 MiB | 0 | 0 | 0 | none in sampled pages |
| gobyexample.com_en_all | zimit | 0.3 MiB | 0 | 0 | 0 | none in sampled pages |
| freecodecamp_en_coding-interview-prep | freecodecamp | 6.7 MiB | 0 | 0 | 0 | none in sampled pages |
| phzh_core-italian-one_it | mooc | 82.5 MiB | 10 | 1 | 0 | `<source type=video/webm>`×1, ogv.js×40 |
| wikiquote_tr_all | wikiquote | 70.5 MiB | 2 | 0 | 0 | `<source type=video/webm>`×1 |
| ted_mul_microbes | ted | 76.4 MiB | 9 | 0 | 0 | `<source type=video/webm>`×9, ogv.js×63 |
| devdocs_en_bower | devdocs | 0.3 MiB | 0 | 0 | 0 | none in sampled pages |
| maitre_lucas_apprendre_a_dessiner_fr | other | 6.3 MiB | 0 | 0 | 0 | ogv.js×8 |
| gutenberg_ang_all | gutenberg | 6.1 MiB | 0 | 0 | 0 | none in sampled pages |
| fretmap.app_en_all | zimit | 52.8 MiB | 0 | 0 | 0 | none in sampled pages |
| scratch-wiki_fr_all | other | 2.0 MiB | 0 | 0 | 0 | none in sampled pages |
| installgentoo_en_all | other | 2.1 MiB | 0 | 0 | 0 | none in sampled pages |
| phet_oc_all | phet | 4.0 MiB | 0 | 0 | 0 | none in sampled pages |
| nhs.uk_en_medicines | zimit | 15.3 MiB | 0 | 0 | 0 | `.m3u8`×30, `.mp4` paths×4 |
| lost-stats.github.io_en_all | zimit | 36.0 MiB | 0 | 2 | 0 | YouTube player JSON: mp4×10, webm×2 |

Across the sample: 32 `video/webm` entries and 3 `video/mp4` entries. All 14 TED WebM entries across the two TED archives are referenced by `<source type="video/webm">`; the youtube2zim archive references all 6 of its WebM files from JSON `videoPath` values; the wikiquote archive references its WebM files from `<source type="video/webm">` markup. The Open edX archive references WebM from `<source type="video/webm">`, but its single MP4 is not referenced by any content page (see below).

### Codec probes (`ffprobe`)

| Archive / entry | Container | Video codec | Audio codec |
| --- | --- | --- | --- |
| `ted_mul_street-art` → `videos/139080/video.webm` | WebM | VP9, 480×270 | Vorbis |
| `unine.ch_fr_premiers-pas-avec-python` → `videos/p6RRZHjLpRo/video.webm` | WebM | VP9, 480×270 | Vorbis |
| `wikiquote_tr_all` → `Stalin_Speech_...webm` | WebM | VP9, 320×240 | Opus |
| `phzh_core-italian-one_it` → `.../xblock/video.webm` | WebM | VP8, 480×270 | Vorbis |
| `phzh_core-italian-one_it` → `.../step-2/xblock/video.mp4` | MP4 | H.264, 640×360 | AAC |
| `lost-stats.github.io_en_all` → `videoplayback?...` | MP4 | H.264, 640×360 | AAC |
| `nhs.uk_en_medicines` → HLS segment | MPEG-TS | H.264, 640×360 | AAC |

The sampled WebM-only recipes use VP9 (TED, youtube2zim, Wikiquote); the sampled Open edX WebM is VP8. Native VP8 **and** VP9 coverage is therefore required to retire the WASM fallback for the sampled families.

### MIME false positives and tag accuracy

- **PhET (114 archives).** Every PhET archive's MIME list contains `video/mp2t`. The probe (confirmed in three PhET archives: `phet_oc_all`, `phet_bi_all`, `phet_ay_all`) shows the only such entry is `index.ts`, a 7 KiB TypeScript source file beginning `import swal from 'sweetalert2'`. The `.ts` extension was mapped to MPEG-TS. These are **not** video archives and are excluded from the 608.
- **Open edX MP4.** `phzh_core-italian-one_it` contains one `video/mp4` (H.264/AAC) and ten `video/webm` entries. A full dump of the archive shows content pages reference `xblock/video.webm`; the only `video.mp4` strings are in the bundled video.js library code. The MP4 is an unreferenced fallback in this archive, so “MP4 present” overstates actual MP4 playback for Open edX.
- **`_videos:yes` is a superset, not a filter.** 608 of the 1,173 tagged archives (51.8%) actually contain a video MIME type. The remaining 565 are false positives: devdocs (243), gutenberg (103), freecodecamp (60), ifixit (12), plus 147 others. After correcting the PhET mislabel, no untagged archive contains a real video type, so the tag has 100% recall but only 51.8% precision.
- **Tagged but empty.** `maitre_lucas_enigmes_fr` and `maitre_lucas_apprendre_a_dessiner_fr` carry `_videos:yes` and ship ogv.js player assets, but contain no `videos/` entries and no video MIME types. The tag describes the source site, not the archived payload.

---

## What this implies for the decision

1. **Native VP8/VP9 decoding would cover almost all catalog video.** 556 of 608 video-bearing archives (91.4%) contain WebM; 546 (89.8%) are WebM-only. That includes the entire TED (357) and youtube2zim (170) families and the largest single archives (Khan Academy, Crash Course, mdwiki).
2. **An upstream MP4 recipe change is a large rebuild.** Moving the WebM-only set to MP4 means re-encoding and republishing 546 archives totalling 1.50 TB of ZIM payload (TED alone is 680 GB), across at least the TED, youtube2zim, Khan Academy, Crash Course and mdwiki pipelines. It would also need a size/quality policy for H.264 versus the current VP9 encodes.
3. **MP4 already exists where the recipe produced it.** 59 archives (9.7%) contain MP4: Open edX (5 mixed), zimit (22 MP4-only + 3 mixed), 28 `other`-family archives (26 MP4-only + 2 mixed) and one libretexts archive. But the Open edX sample shows an MP4 that is present but unreferenced; “MP4-capable” should be verified per recipe before counting on it.
4. **Three archives are neither WebM nor MP4.** `nhs.uk_en_medicines` and `eleda.education_fr_fo-offline` use MPEG-TS/HLS; `lua.org_en_all` uses ASF. Native VP8/VP9 or an MP4 recipe change does not address them.
5. **The WASM fallback already exists.** TED and youtube2zim ship ogv.js VP8/VP9 WASM decoders and select them after the native `html5` tech. iOS can therefore play these ZIMs today through JavaScript; the trade-off is CPU, battery and startup latency rather than a hard capability gap. A native decoder removes that fallback cost for 91.4% of video-bearing archives.

Confidence:

- **High** for the container classification: every one of the 3,656 archives was scanned from its own MIME list, with 0 errors.
- **High** for TED/youtube2zim being WebM-only: all 527 archives are WebM-only by MIME list, and the download sample confirms WebM entries and page references.
- **Medium** for “actually referenced”: page scans are bounded (10 HTML + 20 JSON/HLS entries), so an archive marked “no reference found” is not proof of unused media. Conversely, the Open edX MP4 was checked with a full dump and is genuinely unreferenced.
- **Medium** for MP4 usability: MIME presence is verified, but the one MP4 in the Open edX sample is not referenced, and zimit MP4 playback depends on the archived YouTube player.

## Limitations

- **Sampling bias in the deep sample.** Downloads were capped at 150 MB per archive and 380 MB total; the 19 inspected archives are biased toward the smallest/cheapest archives in each family, not a size-representative sample. Three candidates were skipped for budget, and archives up to 180 GB were not downloaded. The catalog-wide MIME classification is exhaustive, so this bias affects counts/references/codecs, not the presence shares.
- **MIME-list semantics.** Presence means at least one entry of that type, not that a page references it, nor that the type is correct. The PhET `.ts` mislabel and the unreferenced Open edX MP4 are concrete examples. Media stored as `application/octet-stream` would be missed; the sample's octet-stream entries were fonts and player assets, not video.
- **Page-scan coverage.** The HTML/JSON scan is bounded and heuristic (media-adjacent directories, shared filename tokens, main page). “No reference found” is reported as such; it is not evidence of absence. For example, `wikiquote_tr_all` only surfaced its WebM references after pages were ranked by the video filename token.
- **Catalog metadata.** `_videos:yes` has 51.8% precision and describes the source site rather than the archived payload. Sizes are catalog-reported (rounded) and the snapshot is 2026-11-01; the catalog changes continuously. Duplicate OPDS names exist for maxi/nopic variants, so the scan keys archives by ZIM filename, not by `<name>`.
- **Container only.** The survey classifies containers and probes a representative codec per recipe; it does not measure bitrate, resolution distribution, DRM, or whether every entry in a mixed archive is playable. `video/webm` and `video/mp4` entries were treated as video; an audio-only WebM/MP4 would be miscounted (the sampled entries were VP8/VP9 and H.264 video). It also does not test playback in WKWebView.
