# Error copy inventory

Generated from current sources by `node scripts/generate-error-docs.mjs`.
This is a heuristic candidate-literal inventory, not proof that a string reaches
a user, is translated, or has a particular runtime severity. Extraction can
include code, tests and interpolation fragments; review candidates in context.
No replacement history is inferred by pairing unrelated added/deleted strings.

## Runtime error policy

- **Blocking:** the requested action could not complete. Display its explanation
  even when **Show technical error details** is off.
- **Recoverable:** reading or the main action can continue. Record the failure;
  display it only when technical details are enabled.
- Lifecycle suspension is a typed gate, not an error banner.
- The severity argument at the actual call site is authoritative. Behavioral
  regressions exercise visibility, suspension and retry; this inventory does not.
- Technical diagnostics may retain developer detail. Normal copy should explain
  what failed and the available recovery, rather than internal mechanisms.

Vocabulary review terms: database, record, ipc, snapshot, generation, authority, retired, installation id, uuid, handoff, quarantine, managed storage, token, shared archive, schema.
An extraction hit is not automatically a user-visible defect or certification
failure. Command dispatch also applies `user_error::plain`.

## Rust candidate literals

| Source | Extracted literal | Vocabulary review candidates |
| --- | --- | --- |
| src-tauri/src/acquisition/finalize.rs:28 | Downloaded archive failed its advertised SHA-256 checksum; cancel and start a new download |  |
| src-tauri/src/acquisition/finalize.rs:33 | Final download length {downloaded} differs from expected {expected} |  |
| src-tauri/src/acquisition/finalize.rs:160 | Download part changed before publication |  |
| src-tauri/src/acquisition/finalize.rs:162 | Unknown download ID |  |
| src-tauri/src/acquisition/finalize.rs:210 | Publishing verified archive without overwrite: {error} |  |
| src-tauri/src/acquisition/finalize.rs:217 | complete |  |
| src-tauri/src/acquisition/finalize.rs:226 | Destination has no parent |  |
| src-tauri/src/acquisition/finalize.rs:237 | Archive added, but Kiwix could not finish saving its download history. Restart the app to finish tidying up |  |
| src-tauri/src/acquisition/finalize.rs:269 | Saved download layout is incomplete |  |
| src-tauri/src/acquisition/finalize.rs:275 | The downloaded file was replaced before it could be finished; resume to download it again |  |
| src-tauri/src/acquisition/finalize.rs:283 | The downloaded file is not the expected size any more; resume to download it again |  |
| src-tauri/src/acquisition/finalize.rs:288 | Downloaded file is too short to be a ZIM archive |  |
| src-tauri/src/acquisition/finalize.rs:314 | Downloaded resource is not a ZIM archive (invalid magic) |  |
| src-tauri/src/acquisition/finalize.rs:322 | Download receipt does not match the durable byte count |  |
| src-tauri/src/acquisition/finalize.rs:342 | Download changed length during checksum verification |  |
| src-tauri/src/acquisition/finalize.rs:346 | Download changed identity during checksum verification |  |
| src-tauri/src/acquisition/finalize.rs:384 | downloads |  |
| src-tauri/src/acquisition/finalize.rs:427 | verifying |  |
| src-tauri/src/acquisition/finalize.rs:441 | Download finalizer did not start |  |
| src-tauri/src/acquisition/http_meta.rs:27 | Download has no destination directory |  |
| src-tauri/src/acquisition/http_meta.rs:28 | Checking free space: {e} |  |
| src-tauri/src/acquisition/http_meta.rs:31 | Advertised download size overflows available-space calculation |  |
| src-tauri/src/acquisition/http_meta.rs:34 | Not enough disk space: need {needed} bytes, have {free} |  |
| src-tauri/src/acquisition/http_meta.rs:56 | Invalid HTTP {name} |  |
| src-tauri/src/acquisition/http_meta.rs:130 | Metalink server returned HTTP {} |  |
| src-tauri/src/acquisition/http_meta.rs:138 | Metalink response exceeds size limit |  |
| src-tauri/src/acquisition/http_meta.rs:151 | Metalink resolution stopped |  |
| src-tauri/src/acquisition/http_meta.rs:176 | Expected an IETF Metalink document |  |
| src-tauri/src/acquisition/http_meta.rs:179 | file |  |
| src-tauri/src/acquisition/http_meta.rs:181 | Metalink has no file |  |
| src-tauri/src/acquisition/http_meta.rs:183 | Multi-file Metalinks are not supported |  |
| src-tauri/src/acquisition/http_meta.rs:185 | name |  |
| src-tauri/src/acquisition/http_meta.rs:185 | Metalink file has no name |  |
| src-tauri/src/acquisition/http_meta.rs:195 | Metalink has no valid file size |  |
| src-tauri/src/acquisition/http_meta.rs:208 | Invalid Metalink SHA-256 |  |
| src-tauri/src/acquisition/http_meta.rs:240 | Metalink has no eligible HTTPS mirror (loopback HTTP is fixture-only) |  |
| src-tauri/src/acquisition/manager.rs:111 | Some saved downloads could not be restored and are shown as failed. |  |
| src-tauri/src/acquisition/manager.rs:261 | Only finished complete, failed or cancelled downloads can be removed from history |  |
| src-tauri/src/acquisition/manager.rs:289 | Only finished complete, failed or cancelled downloads can be forgotten |  |
| src-tauri/src/acquisition/manager.rs:306 | failed |  |
| src-tauri/src/acquisition/manager.rs:306 | cancelled |  |
| src-tauri/src/acquisition/manager.rs:307 | Only terminal downloads can authorize cleanup |  |
| src-tauri/src/acquisition/manager.rs:322 | Download changed during cleanup authorization |  |
| src-tauri/src/acquisition/manager.rs:331 | Download was removed during cleanup authorization |  |
| src-tauri/src/acquisition/manager.rs:333 | Download changed during cleanup authorization; retry with its current state |  |
| src-tauri/src/acquisition/manager.rs:347 | Cannot prove original download ownership of {}; no installed archive was authorized for deletion |  |
| src-tauri/src/acquisition/manager.rs:348 | Owned download disappeared during cleanup authorization |  |
| src-tauri/src/acquisition/manager.rs:351 | Published download has no original inode proof |  |
| src-tauri/src/acquisition/manager.rs:352 | Published download has no durable checksum proof |  |
| src-tauri/src/acquisition/manager.rs:353 | Published download could not be verified |  |
| src-tauri/src/acquisition/manager.rs:355 | Published archive no longer belongs to this download; library deletion was not authorized |  |
| src-tauri/src/acquisition/manager.rs:371 | Downloaded archive path changed before import |  |
| src-tauri/src/acquisition/manager.rs:379 | Downloaded archive is no longer available |  |
| src-tauri/src/acquisition/manager.rs:395 | Download ownership changed during verification; retry the import |  |
| src-tauri/src/acquisition/manager.rs:408 | Only finished downloads can have their files deleted |  |
| src-tauri/src/acquisition/manager.rs:433 | Download changed while its files were being deleted |  |
| src-tauri/src/acquisition/manager.rs:445 | Cannot prove download ownership of {}; files and history were preserved |  |
| src-tauri/src/acquisition/manager.rs:453 | Published download disappeared before cleanup |  |
| src-tauri/src/acquisition/manager.rs:456 | ZIM\x04 |  |
| src-tauri/src/acquisition/manager.rs:457 | Published download changed; file and history were preserved |  |
| src-tauri/src/acquisition/manager.rs:481 | Cannot prove ownership of {}; files and download history were preserved. Remove installed archives through the library |  |
| src-tauri/src/acquisition/manager.rs:539 | The shared archive changed; cancel the previous download before downloading it again | shared archive |
| src-tauri/src/acquisition/manager.rs:542 | Shared library grant lock is poisoned |  |
| src-tauri/src/acquisition/manager.rs:595 | kiwix-library: |  |
| src-tauri/src/acquisition/manager.rs:607 | Expected size must be positive when supplied |  |
| src-tauri/src/acquisition/manager.rs:610 | .meta4 |  |
| src-tauri/src/acquisition/manager.rs:610 | .metalink |  |
| src-tauri/src/acquisition/manager.rs:624 | Destination is not valid UTF-8 |  |
| src-tauri/src/acquisition/manager.rs:627 | Encoding download request: {e} |  |
| src-tauri/src/acquisition/manager.rs:654 | Download ownership ledger is full; explicitly delete retained downloads and files before adding jobs |  |
| src-tauri/src/acquisition/manager.rs:662 | A download already owns this URL or destination; resume or cancel it instead |  |
| src-tauri/src/acquisition/manager.rs:667 | Destination already exists; existing files are never overwritten |  |
| src-tauri/src/acquisition/manager.rs:748 | Completed downloads cannot be paused or cancelled; manage the archive in the library |  |
| src-tauri/src/acquisition/manager.rs:849 | Download failed while stopping |  |
| src-tauri/src/acquisition/manager.rs:896 | Download was removed from history or file cleanup has started; retry deletion instead |  |
| src-tauri/src/acquisition/manager.rs:898 | paused |  |
| src-tauri/src/acquisition/manager.rs:905 | Download is still stopping; await pause/cancel before resuming |  |
| src-tauri/src/acquisition/manager.rs:909 | Only paused or failed downloads can resume |  |
| src-tauri/src/acquisition/manager.rs:927 | Destination already exists; refusing to overwrite it |  |
| src-tauri/src/acquisition/manager.rs:1140 | Native download reconciliation requires normal writable mode |  |
| src-tauri/src/acquisition/persistence.rs:88 | Persisted download key/ID mismatch |  |
| src-tauri/src/acquisition/persistence.rs:91 | Invalid shared library history URL |  |
| src-tauri/src/acquisition/persistence.rs:107 | Invalid native download adoption checkpoint |  |
| src-tauri/src/acquisition/persistence.rs:120 | Invalid persisted download status |  |
| src-tauri/src/acquisition/persistence.rs:269 | Download state exceeds size limit |  |
| src-tauri/src/acquisition/persistence.rs:271 | Opening download state: {e} |  |
| src-tauri/src/acquisition/persistence.rs:284 | Invalid persistent download state: expected an object |  |
| src-tauri/src/acquisition/persistence.rs:293 | A saved download could not be read and was skipped. |  |
| src-tauri/src/acquisition/persistence.rs:297 | Too many persisted download jobs |  |
| src-tauri/src/acquisition/persistence.rs:336 | Kiwix could not move a damaged saved download file aside ({path}). Restart the app; if this keeps happening, free some storage |  |
| src-tauri/src/acquisition/persistence.rs:343 | Kiwix could not move a damaged saved download file aside. Restart the app; if this keeps happening, free some storage |  |
| src-tauri/src/acquisition/segments.rs:220 | Requesting download segment: {error} |  |
| src-tauri/src/acquisition/segments.rs:237 | Encoded HTTP download cannot be resumed safely |  |
| src-tauri/src/acquisition/segments.rs:248 | Server ignored a range request during a segmented transfer |  |
| src-tauri/src/acquisition/segments.rs:254 | 416 response ended a segmented transfer |  |
| src-tauri/src/acquisition/segments.rs:261 | Download server returned HTTP {status} for a segment |  |
| src-tauri/src/acquisition/segments.rs:275 | Download server returned HTTP {other} |  |
| src-tauri/src/acquisition/segments.rs:296 | Invalid 206 Content-Range |  |
| src-tauri/src/acquisition/segments.rs:304 | 206 range or object total changed during a segmented transfer |  |
| src-tauri/src/acquisition/segments.rs:311 | 206 Content-Length does not match Content-Range |  |
| src-tauri/src/acquisition/segments.rs:317 | Object validator changed during a segmented transfer |  |
| src-tauri/src/acquisition/segments.rs:370 | Receiving download segment: {error} |  |
| src-tauri/src/acquisition/segments.rs:381 | Download segment exceeds its declared length |  |
| src-tauri/src/acquisition/segments.rs:408 | Download segment ended before its declared length |  |
| src-tauri/src/acquisition/segments.rs:435 | Segmented download layout is missing |  |
| src-tauri/src/acquisition/segments.rs:437 | Segmented download layout changed while it was running |  |
| src-tauri/src/acquisition/segments.rs:529 | Download segment task failed: {error} |  |
| src-tauri/src/acquisition/segments.rs:587 | Segmented download ended with an incomplete range |  |
| src-tauri/src/acquisition/transfer.rs:16 | MOBILE_NETWORK_BLOCKED |  |
| src-tauri/src/acquisition/transfer.rs:58 | Owned HTTP part was replaced; refusing to resume writes |  |
| src-tauri/src/acquisition/transfer.rs:164 | Download has no transfer URL |  |
| src-tauri/src/acquisition/transfer.rs:222 | identity |  |
| src-tauri/src/acquisition/transfer.rs:226 | Partial transfer lacks a safe resume validator |  |
| src-tauri/src/acquisition/transfer.rs:227 | bytes={downloaded}- |  |
| src-tauri/src/acquisition/transfer.rs:227 | Invalid download range |  |
| src-tauri/src/acquisition/transfer.rs:228 | Invalid download validator |  |
| src-tauri/src/acquisition/transfer.rs:253 | 416 response has no valid bytes */length Content-Range |  |
| src-tauri/src/acquisition/transfer.rs:259 | 416 response does not prove that the partial file is complete |  |
| src-tauri/src/acquisition/transfer.rs:268 | Download server returned HTTP {status} |  |
| src-tauri/src/acquisition/transfer.rs:273 | Server sent an unsolicited partial response |  |
| src-tauri/src/acquisition/transfer.rs:283 | 206 range start or object validator does not match the partial file |  |
| src-tauri/src/acquisition/transfer.rs:291 | Resumed object total changed |  |
| src-tauri/src/acquisition/transfer.rs:308 | Download size differs from expected size: {actual} versus {expected} |  |
| src-tauri/src/acquisition/transfer.rs:493 | Download response length overflow |  |
| src-tauri/src/acquisition/transfer.rs:496 | Download length overflow |  |
| src-tauri/src/acquisition/transfer.rs:500 | Download body exceeds its declared length |  |
| src-tauri/src/acquisition/transfer.rs:563 | Download response ended before its declared length |  |
| src-tauri/src/acquisition/transfer.rs:582 | Download ended before the expected object size |  |
| src-tauri/src/acquisition.rs:183 | This download is no longer available for import |  |
| src-tauri/src/acquisition.rs:185 | Kiwix cannot verify this download's original file. Add the archive to your library manually |  |
| src-tauri/src/acquisition.rs:202 | A download must have exactly one published archive |  |
| src-tauri/src/acquisition.rs:209 | The downloaded archive was replaced or changed. Kiwix left the file unchanged; add it to your library manually if needed |  |
| src-tauri/src/acquisition.rs:219 | The downloaded archive changed. Kiwix left the file unchanged; add it to your library manually if needed |  |
| src-tauri/src/acquisition.rs:636 | Acquisition monitor claim requires an absolute path |  |
| src-tauri/src/acquisition.rs:639 | Acquisition monitor claim requires a Unicode path without NUL |  |
| src-tauri/src/acquisition.rs:651 | Acquisition monitor claim has a non-Unicode filename |  |
| src-tauri/src/acquisition.rs:663 | Acquisition monitor claim has an invalid filename |  |
| src-tauri/src/acquisition.rs:708 | Missing acquisition history filename |  |
| src-tauri/src/acquisition.rs:713 | Invalid acquisition history component |  |
| src-tauri/src/acquisition.rs:772 | Filename must be a single safe .zim filename, at most 240 UTF-8 bytes |  |
| src-tauri/src/acquisition.rs:774 |  |  |
| src-tauri/src/acquisition.rs:775 | CON |  |
| src-tauri/src/acquisition.rs:775 | PRN |  |
| src-tauri/src/acquisition.rs:775 | AUX |  |
| src-tauri/src/acquisition.rs:775 | NUL |  |
| src-tauri/src/acquisition.rs:780 | Filename is reserved by the operating system |  |
| src-tauri/src/acquisition.rs:796 | Malformed percent-encoded filename |  |
| src-tauri/src/acquisition.rs:819 | URL has no filename; specify one explicitly |  |
| src-tauri/src/acquisition.rs:838 | Noncanonical persisted download ID |  |
| src-tauri/src/acquisition.rs:845 | Download destination is not absolute |  |
| src-tauri/src/acquisition.rs:847 | Persisted destination does not match filename |  |
| src-tauri/src/acquisition.rs:855 | Persisted destination is not normalized |  |
| src-tauri/src/acquisition.rs:857 | .kiwix-{}.part |  |
| src-tauri/src/acquisition.rs:864 | Refusing non-regular file: {} |  |
| src-tauri/src/acquisition.rs:866 | Inspecting {}: {e} |  |
| src-tauri/src/acquisition.rs:883 | Cancelled download file was left in place because Kiwix cannot prove it created it |  |
| src-tauri/src/acquisition.rs:1005 | Temporary download file is missing |  |
| src-tauri/src/acquisition.rs:1008 | Opening download part: {e} |  |
| src-tauri/src/acquisition.rs:1012 | Temporary download file disappeared |  |
| src-tauri/src/acquisition.rs:1014 | Temporary download file changed identity or has hard links |  |
| src-tauri/src/acquisition.rs:1023 | Download part is already in use: {e} |  |
| src-tauri/src/acquisition.rs:1043 | Download part changed identity during a segmented transfer |  |
| src-tauri/src/acquisition.rs:1089 | Published archive disappeared during recovery |  |
| src-tauri/src/acquisition.rs:1168 | The downloaded archive was replaced. Kiwix left the replacement file unchanged; add it to your library manually if needed |  |
| src-tauri/src/acquisition.rs:1200 | Invalid completed HTTP publication size |  |
| src-tauri/src/acquisition.rs:1604 | Download has no native transport lease |  |
| src-tauri/src/acquisition.rs:1755 | Owned native part was replaced; refusing to start transport |  |
| src-tauri/src/acquisition.rs:1759 | Native staging directory has no accessible filesystem ancestor |  |
| src-tauri/src/acquisition.rs:1766 | Native staging and adoption size overflow |  |
| src-tauri/src/acquisition.rs:1820 | Opening completed native download: {error} |  |
| src-tauri/src/acquisition.rs:1822 | Native download has no stable inode receipt |  |
| src-tauri/src/acquisition.rs:1826 | Native download no longer matches its completed transport receipt |  |
| src-tauri/src/acquisition.rs:1829 | Native download length differs from its authoritative archive size |  |
| src-tauri/src/acquisition.rs:1832 | Retaining native download receipt: {error} |  |
| src-tauri/src/acquisition.rs:1834 | Native download stage changed before adoption |  |
| src-tauri/src/acquisition.rs:1840 | Native download cannot replace an unowned canonical part |  |
| src-tauri/src/acquisition.rs:1843 | Native staging and canonical download part must be distinct files |  |
| src-tauri/src/acquisition.rs:1883 | Native download stage ended before its completed size |  |
| src-tauri/src/acquisition.rs:1886 | Adopting native download bytes: {error} |  |
| src-tauri/src/acquisition.rs:1902 | Native download stage changed while its bytes were being adopted |  |
| src-tauri/src/acquisition.rs:1969 | Native download has no resolved URL |  |
| src-tauri/src/acquisition.rs:1996 | Native download did not provide a complete successful archive response |  |
| src-tauri/src/acquisition.rs:1999 | Native completion has no durable stage |  |
| src-tauri/src/acquisition.rs:2001 | Native completion has no device receipt |  |
| src-tauri/src/acquisition.rs:2002 | Native completion has no inode receipt |  |
| src-tauri/src/acquisition.rs:2030 | Native background download failed |  |
| src-tauri/src/acquisition.rs:2031 | missing |  |
| src-tauri/src/acquisition.rs:2032 | Native download returned an unknown transport state |  |
| src-tauri/src/android_evaluation.rs:38 | Invalid fixture relative path |  |
| src-tauri/src/android_evaluation.rs:99 | Run size overflow |  |
| src-tauri/src/android_evaluation.rs:115 | Unknown evaluation case |  |
| src-tauri/src/android_evaluation.rs:116 | Case exceeds bounds or lacks validity assertions |  |
| src-tauri/src/android_evaluation.rs:117 | Article assertion exceeds bound |  |
| src-tauri/src/android_evaluation.rs:118 | Search assertion exceeds bound |  |
| src-tauri/src/android_evaluation.rs:141 | Negative case unexpectedly published output |  |
| src-tauri/src/android_evaluation.rs:142 | cancel |  |
| src-tauri/src/android_evaluation.rs:142 | Map composition cancelled |  |
| src-tauri/src/android_evaluation.rs:142 | Composed ZIM MD5 mismatch |  |
| src-tauri/src/android_evaluation.rs:142 | Negative case failed for an unrelated reason |  |
| src-tauri/src/android_evaluation.rs:143 | Negative case lost journal |  |
| src-tauri/src/android_evaluation.rs:144 | Negative case did not exercise real writing |  |
| src-tauri/src/android_evaluation.rs:145 | output |  |
| src-tauri/src/android_evaluation.rs:145 | Negative case left output or partial |  |
| src-tauri/src/android_evaluation.rs:146 | assertions |  |
| src-tauri/src/android_evaluation.rs:146 | observedError |  |
| src-tauri/src/android_evaluation.rs:146 | noPublishedOutput |  |
| src-tauri/src/android_evaluation.rs:146 | noPartial |  |
| src-tauri/src/android_evaluation.rs:146 | wroteBytes |  |
| src-tauri/src/android_evaluation.rs:159 | Missing parent |  |
| src-tauri/src/android_evaluation.rs:159 | Missing name |  |
| src-tauri/src/android_evaluation.rs:162 | Retained-byte ZIM checksum failed |  |
| src-tauri/src/android_evaluation.rs:243 | schema |  |
| src-tauri/src/android_evaluation.rs:243 | ok |  |
| src-tauri/src/android_evaluation.rs:243 | error |  |
| src-tauri/src/android_evaluation.rs:243 | Native evaluation panicked |  |
| src-tauri/src/android_notifications_service.rs:51 | Invalid Android notification profile identity |  |
| src-tauri/src/android_notifications_service.rs:87 | Android notification profile unavailable |  |
| src-tauri/src/android_notifications_service.rs:95 | available |  |
| src-tauri/src/android_notifications_service.rs:95 | reason |  |
| src-tauri/src/android_notifications_service.rs:100 | main |  |
| src-tauri/src/android_notifications_service.rs:100 | Completion notification permission requires the trusted root window |  |
| src-tauri/src/android_notifications_service.rs:115 | granted |  |
| src-tauri/src/android_notifications_service.rs:115 | Missing Android notification permission result |  |
| src-tauri/src/android_notifications_service.rs:121 | Notification permission expired after a privacy or lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs:124 | Mobile lifecycle unavailable |  |
| src-tauri/src/android_notifications_service.rs:125 | Notification permission expired after a lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs:194 | Android completion notification: {error} |  |
| src-tauri/src/android_notifications_service.rs:205 | tap |  |
| src-tauri/src/android_notifications_service.rs:205 | Download notifications are unavailable. Restart the app |  |
| src-tauri/src/android_notifications_service.rs:217 | Launch queue unavailable |  |
| src-tauri/src/android_notifications_service.rs:229 | Too many pending launch locations |  |
| src-tauri/src/android_notifications_service.rs:232 | launchId |  |
| src-tauri/src/android_notifications_service.rs:246 | Notification opens belong to the trusted root window |  |
| src-tauri/src/android_notifications_service.rs:249 | __notificationGeneration |  |
| src-tauri/src/android_notifications_service.rs:250 | Notification open expired after a privacy or lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs:252 | __notification |  |
| src-tauri/src/android_notifications_service.rs:254 | The notified archive installation is no longer available |  |
| src-tauri/src/android_notifications_service.rs:256 | takeCompletionNotificationTap |  |
| src-tauri/src/android_notifications_service.rs:257 | The notification open is no longer pending |  |
| src-tauri/src/android_notifications_service.rs:259 | Notification open expired after a lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs:261 | acknowledgeCompletionNotificationTap |  |
| src-tauri/src/android_notifications_service.rs:261 | id |  |
| src-tauri/src/android_tls.rs:74 | the Android application context is unavailable |  |
| src-tauri/src/archive.rs:203 | Invalid native archive response buffer |  |
| src-tauri/src/archive.rs:225 | {category}: {detail} |  |
| src-tauri/src/archive.rs:266 | Integrity check cancelled |  |
| src-tauri/src/archive.rs:274 | Retained archive size overflow |  |
| src-tauri/src/archive.rs:285 | Retained archive part size changed |  |
| src-tauri/src/archive.rs:304 | Retained archive exceeds its checksum boundary |  |
| src-tauri/src/archive.rs:327 | Archive filesystem path is not valid UTF-8 |  |
| src-tauri/src/archive.rs:338 | Native archive open returned a null handle |  |
| src-tauri/src/archive.rs:363 | Invalid retained archive part count |  |
| src-tauri/src/archive.rs:409 | Native retained archive open returned a null handle |  |
| src-tauri/src/archive.rs:431 | Search cancelled |  |
| src-tauri/src/archive.rs:457 | Invalid native archive JSON response: {error} |  |
| src-tauri/src/archive.rs:487 | Archive read exceeds 16 MiB range limit; use successive ranges |  |
| src-tauri/src/archive.rs:490 | Archive range does not fit this platform's address space |  |
| src-tauri/src/archive.rs:515 | Native archive read returned an invalid byte count or buffer |  |
| src-tauri/src/archive.rs:630 | Evaluation requires retained archive files |  |
| src-tauri/src/archive.rs:633 | android-evaluation |  |
| src-tauri/src/archive.rs:633 | android |  |
| src-tauri/src/archive.rs:643 | Archive illustration exceeds 2 MiB |  |
| src-tauri/src/archive_source.rs:51 | External document-provider references require their original mobile platform; select the archive again on this device |  |
| src-tauri/src/catalog.rs:103 | This is not an official BitTorrent download endpoint |  |
| src-tauri/src/catalog.rs:109 | The BitTorrent download link is too large |  |
| src-tauri/src/catalog.rs:110 | The publisher returned an invalid BitTorrent link |  |
| src-tauri/src/catalog.rs:112 | magnet |  |
| src-tauri/src/catalog.rs:127 | The publisher returned no unambiguous BitTorrent v1 download |  |
| src-tauri/src/catalog.rs:129 | The publisher returned an invalid BitTorrent info hash |  |
| src-tauri/src/catalog.rs:158 | URL exceeds 8192 bytes |  |
| src-tauri/src/catalog.rs:160 | Invalid URL: {e} |  |
| src-tauri/src/catalog.rs:168 | HTTPS is required (downloads additionally permit literal loopback HTTP fixture URLs) |  |
| src-tauri/src/catalog.rs:177 | URL must have a host and no credentials or fragment |  |
| src-tauri/src/catalog.rs:213 | HTTP {} for {} |  |
| src-tauri/src/catalog.rs:219 | Response exceeds {maximum} bytes |  |
| src-tauri/src/catalog.rs:273 | Facet root must be an Atom feed |  |
| src-tauri/src/catalog.rs:298 | Facet feed exceeds {MAX_FACETS} distinct values |  |
| src-tauri/src/catalog.rs:561 | Catalog root must be an Atom feed |  |
| src-tauri/src/catalog.rs:564 | entry |  |
| src-tauri/src/catalog.rs:580 | Acquisition link has no href |  |
| src-tauri/src/catalog.rs:583 | title |  |
| src-tauri/src/catalog.rs:585 | Acquisition entry requires an id and title |  |
| src-tauri/src/catalog.rs:588 | link |  |
| src-tauri/src/catalog.rs:602 | href |  |
| src-tauri/src/catalog.rs:602 | Image link has no href |  |
| src-tauri/src/catalog.rs:633 | Torrent acquisition has no href |  |
| src-tauri/src/catalog.rs:634 | magnet: |  |
| src-tauri/src/catalog.rs:636 | Invalid magnet URL: {e} |  |
| src-tauri/src/catalog.rs:645 | Invalid magnet acquisition link |  |
| src-tauri/src/catalog.rs:671 | Acquisition entry exceeds {MAX_ENTRY_TAGS} distinct tags |  |
| src-tauri/src/catalog.rs:736 | Next link has no href |  |
| src-tauri/src/catalog.rs:738 | totalResults |  |
| src-tauri/src/catalog_cache.rs:105 | A saved catalog page is damaged. Refresh the catalog |  |
| src-tauri/src/catalog_cache.rs:147 | Catalog cache directory attachment changed |  |
| src-tauri/src/catalog_cache.rs:192 | Reading catalog index: {error} |  |
| src-tauri/src/catalog_cache.rs:196 | Catalog index is not a bounded, unlinked regular file |  |
| src-tauri/src/catalog_cache.rs:200 | Catalog index exceeds disk limit |  |
| src-tauri/src/catalog_cache.rs:207 | Catalog cache directory unavailable |  |
| src-tauri/src/catalog_cache.rs:208 | catalog-index-v1.pending |  |
| src-tauri/src/catalog_cache.rs:208 | catalog-index-v1.json |  |
| src-tauri/src/catalog_cache.rs:272 | Loading catalog cache: {e} |  |
| src-tauri/src/catalog_cache.rs:291 | The catalog did not refresh. Try again |  |
| src-tauri/src/catalog_cache.rs:319 | {error}; {warning} |  |
| src-tauri/src/catalog_cache.rs:322 | offline |  |
| src-tauri/src/catalog_cache.rs:354 | Too many HTTP redirects |  |
| src-tauri/src/catalog_cache.rs:356 | OPDS redirect has no valid Location |  |
| src-tauri/src/catalog_cache.rs:357 | Invalid OPDS redirect: {e} |  |
| src-tauri/src/catalog_cache.rs:359 | https |  |
| src-tauri/src/catalog_cache.rs:359 | http |  |
| src-tauri/src/catalog_cache.rs:359 | Unsafe HTTP redirect |  |
| src-tauri/src/catalog_cache.rs:403 | Creating catalog cache: {error} |  |
| src-tauri/src/catalog_cache.rs:406 | cache |  |
| src-tauri/src/catalog_cache.rs:409 | Opening catalog cache: {error} |  |
| src-tauri/src/catalog_cache.rs:428 | Catalog cache permission descriptor changed identity |  |
| src-tauri/src/catalog_cache.rs:467 | Catalog representation exceeds durable cache capacity |  |
| src-tauri/src/catalog_cache.rs:476 | Your privacy settings changed, so the catalog did not update. Open the catalog again |  |
| src-tauri/src/catalog_cache.rs:490 | Reading catalog cache: {error} |  |
| src-tauri/src/catalog_cache.rs:494 | Catalog cache is not a bounded, unlinked regular file |  |
| src-tauri/src/catalog_cache.rs:497 | Reading catalog cache: {e} |  |
| src-tauri/src/catalog_cache.rs:498 | Catalog cache exceeds disk limit |  |
| src-tauri/src/catalog_cache.rs:499 | Catalog cache unavailable: {e} |  |
| src-tauri/src/catalog_cache.rs:500 | Unsupported or oversized catalog cache |  |
| src-tauri/src/catalog_cache.rs:537 | Catalog cache scratch file is unsafe |  |
| src-tauri/src/catalog_cache.rs:550 | Catalog cache scratch file changed |  |
| src-tauri/src/catalog_cache.rs:556 | Catalog cache publication changed |  |
| src-tauri/src/catalog_index.rs:210 | Catalog snapshot exceeds durable index capacity | snapshot |
| src-tauri/src/catalog_index.rs:215 | Catalog index lock failed |  |
| src-tauri/src/catalog_index.rs:273 | Unknown catalog selection. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs:276 | Catalog edition selection is unavailable in private mode |  |
| src-tauri/src/catalog_index.rs:284 | Unknown catalog source. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs:300 | The catalog selection is stale. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs:303 | Unknown or conflicting catalog entry. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs:305 | The catalog selection has invalid source identity |  |
| src-tauri/src/catalog_index.rs:313 | Unsupported catalog index |  |
| src-tauri/src/catalog_index.rs:322 | Saved catalog index has invalid source identity or bounds |  |
| src-tauri/src/catalog_index.rs:324 | traversed |  |
| src-tauri/src/catalog_index.rs:326 | Saved catalog traversal has invalid completeness |  |
| src-tauri/src/catalog_index.rs:342 | Saved catalog page membership is invalid |  |
| src-tauri/src/catalog_index.rs:351 | Saved catalog entry has invalid source membership |  |
| src-tauri/src/crash.rs:162 | unknown |  |
| src-tauri/src/crash.rs:332 | Kiwix could not read the saved crash report. Close this prompt and try again |  |
| src-tauri/src/crash.rs:336 | The saved crash report is too large to open. Delete it from Troubleshooting |  |
| src-tauri/src/crash.rs:347 | The saved crash report is damaged. Delete it from Troubleshooting |  |
| src-tauri/src/crash.rs:349 | The saved crash report was written by a different version of Kiwix. Delete it from Troubleshooting |  |
| src-tauri/src/crash.rs:376 | Kiwix could not delete the saved crash report. Try again |  |
| src-tauri/src/desktop_workflow.rs:18 | Unknown desktop dialog kind |  |
| src-tauri/src/desktop_workflow.rs:25 | Invalid desktop dialog path |  |
| src-tauri/src/desktop_workflow.rs:29 | Desktop dialog paths must be native absolute paths |  |
| src-tauri/src/desktop_workflow.rs:37 | Desktop dialog selection must name a file |  |
| src-tauri/src/desktop_workflow.rs:43 | Desktop dialog parent is not a native Unicode path |  |
| src-tauri/src/desktop_workflow.rs:76 | Invalid suggested export filename |  |
| src-tauri/src/desktop_workflow.rs:84 | Desktop dialog path is not Unicode |  |
| src-tauri/src/device_scan.rs:86 | The file list is out of date. Scan your device again |  |
| src-tauri/src/device_scan.rs:94 | Invalid {what} in device scan |  |
| src-tauri/src/device_scan.rs:136 | Device scan returned too many archives |  |
| src-tauri/src/device_scan.rs:141 | Device scan returned too many limitations |  |
| src-tauri/src/device_scan.rs:143 | limitation |  |
| src-tauri/src/device_scan.rs:154 | archive name |  |
| src-tauri/src/device_scan.rs:156 | Invalid archive name in device scan |  |
| src-tauri/src/device_scan.rs:159 | Device scan archive has too many parts |  |
| src-tauri/src/device_scan.rs:172 | Invalid archive part in device scan |  |
| src-tauri/src/device_scan.rs:177 | Device scan archive name does not match its parts |  |
| src-tauri/src/device_scan.rs:179 | volume |  |
| src-tauri/src/device_scan.rs:180 | location |  |
| src-tauri/src/device_scan.rs:187 | Unknown device scan source |  |
| src-tauri/src/device_scan.rs:193 | Device scan entry has no source |  |
| src-tauri/src/diagnostics.rs:81 | Cannot write opt-in diagnostics: {error} |  |
| src-tauri/src/disk_warning.rs:27 | Not enough free space for this download |  |
| src-tauri/src/disk_warning.rs:35 | Download destination must be an absolute directory |  |
| src-tauri/src/disk_warning.rs:47 | Download destination ancestor is not a directory |  |
| src-tauri/src/disk_warning.rs:49 | Invalid download destination |  |
| src-tauri/src/disk_warning.rs:51 | Inspecting download volume: {e} |  |
| src-tauri/src/disk_warning.rs:54 | Download destination has no accessible filesystem ancestor |  |
| src-tauri/src/disk_warning.rs:94 | Download destination changed while checking its free space |  |
| src-tauri/src/disk_warning.rs:96 | known remaining size |  |
| src-tauri/src/disk_warning.rs:111 | {WARNING_PREFIX}{} |  |
| src-tauri/src/disk_warning.rs:112 | Encoding download warning: {e} |  |
| src-tauri/src/edition_service.rs:78 | {source}: {error} |  |
| src-tauri/src/edition_service.rs:86 | Configured catalogs changed; check archive editions again |  |
| src-tauri/src/edition_service.rs:89 | updates |  |
| src-tauri/src/edition_service.rs:89 | sources |  |
| src-tauri/src/edition_service.rs:89 | errors |  |
| src-tauri/src/edition_service.rs:102 | The installed archive changed; select its edition again |  |
| src-tauri/src/edition_service.rs:103 | intentId |  |
| src-tauri/src/edition_service.rs:107 | The retained edition decision changed; check for editions again |  |
| src-tauri/src/edition_service.rs:110 | The catalog artifact changed. The retained verified output was kept; select an edition again |  |
| src-tauri/src/edition_service.rs:116 | catalogGeneration |  |
| src-tauri/src/edition_service.rs:116 | Select the catalog edition again |  |
| src-tauri/src/edition_service.rs:122 | This catalog is no longer configured |  |
| src-tauri/src/edition_service.rs:124 | associationConfirmed |  |
| src-tauri/src/edition_service.rs:143 | The edition decision is no longer available |  |
| src-tauri/src/edition_service.rs:146 | The edition transfer record is unavailable; retained files have not been changed | record |
| src-tauri/src/edition_service.rs:150 | The catalog edition changed; select it again |  |
| src-tauri/src/edition_service.rs:152 | Edition destination has no parent |  |
| src-tauri/src/edition_service.rs:154 | Invalid edition output name |  |
| src-tauri/src/edition_service.rs:155 | diskSpaceAcknowledgement |  |
| src-tauri/src/edition_service.rs:156 | ios |  |
| src-tauri/src/edition_service.rs:185 | Unknown edition intent |  |
| src-tauri/src/edition_service.rs:186 | committed |  |
| src-tauri/src/edition_service.rs:186 | Use explicit rollback for a committed edition |  |
| src-tauri/src/edition_service.rs:205 | installationId |  |
| src-tauri/src/edition_service.rs:205 | Edition installation changed; review it again |  |
| src-tauri/src/edition_service.rs:209 | Stop sharing this archive before replacing its edition |  |
| src-tauri/src/edition_service.rs:219 | Wait for archive verification to finish before replacing |  |
| src-tauri/src/edition_service.rs:220 | decisionToken |  |
| src-tauri/src/edition_service.rs:222 | The catalog edition changed; review the verified output again |  |
| src-tauri/src/edition_service.rs:249 | Stop sharing this archive before rolling back its edition |  |
| src-tauri/src/edition_service.rs:268 | Unknown edition operation |  |
| src-tauri/src/edition_service.rs:296 | Edition errors unavailable |  |
| src-tauri/src/edition_updates.rs:129 | Edition changes require normal writable mode |  |
| src-tauri/src/edition_updates.rs:172 | Verified archive does not match the selected catalog edition |  |
| src-tauri/src/edition_updates.rs:174 | urn:uuid: |  |
| src-tauri/src/edition_updates.rs:175 | Catalog promises an invalid archive UUID | uuid |
| src-tauri/src/edition_updates.rs:177 | Downloaded archive UUID does not match the catalog's explicit UUID promise | uuid |
| src-tauri/src/edition_updates.rs:188 | Select this edition again before continuing |  |
| src-tauri/src/edition_updates.rs:211 | Select an unambiguous native catalog entry first |  |
| src-tauri/src/edition_updates.rs:215 | Select a currently configured catalog source |  |
| src-tauri/src/edition_updates.rs:221 | Confirm this archive's catalog association before preparing its edition |  |
| src-tauri/src/edition_updates.rs:228 | This catalog entry is not a comparable newer edition of the installed archive |  |
| src-tauri/src/edition_updates.rs:240 | Edition download directory changed. Select this edition again |  |
| src-tauri/src/edition_updates.rs:272 | Select this edition again before downloading |  |
| src-tauri/src/edition_updates.rs:275 | Opening edition download directory: {error} |  |
| src-tauri/src/edition_updates.rs:281 | Edition series changed. Select this edition again |  |
| src-tauri/src/edition_updates.rs:293 | Edition intent is no longer available |  |
| src-tauri/src/edition_updates.rs:299 | Edition intent changed. Select this edition again |  |
| src-tauri/src/edition_updates.rs:345 | reserved |  |
| src-tauri/src/edition_updates.rs:345 | downloading |  |
| src-tauri/src/edition_updates.rs:345 | ready |  |
| src-tauri/src/edition_updates.rs:346 | Download does not belong to this edition intent |  |
| src-tauri/src/edition_updates.rs:386 | Edition intent changed during verification |  |
| src-tauri/src/edition_updates.rs:426 | Select this edition again before replacing |  |
| src-tauri/src/edition_updates.rs:428 | Edition authorization belongs to another download |  |
| src-tauri/src/edition_updates.rs:438 | Verify this edition before replacing the installed archive |  |
| src-tauri/src/edition_updates.rs:446 | Edition intent changed before commit |  |
| src-tauri/src/edition_updates.rs:457 | A committed edition requires explicit rollback |  |
| src-tauri/src/edition_updates.rs:473 | Unknown edition transaction |  |
| src-tauri/src/edition_updates.rs:474 | This edition was already rolled back |  |
| src-tauri/src/edition_updates.rs:477 | The installed edition changed; rollback cannot overwrite it |  |
| src-tauri/src/edition_updates.rs:479 | Original archive is unavailable; reconnect its storage or provider: {error} |  |
| src-tauri/src/edition_updates.rs:481 | The retained original archive changed; rollback was not authorized |  |
| src-tauri/src/edition_updates.rs:485 | The original archive failed verification; neither edition was changed |  |
| src-tauri/src/edition_updates.rs:495 | Edition rollback authorization expired; choose rollback again |  |
| src-tauri/src/edition_updates.rs:499 | Original archive changed after rollback verification |  |
| src-tauri/src/edition_updates.rs:506 | Rollback transaction disappeared |  |
| src-tauri/src/edition_updates.rs:512 | Edition authorization expired; select this edition again |  |
| src-tauri/src/edition_updates.rs:520 | Download location changed; select this edition again |  |
| src-tauri/src/edition_updates.rs:524 | Catalog source changed; select this edition again |  |
| src-tauri/src/edition_updates.rs:533 | Verified import does not belong to this edition intent |  |
| src-tauri/src/edition_updates.rs:534 | Bind the edition download before verification |  |
| src-tauri/src/edition_updates.rs:536 | Edition receipt belongs to another acquisition |  |
| src-tauri/src/edition_updates.rs:551 | Archive relocation changed its installation or content metadata |  |
| src-tauri/src/edition_updates.rs:561 | Edition relocation no longer matches its transaction |  |
| src-tauri/src/edition_updates.rs:581 | Edition acquisition was already consumed |  |
| src-tauri/src/edition_updates.rs:585 | Edition cutover requires its exact durable verified reservation |  |
| src-tauri/src/edition_updates.rs:589 | The verified edition is already installed separately; no archive was overwritten |  |
| src-tauri/src/edition_updates.rs:610 | Edition reservation disappeared |  |
| src-tauri/src/edition_updates.rs:619 | Edition transaction is no longer available |  |
| src-tauri/src/edition_updates.rs:622 | Edition transaction changed; rollback was not authorized |  |
| src-tauri/src/edition_updates.rs:627 | The original edition is already installed separately |  |
| src-tauri/src/edition_updates.rs:646 | Reference revision exhausted |  |
| src-tauri/src/edition_updates.rs:647 | Archive installation changed |  |
| src-tauri/src/edition_updates.rs:677 | Session reference revision is newer than this profile |  |
| src-tauri/src/edition_updates.rs:700 | Saved catalog association is invalid |  |
| src-tauri/src/edition_updates.rs:708 | Saved edition intent is invalid; preserve the profile and restore its backup |  |
| src-tauri/src/edition_updates.rs:719 | Saved edition transaction is invalid |  |
| src-tauri/src/edition_updates.rs:726 | Invalid reference revision |  |
| src-tauri/src/edition_updates.rs:730 | Saved edition reference-transition history is invalid |  |
| src-tauri/src/edition_updates.rs:733 | Saved reference revision has missing transitions |  |
| src-tauri/src/edition_updates.rs:738 | Session-only provider references cannot be retained by an edition transaction |  |
| src-tauri/src/exports.rs:59 | MHTML export requires an HTML article |  |
| src-tauri/src/exports.rs:62 | This archive is damaged. Remove it from your library and add it again |  |
| src-tauri/src/exports.rs:87 | MHTML package exceeds the 2 GiB source-data limit |  |
| src-tauri/src/exports.rs:92 | Text asset {} exceeds the 16 MiB parsing limit |  |
| src-tauri/src/exports.rs:132 | Article exceeds the 32768 static-reference limit |  |
| src-tauri/src/exports.rs:139 | Conflicting redirected asset locations: {url} |  |
| src-tauri/src/exports.rs:145 | Article exceeds the 4096 packaged-asset limit |  |
| src-tauri/src/exports.rs:211 | Invalid or oversized archive resource path |  |
| src-tauri/src/exports.rs:216 | Missing archive resource path |  |
| src-tauri/src/exports.rs:232 | Archive path contains dot segments that cannot be represented as an MHTML URL |  |
| src-tauri/src/exports.rs:242 | Asset URL exceeds the interoperable MHTML header limit: {location} |  |
| src-tauri/src/exports.rs:253 | Archive resource has an invalid MIME header |  |
| src-tauri/src/exports.rs:277 | Unexpected end of archive resource {} at byte {offset} |  |
| src-tauri/src/exports.rs:282 | Archive returned bytes beyond the resource size |  |
| src-tauri/src/exports.rs:369 | Export destination has no filename |  |
| src-tauri/src/exports.rs:444 | Static asset URL exceeds 16 KiB |  |
| src-tauri/src/exports.rs:467 | Too many static references in one text asset |  |
| src-tauri/src/exports.rs:744 | Unterminated CSS string in article assets |  |
| src-tauri/src/exports.rs:750 | Unescaped newline in CSS string |  |
| src-tauri/src/exports.rs:758 | import |  |
| src-tauri/src/exports.rs:760 | url |  |
| src-tauri/src/exports.rs:760 | src |  |
| src-tauri/src/exports.rs:832 | CSS static-reference limit exceeded |  |
| src-tauri/src/hotspot.rs:21 | Android returned an invalid hotspot name |  |
| src-tauri/src/hotspot.rs:24 | Android returned an invalid hotspot password |  |
| src-tauri/src/hotspot.rs:29 | Android returned hotspot credentials with control characters |  |
| src-tauri/src/hotspot.rs:85 | Android has not published the hotspot interface yet; wait for the hotspot to finish starting and try again |  |
| src-tauri/src/hotspot.rs:219 | The Wi-Fi hotspot is already starting |  |
| src-tauri/src/hotspot.rs:220 | The Wi-Fi hotspot is already running |  |
| src-tauri/src/hotspot.rs:221 | The Wi-Fi hotspot is still stopping |  |
| src-tauri/src/imports.rs:132 | Archive changed after verification; retry the import |  |
| src-tauri/src/imports.rs:160 | Archive verification omitted its retained content identities |  |
| src-tauri/src/imports.rs:212 | Archive verification is shut down |  |
| src-tauri/src/imports.rs:218 | Acquisition receipt already belongs to different archive paths |  |
| src-tauri/src/imports.rs:233 | Archive verification requires a receipt and archive paths |  |
| src-tauri/src/imports.rs:363 | Verified import is no longer available |  |
| src-tauri/src/imports.rs:455 | Archive verification queue is full |  |
| src-tauri/src/imports.rs:460 | Unknown acquisition receipt |  |
| src-tauri/src/imports.rs:462 | Only failed archive verification can be retried |  |
| src-tauri/src/imports.rs:507 | Archive verification worker ownership missing |  |
| src-tauri/src/imports.rs:509 | Archive verification worker panicked |  |
| src-tauri/src/imports.rs:520 | Archive verification worker did not retire |  |
| src-tauri/src/instance.rs:189 | Unsupported instance protocol version |  |
| src-tauri/src/instance.rs:191 | activate |  |
| src-tauri/src/instance.rs:192 | Unsupported instance action |  |
| src-tauri/src/instance.rs:195 | Too many launch arguments |  |
| src-tauri/src/instance.rs:202 | Unsupported launch payload |  |
| src-tauri/src/instance.rs:206 | Launch payload has no absolute working directory |  |
| src-tauri/src/instance.rs:254 | Profile path is too long for a local socket ({}); second instances will explain the conflict instead |  |
| src-tauri/src/instance.rs:274 | Kiwix instance: accept failed: {error} |  |
| src-tauri/src/instance.rs:292 | {} is not a directory |  |
| src-tauri/src/instance.rs:298 | {} is owned by another user |  |
| src-tauri/src/instance.rs:369 | The owning instance rejected the activation request |  |
| src-tauri/src/instance.rs:398 | No profile rendezvous |  |
| src-tauri/src/instance.rs:428 | Unauthorized peer |  |
| src-tauri/src/instance.rs:460 | Request exceeds the instance protocol limit |  |
| src-tauri/src/instance.rs:466 | Empty request |  |
| src-tauri/src/instance.rs:468 | Invalid request: {error} |  |
| src-tauri/src/instance.rs:480 | {"status":"error"} |  |
| src-tauri/src/integrity.rs:113 | An archive integrity check is already running |  |
| src-tauri/src/integrity.rs:116 | Integrity worker unavailable |  |
| src-tauri/src/integrity.rs:121 | Archive not in library |  |
| src-tauri/src/integrity.rs:223 | checksum |  |
| src-tauri/src/integrity.rs:223 | structure |  |
| src-tauri/src/integrity.rs:231 | Archive changed during integrity check |  |
| src-tauri/src/integrity.rs:237 | Archive no longer in library |  |
| src-tauri/src/ios_background.rs:118 | Native application delegate class is unavailable before application run |  |
| src-tauri/src/ios_background.rs:127 | Native background transport returned no response |  |
| src-tauri/src/ios_background.rs:130 | Reading native background response: {error} |  |
| src-tauri/src/lib.rs:46 | --data-dir requires a path |  |
| src-tauri/src/lib.rs:49 | --port |  |
| src-tauri/src/lib.rs:52 | --port requires a number |  |
| src-tauri/src/lib.rs:54 | Invalid port |  |
| src-tauri/src/lib.rs:59 | --article requires an archive article path |  |
| src-tauri/src/lib.rs:62 | --help |  |
| src-tauri/src/lib.rs:62 | -h |  |
| src-tauri/src/lib.rs:74 | Unknown option: {arg} |  |
| src-tauri/src/lib.rs:79 | --article requires exactly one archive path |  |
| src-tauri/src/lib.rs:94 | Close this window to continue |  |
| src-tauri/src/lib.rs:98 | This action could not be started. Try again |  |
| src-tauri/src/lib.rs:99 | __windowLabel |  |
| src-tauri/src/lib.rs:119 | Portable mode requires adjacent data/; conflicting --data-dir is not permitted (the matching data path is allowed) |  |
| src-tauri/src/lib.rs:131 | Cannot locate application data directory; provide --data-dir |  |
| src-tauri/src/lib.rs:157 | Cannot identify profile {}: {error} |  |
| src-tauri/src/lib.rs:188 | {reason}. No archives were opened or imported. Quit the running Kiwix instance, then repeat this command to use the requested profile and mode. |  |
| src-tauri/src/lib.rs:228 | system |  |
| src-tauri/src/lib.rs:232 | {}\n\n{error} |  |
| src-tauri/src/lib.rs:249 | No native application is available to open the archive |  |
| src-tauri/src/lib.rs:251 | Cannot open archives while application Quit is pending |  |
| src-tauri/src/lib.rs:259 | No native window is available to open the archive |  |
| src-tauri/src/lib.rs:306 | Command-line arguments: {error} |  |
| src-tauri/src/lib.rs:417 | path |  |
| src-tauri/src/lib.rs:417 | Read-only mode can open registered archives only |  |
| src-tauri/src/lib.rs:439 | Imported archives payload is missing its book list |  |
| src-tauri/src/lib.rs:477 | recover_migrations |  |
| src-tauri/src/lib.rs:496 | maps |  |
| src-tauri/src/lib.rs:508 | library_maintenance |  |
| src-tauri/src/lib.rs:927 | Window groups could not be restored: {error} |  |
| src-tauri/src/library_files.rs:56 | Archive parts must be regular files without hard links |  |
| src-tauri/src/library_files.rs:63 | Archive parts must be regular files |  |
| src-tauri/src/library_files.rs:100 | Directory identity changed: {} |  |
| src-tauri/src/library_files.rs:119 | Not a regular archive file: {name} |  |
| src-tauri/src/library_files.rs:130 | Archive file changed while opening |  |
| src-tauri/src/library_files.rs:166 | Invalid single-component archive filename |  |
| src-tauri/src/library_files.rs:179 | Archive streams are not regular archive filenames |  |
| src-tauri/src/library_files.rs:187 | Archive filename must be valid Unicode |  |
| src-tauri/src/library_files.rs:219 | Owned library operation directory must be private to this user (0700) |  |
| src-tauri/src/library_files.rs:236 | Owned operation directory was replaced; nothing was taken from {} |  |
| src-tauri/src/library_files.rs:256 | Owned operation directory capability unavailable |  |
| src-tauri/src/library_files.rs:260 | Owned operation directory was replaced |  |
| src-tauri/src/library_files.rs:311 | File identity changed: {name} |  |
| src-tauri/src/library_files.rs:316 | Archive part too large |  |
| src-tauri/src/library_files.rs:326 | Archive changed size while hashing |  |
| src-tauri/src/library_files.rs:330 | File changed while checking: {name} |  |
| src-tauri/src/library_files.rs:363 | Select a .zim or first .zimaa archive part |  |
| src-tauri/src/library_files.rs:392 | Archive is missing a declared split part |  |
| src-tauri/src/library_files.rs:393 | Split archive has ambiguous case-colliding part names |  |
| src-tauri/src/library_files.rs:398 | The exact selected archive primary is missing |  |
| src-tauri/src/library_files.rs:402 | Empty split archive part |  |
| src-tauri/src/library_files.rs:407 | Archive header part changed before reading |  |
| src-tauri/src/library_files.rs:410 | Reading ZIM header: {e} |  |
| src-tauri/src/library_files.rs:415 | Archive header part changed while reading |  |
| src-tauri/src/library_files.rs:418 | File is not a ZIM archive |  |
| src-tauri/src/library_files.rs:420 | Invalid ZIM checksum position |  |
| src-tauri/src/library_files.rs:425 | Archive part lengths overflow |  |
| src-tauri/src/library_files.rs:427 | Archive parts exceed their declared checksum boundary |  |
| src-tauri/src/library_files.rs:434 | Archive is missing parts, exceeds the 676-part bound, or has trailing data |  |
| src-tauri/src/library_files.rs:437 | Kiwix could not verify this archive, so no files were deleted. Remove it from your library instead |  |
| src-tauri/src/library_files.rs:450 | Archive changed during verification |  |
| src-tauri/src/library_files.rs:482 | Archive changed while retaining its exact part set |  |
| src-tauri/src/library_files.rs:515 | Archive has no parent directory |  |
| src-tauri/src/library_files.rs:543 | Duplicate archive has no parent directory |  |
| src-tauri/src/library_files.rs:568 | This staged copy changed while it was being removed |  |
| src-tauri/src/library_files.rs:576 | This staged copy is not the archive that is already in the library; nothing was removed |  |
| src-tauri/src/library_files.rs:593 | Removing this staged copy failed: {error} |  |
| src-tauri/src/library_files.rs:629 | Archive changed during source admission |  |
| src-tauri/src/library_files.rs:641 | Retained archive directory was replaced |  |
| src-tauri/src/library_files.rs:647 | Retained archive source changed |  |
| src-tauri/src/library_files.rs:664 | Kiwix could not verify this archive. Remove it from your library |  |
| src-tauri/src/library_files.rs:671 | Archive changed during source verification |  |
| src-tauri/src/library_files.rs:680 | This mobile platform cannot reveal archive folders in an external file manager. Use Export to save a copy. |  |
| src-tauri/src/library_files.rs:717 | This mobile platform has no supported OS Trash. Disable Move deleted archives to Trash and explicitly confirm permanent deletion. |  |
| src-tauri/src/library_files.rs:724 | Archive directory must be valid Unicode |  |
| src-tauri/src/library_files.rs:732 | This archive is not owned by the selected download |  |
| src-tauri/src/library_files.rs:734 | Downloaded archive changed before deletion |  |
| src-tauri/src/library_files.rs:752 | Deletion staging capability unavailable |  |
| src-tauri/src/library_files.rs:756 | Archive part changed before deletion |  |
| src-tauri/src/library_files.rs:760 | Staged archive changed identity |  |
| src-tauri/src/library_files.rs:773 | Captured archive no longer belongs to the selected download |  |
| src-tauri/src/library_files.rs:788 | Deletion staging directory contains unexpected files |  |
| src-tauri/src/library_files.rs:821 | Staged archive part changed; deletion stopped |  |
| src-tauri/src/library_files.rs:861 | {error}. Any retained archive parts are recoverable at {}. {} |  |
| src-tauri/src/library_files.rs:863 | ; |  |
| src-tauri/src/library_files.rs:957 | Migration directory has no ownership marker; no recovery performed |  |
| src-tauri/src/library_files.rs:982 | Migration journal exceeds size bound |  |
| src-tauri/src/library_files.rs:984 | {}.{}.tmp |  |
| src-tauri/src/library_files.rs:1011 | This action could not be verified. Try again |  |
| src-tauri/src/library_files.rs:1022 | Unowned or invalid migration journal; sources were not touched |  |
| src-tauri/src/library_files.rs:1025 | .zimaa |  |
| src-tauri/src/library_files.rs:1027 | Invalid migration archive set |  |
| src-tauri/src/library_files.rs:1042 | Invalid exact migration part manifest |  |
| src-tauri/src/library_files.rs:1050 | Journal is not bound to a managed library archive |  |
| src-tauri/src/library_files.rs:1054 | Library path no longer belongs to this migration; nothing was deleted |  |
| src-tauri/src/library_files.rs:1057 | Migration source and destination are identical |  |
| src-tauri/src/library_files.rs:1069 | Managed archive removed during migration |  |
| src-tauri/src/library_files.rs:1072 | Archive path changed during migration |  |
| src-tauri/src/library_files.rs:1078 | Destination path must be Unicode |  |
| src-tauri/src/library_files.rs:1088 | Archive part checksum changed: {} |  |
| src-tauri/src/library_files.rs:1108 | Migration directory identity changed; all remaining sources were preserved |  |
| src-tauri/src/library_files.rs:1112 | Migration filesystem identity changed |  |
| src-tauri/src/library_files.rs:1129 | Both migration names exist; source retained for manual recovery |  |
| src-tauri/src/library_files.rs:1173 | Missing published copy identity |  |
| src-tauri/src/library_files.rs:1186 | Migration staging directory was replaced |  |
| src-tauri/src/library_files.rs:1198 | Unowned migration destination collision; source retained |  |
| src-tauri/src/library_files.rs:1226 | Archive size changed while copying; source retained |  |
| src-tauri/src/library_files.rs:1230 | Copied archive checksum differs; source retained |  |
| src-tauri/src/library_files.rs:1241 | Missing copy identity |  |
| src-tauri/src/library_files.rs:1267 | Journal does not describe the exact verified archive set; no source cleanup performed |  |
| src-tauri/src/library_files.rs:1277 | Verified migration archive changed before backup policy |  |
| src-tauri/src/library_files.rs:1313 | Missing destination identity |  |
| src-tauri/src/library_files.rs:1328 | Source cleanup staging directory disappeared; remaining files were preserved |  |
| src-tauri/src/library_files.rs:1338 | Source cleanup staging directory was replaced |  |
| src-tauri/src/library_files.rs:1345 | Unowned source cleanup entries preserved at {} |  |
| src-tauri/src/library_files.rs:1360 | Source cleanup staging capability unavailable |  |
| src-tauri/src/library_files.rs:1368 | Source name reappeared during cleanup; both files were preserved |  |
| src-tauri/src/library_files.rs:1432 | {error}. Retained cleanup entries are recoverable at {}. {} |  |
| src-tauri/src/library_files.rs:1444 | Unsafe managed archive relative directory |  |
| src-tauri/src/library_files.rs:1466 | Download directory must be an absolute normalized path |  |
| src-tauri/src/library_files.rs:1478 | Download directory has no accessible filesystem root |  |
| src-tauri/src/library_files.rs:1493 | Read-only mode cannot move archives |  |
| src-tauri/src/library_files.rs:1497 | Save the new download directory before moving previous downloads |  |
| src-tauri/src/library_files.rs:1504 | Download migration directories cannot contain one another |  |
| src-tauri/src/library_files.rs:1506 | Cannot create migration journals |  |
| src-tauri/src/library_files.rs:1535 | Archive has no relative parent |  |
| src-tauri/src/library_files.rs:1537 | Archive has no parent |  |
| src-tauri/src/library_files.rs:1544 | Destination already exists: {} |  |
| src-tauri/src/library_files.rs:1581 | moved |  |
| src-tauri/src/library_files.rs:1581 | skipped |  |
| src-tauri/src/library_files.rs:1606 | Too many migration journals; automatic recovery stopped without sweeping files |  |
| src-tauri/src/library_files.rs:1623 | Invalid journal filename |  |
| src-tauri/src/library_files.rs:1633 | Oversized migration journal |  |
| src-tauri/src/library_files.rs:1646 | Migration journal filename mismatch |  |
| src-tauri/src/library_files.rs:1732 | Choose a folder inside Kiwix's own storage |  |
| src-tauri/src/library_files.rs:1763 | Archive size overflow |  |
| src-tauri/src/library_files.rs:1807 | A saved backup could not be read. Restart the app |  |
| src-tauri/src/library_files.rs:1811 | Acquisition backup evidence exceeds its state limit |  |
| src-tauri/src/library_files.rs:1977 | Native import returned a path outside its owned staging directory |  |
| src-tauri/src/library_files.rs:1979 | Imported document has no directory |  |
| src-tauri/src/library_files.rs:1984 | Native import returned duplicate document paths |  |
| src-tauri/src/library_files.rs:2008 | .zim |  |
| src-tauri/src/library_files.rs:2008 | Selected document is not a ZIM archive or exact split part |  |
| src-tauri/src/library_files.rs:2010 | Selected document is not an exact split archive part |  |
| src-tauri/src/library_files.rs:2012 | {}aa |  |
| src-tauri/src/library_files.rs:2014 | Split archive selection is incomplete: {} requires its .zimaa primary and full cohort |  |
| src-tauri/src/library_files.rs:2032 | Export has no directory |  |
| src-tauri/src/library_files.rs:2062 | Kiwix will not clean up files outside its own export folder |  |
| src-tauri/src/library_files.rs:2074 | Prepared export changed identity; retained at {} |  |
| src-tauri/src/library_files.rs:2106 | Invalid canonical attachment path |  |
| src-tauri/src/library_files.rs:2109 | This attachment type cannot be opened safely in an external application. Use Save attachment instead. |  |
| src-tauri/src/library_files.rs:2111 | External attachments must be between 1 byte and 512 MiB. Use Save attachment for larger files. |  |
| src-tauri/src/library_files.rs:2115 | Attachment contents do not match the permitted document/media type. Use Save attachment instead. |  |
| src-tauri/src/library_files.rs:2118 | attachment-exports-v1 |  |
| src-tauri/src/library_files.rs:2128 | Attachment ended before its declared size |  |
| src-tauri/src/library_files.rs:2255 | Exclusive archive rename: {} |  |
| src-tauri/src/library_root.rs:161 | Archive ownership requires a regular file |  |
| src-tauri/src/library_root.rs:166 | Archive ownership requires a regular file without hard links |  |
| src-tauri/src/library_root.rs:197 | Native storage enumeration returned no locations |  |
| src-tauri/src/library_root.rs:199 | Native storage enumeration returned too many locations |  |
| src-tauri/src/library_root.rs:208 | Native storage location is not an absolute normalized path |  |
| src-tauri/src/library_root.rs:210 | internal |  |
| src-tauri/src/library_root.rs:210 | external |  |
| src-tauri/src/library_root.rs:211 | Native storage location has an unknown kind |  |
| src-tauri/src/library_root.rs:306 | Read-only mode cannot change the library location |  |
| src-tauri/src/library_root.rs:309 | Private mode cannot copy or move the library |  |
| src-tauri/src/library_root.rs:313 | A previous storage move is still pending. Restart Kiwix to finish it before changing the location again. |  |
| src-tauri/src/library_root.rs:318 | Library location must be an absolute normalized path |  |
| src-tauri/src/library_root.rs:322 | Choose one of the storage locations this app exposes |  |
| src-tauri/src/library_root.rs:326 | The current library location is not an absolute normalized path |  |
| src-tauri/src/library_root.rs:336 | Library locations cannot contain one another |  |
| src-tauri/src/library_root.rs:346 | Library size overflow |  |
| src-tauri/src/library_root.rs:398 | {error}. Copies and the recovery journal were preserved; restart Kiwix to finish the storage move. |  |
| src-tauri/src/library_root.rs:401 | {error}. Partial copies were preserved for recovery: {rollback} |  |
| src-tauri/src/library_root.rs:416 | Private or read-only mode cannot copy or move archives |  |
| src-tauri/src/library_root.rs:419 | A storage operation is pending. Restart Kiwix to finish recovery first |  |
| src-tauri/src/library_root.rs:425 | Select at least one archive to copy or move |  |
| src-tauri/src/library_root.rs:432 | An archive was selected more than once |  |
| src-tauri/src/library_root.rs:433 | Selected archive is no longer in the library |  |
| src-tauri/src/library_root.rs:434 | Reconnect the selected archive before copying it |  |
| src-tauri/src/library_root.rs:437 | Only app-managed archives or native provider selections can be copied or moved |  |
| src-tauri/src/library_root.rs:439 | Selected archive has no parent directory |  |
| src-tauri/src/library_root.rs:441 | A selected archive is already at that location |  |
| src-tauri/src/library_root.rs:448 | A selected archive failed verification; no files were moved |  |
| src-tauri/src/library_root.rs:454 | Archive parts must not be empty |  |
| src-tauri/src/library_root.rs:457 | Selected archive has no parts |  |
| src-tauri/src/library_root.rs:491 | {error}. Copies and the recovery journal were preserved; restart Kiwix to finish recovery |  |
| src-tauri/src/library_root.rs:500 | Storage selection is missing |  |
| src-tauri/src/library_root.rs:508 | Storage selection entry is missing |  |
| src-tauri/src/library_root.rs:529 | This platform cannot delete files from that document provider. Choose Copy instead |  |
| src-tauri/src/library_root.rs:533 | Provider source missing |  |
| src-tauri/src/library_root.rs:534 | Kiwix could not confirm permission to delete the original, so it was kept |  |
| src-tauri/src/library_root.rs:538 | Copied provider digest is missing; original and journal were preserved |  |
| src-tauri/src/library_root.rs:539 | size |  |
| src-tauri/src/library_root.rs:539 | sha256 |  |
| src-tauri/src/library_root.rs:544 | Provider deletion is unavailable on this platform; original and journal were preserved |  |
| src-tauri/src/library_root.rs:565 | Selected archives changed during recovery; all copies and the journal were preserved |  |
| src-tauri/src/library_root.rs:615 | Selected source part disappeared |  |
| src-tauri/src/library_root.rs:619 | Archive source was replaced while copying |  |
| src-tauri/src/library_root.rs:622 | Library migration journal is missing a part |  |
| src-tauri/src/library_root.rs:648 | A file already exists at {}; nothing was overwritten |  |
| src-tauri/src/library_root.rs:653 | Checking library destination: {error} |  |
| src-tauri/src/library_root.rs:663 | Library destination path is not UTF-8 |  |
| src-tauri/src/library_root.rs:671 | A library archive was removed during the storage move |  |
| src-tauri/src/library_root.rs:675 | Selected archive source is missing |  |
| src-tauri/src/library_root.rs:678 | A selected archive changed during copying |  |
| src-tauri/src/library_root.rs:683 | A library archive changed during the storage move |  |
| src-tauri/src/library_root.rs:719 | from |  |
| src-tauri/src/library_root.rs:720 | to |  |
| src-tauri/src/library_root.rs:721 | removedOld |  |
| src-tauri/src/library_root.rs:737 | Provider source disappeared |  |
| src-tauri/src/library_root.rs:739 | The managed copy is ready, but its original is retained for edition rollback; keep the original archive and its provider permission |  |
| src-tauri/src/library_root.rs:750 | An original archive changed; copies and recovery journal were preserved |  |
| src-tauri/src/library_root.rs:752 | Removing original archive: {error} |  |
| src-tauri/src/library_root.rs:755 | Checking original archive before cleanup: {error} |  |
| src-tauri/src/library_root.rs:821 | Replacement archive is no longer admitted; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs:825 | Replacement installation changed; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs:831 | Replacement archive is missing or changed; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs:838 | Replacement archive part set changed; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs:854 | Publishing archive without overwriting: {} |  |
| src-tauri/src/library_root.rs:895 | The library contains too many archives for one storage move |  |
| src-tauri/src/library_root.rs:908 | Managed archive has no directory below the library root |  |
| src-tauri/src/library_root.rs:918 | Archive parts are not in the same directory |  |
| src-tauri/src/library_root.rs:923 | Archive part filename must be valid Unicode |  |
| src-tauri/src/library_root.rs:926 | Reading archive part size: {error} |  |
| src-tauri/src/library_root.rs:950 | The library location must not be a symbolic link |  |
| src-tauri/src/library_root.rs:953 | The library location is not a directory |  |
| src-tauri/src/library_root.rs:958 | Creating library location: {error} |  |
| src-tauri/src/library_root.rs:960 | Checking library location: {error} |  |
| src-tauri/src/library_root.rs:967 | Library location marker is invalid |  |
| src-tauri/src/library_root.rs:972 | That folder belongs to a different Kiwix profile |  |
| src-tauri/src/library_root.rs:1016 | Native external storage location is not absolute |  |
| src-tauri/src/library_root.rs:1018 | archives |  |
| src-tauri/src/library_root.rs:1059 | Not enough space at the new location: {needed} bytes are needed but {free} bytes are free |  |
| src-tauri/src/library_root.rs:1079 | Archive part changed before it could be copied |  |
| src-tauri/src/library_root.rs:1102 | Archive part length overflow |  |
| src-tauri/src/library_root.rs:1104 | Archive part changed size while copying |  |
| src-tauri/src/library_root.rs:1121 | Copied archive has no parent directory |  |
| src-tauri/src/library_root.rs:1124 | Opening copied archive part for verification: {error} |  |
| src-tauri/src/library_root.rs:1132 | A copied archive did not pass verification; the previous library was kept |  |
| src-tauri/src/library_root.rs:1182 | Select fewer archives: the storage recovery journal would be too large |  |
| src-tauri/src/library_root.rs:1192 | Storage migration journal is oversized |  |
| src-tauri/src/library_root.rs:1194 | Reading storage migration journal: {error} |  |
| src-tauri/src/library_root.rs:1199 | Storage migration journal has an unsupported version |  |
| src-tauri/src/library_root.rs:1205 | Storage migration journal has an invalid ID |  |
| src-tauri/src/library_root.rs:1208 | Storage migration journal belongs to a different profile |  |
| src-tauri/src/library_root.rs:1213 | Storage migration journal has invalid roots |  |
| src-tauri/src/library_root.rs:1216 | Storage migration journal has overlapping roots |  |
| src-tauri/src/library_root.rs:1218 | .kiwix-library-root-{} |  |
| src-tauri/src/library_root.rs:1219 | Storage migration journal has an invalid staging name |  |
| src-tauri/src/library_root.rs:1222 | Storage migration journal has too many entries |  |
| src-tauri/src/library_root.rs:1226 | Storage selection has an invalid source count |  |
| src-tauri/src/library_root.rs:1232 | Storage selection has duplicate or missing sources |  |
| src-tauri/src/library_root.rs:1234 | Storage selection has an invalid installation |  |
| src-tauri/src/library_root.rs:1238 | Provider move receipt is missing |  |
| src-tauri/src/library_root.rs:1239 | Provider move receipt is invalid |  |
| src-tauri/src/library_root.rs:1243 | Kiwix could not confirm which original files belong to this move. The files were kept |  |
| src-tauri/src/library_root.rs:1249 | Storage migration journal has an invalid archive ID |  |
| src-tauri/src/library_root.rs:1256 | Storage migration journal has an invalid part count |  |
| src-tauri/src/library_root.rs:1261 | Storage migration journal has an empty part |  |
| src-tauri/src/library_root.rs:1306 | Archive directory contains an unsafe component |  |
| src-tauri/src/library_root.rs:1327 | Invalid library filename |  |
| src-tauri/src/live_activity.rs:214 | liveActivity |  |
| src-tauri/src/localswarm/apple_transport.rs:34 | Cannot import the pinned LocalSwarm Apple identity |  |
| src-tauri/src/localswarm/apple_transport.rs:46 | Not a Bonjour endpoint |  |
| src-tauri/src/localswarm/apple_transport.rs:47 | Invalid Bonjour endpoint |  |
| src-tauri/src/localswarm/apple_transport.rs:49 | Invalid Bonjour service instance |  |
| src-tauri/src/localswarm/apple_transport.rs:142 | LocalSwarm operation cancelled |  |
| src-tauri/src/localswarm/apple_transport.rs:150 | Cannot create Apple QUIC connection |  |
| src-tauri/src/localswarm/apple_transport.rs:168 | The selected native QUIC interface changed |  |
| src-tauri/src/localswarm/apple_transport.rs:170 | peer-to-peer |  |
| src-tauri/src/localswarm/apple_transport.rs:170 | infrastructure |  |
| src-tauri/src/localswarm/apple_transport.rs:182 | Native QUIC has no live required interface |  |
| src-tauri/src/localswarm/apple_transport.rs:193 | Native QUIC has no live effective local address |  |
| src-tauri/src/localswarm/apple_transport.rs:283 | Cannot create Apple QUIC listener |  |
| src-tauri/src/localswarm/apple_transport.rs:285 | Apple QUIC listener callback lost |  |
| src-tauri/src/localswarm/apple_transport.rs:286 | Invalid Apple QUIC listener port |  |
| src-tauri/src/localswarm/cleanup.rs:19 | localswarm: |  |
| src-tauri/src/localswarm/cleanup.rs:19 | Invalid LocalSwarm cleanup ID |  |
| src-tauri/src/localswarm/cleanup.rs:20 | .kiwix-localswarm-{suffix}.cleanup |  |
| src-tauri/src/localswarm/cleanup.rs:29 | Invalid LocalSwarm cleanup ownership receipt |  |
| src-tauri/src/localswarm/cleanup.rs:33 | LocalSwarm cleanup receipt does not identify the retained files |  |
| src-tauri/src/localswarm/cleanup.rs:47 | LocalSwarm cleanup parent was replaced; retained files were not touched |  |
| src-tauri/src/localswarm/cleanup.rs:55 | LocalSwarm cleanup directory ownership changed; all entries were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:65 | Unexpected entries in LocalSwarm cleanup directory; all entries were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:77 | LocalSwarm cleanup file ownership changed; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:93 | Published LocalSwarm archive changed; remove it explicitly from the library instead |  |
| src-tauri/src/localswarm/cleanup.rs:111 | LocalSwarm cleanup has no ownership receipt for {}; all entries were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:117 | LocalSwarm has no publication ownership proof for the existing destination |  |
| src-tauri/src/localswarm/cleanup.rs:129 | LocalSwarm partial files disappeared before cleanup; ownership was retained |  |
| src-tauri/src/localswarm/cleanup.rs:141 | LocalSwarm retained bytes changed during cleanup admission |  |
| src-tauri/src/localswarm/cleanup.rs:157 | LocalSwarm file disappeared before cleanup capture; ownership was retained |  |
| src-tauri/src/localswarm/cleanup.rs:161 | An unowned file appeared in LocalSwarm cleanup; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:164 | LocalSwarm file changed after cleanup confirmation; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:198 | Both LocalSwarm cleanup and source names exist; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:208 | Missing LocalSwarm cleanup file receipt |  |
| src-tauri/src/localswarm/cleanup.rs:211 | LocalSwarm cleanup file was replaced; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:218 | Unexpected entries appeared during LocalSwarm cleanup; directory was preserved |  |
| src-tauri/src/localswarm/cleanup.rs:246 | Missing LocalSwarm original publication digests |  |
| src-tauri/src/localswarm/cleanup.rs:251 | Missing LocalSwarm publication ownership stamp |  |
| src-tauri/src/localswarm/cleanup.rs:253 | LocalSwarm publication changed before library-removal proof was retained |  |
| src-tauri/src/localswarm/cleanup.rs:259 | LocalSwarm publication changed while library-removal proof was retained |  |
| src-tauri/src/localswarm/cleanup.rs:285 | LocalSwarm job not found |  |
| src-tauri/src/localswarm/cleanup.rs:288 | Stop this LocalSwarm worker before preparing file cleanup |  |
| src-tauri/src/localswarm/cleanup.rs:300 | Files appeared after this LocalSwarm cleanup completed; they cannot authorize library removal |  |
| src-tauri/src/localswarm/cleanup.rs:307 | Both LocalSwarm cleanup and source names exist; cleanup ownership must be resolved first |  |
| src-tauri/src/localswarm/cleanup.rs:352 | Stop this LocalSwarm transfer before removing its history |  |
| src-tauri/src/localswarm/cleanup.rs:378 | Stop this LocalSwarm transfer before forgetting its history |  |
| src-tauri/src/localswarm/cleanup.rs:419 | Stop this LocalSwarm transfer before deleting its files |  |
| src-tauri/src/localswarm/cleanup.rs:422 | A LocalSwarm worker is still using this download; stop it before deleting files |  |
| src-tauri/src/localswarm/cleanup.rs:442 | LocalSwarm transfer is no longer terminal; files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs:470 | Cannot commit LocalSwarm cleanup receipt: {}; history was retained |  |
| src-tauri/src/localswarm/discovery.rs:67 | No bounded reachable IPv4 library listener addresses |  |
| src-tauri/src/localswarm/discovery.rs:70 | Library sharing requires actual listener addresses |  |
| src-tauri/src/localswarm/discovery.rs:71 | Cannot advertise an unreachable library listener address |  |
| src-tauri/src/localswarm/discovery.rs:76 | No reachable IPv4 library listener address for this advertisement |  |
| src-tauri/src/localswarm/discovery.rs:91 | The shared library is no longer available; find nearby and browse again |  |
| src-tauri/src/localswarm/discovery.rs:113 | Invalid LocalSwarm peer ID |  |
| src-tauri/src/localswarm/discovery.rs:121 | LocalSwarm discovery is stopped |  |
| src-tauri/src/localswarm/discovery.rs:131 | Find nearby before browsing a shared library |  |
| src-tauri/src/localswarm/discovery.rs:133 | The shared library is no longer nearby; find nearby and browse again |  |
| src-tauri/src/localswarm/discovery.rs:145 | Stop the failed discovery session before restarting |  |
| src-tauri/src/localswarm/discovery.rs:146 | Cannot browse local network: {e} |  |
| src-tauri/src/localswarm/discovery.rs:160 | Local network discovery stopped unexpectedly |  |
| src-tauri/src/localswarm/discovery.rs:204 | Invalid publisher name |  |
| src-tauri/src/localswarm/discovery.rs:205 | LocalSwarm advertisement limit or invalid port |  |
| src-tauri/src/localswarm/discovery.rs:208 | Invalid listener address |  |
| src-tauri/src/localswarm/discovery.rs:209 | Cannot advertise an unreachable LocalSwarm listener address |  |
| src-tauri/src/localswarm/discovery.rs:212 | No reachable local network address; use a direct connection instead |  |
| src-tauri/src/localswarm/discovery.rs:213 | Kiwix-{} |  |
| src-tauri/src/localswarm/discovery.rs:214 | kiwix-{}.local. |  |
| src-tauri/src/localswarm/discovery.rs:235 | Invalid library session, listener port, or advertisement limit |  |
| src-tauri/src/localswarm/discovery.rs:238 | No bounded reachable library listener addresses |  |
| src-tauri/src/localswarm/discovery.rs:241 | Invalid library listener address |  |
| src-tauri/src/localswarm/discovery.rs:262 | The previous library announcement is still stopping; stop sharing before starting it again |  |
| src-tauri/src/localswarm/discovery.rs:265 | {fullname}#{} |  |
| src-tauri/src/localswarm/discovery.rs:274 | Invalid owned LocalSwarm announcement |  |
| src-tauri/src/localswarm/discovery.rs:277 | LocalSwarm unadvertise timed out |  |
| src-tauri/src/localswarm/discovery.rs:277 | LocalSwarm discovery daemon stopped |  |
| src-tauri/src/localswarm/discovery.rs:287 | Cannot stop LocalSwarm discovery: {e} |  |
| src-tauri/src/localswarm/discovery.rs:290 | LocalSwarm discovery shutdown timed out |  |
| src-tauri/src/localswarm/discovery.rs:290 | LocalSwarm discovery shutdown failed |  |
| src-tauri/src/localswarm/discovery.rs:291 | LocalSwarm discovery did not stop |  |
| src-tauri/src/localswarm/import.rs:17 | Nearby publication has no archive |  |
| src-tauri/src/localswarm/import.rs:25 | This nearby download is no longer available for import |  |
| src-tauri/src/localswarm/import.rs:38 | Nearby download parent or publication changed; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs:44 | Nearby download ownership or split parts changed; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs:47 | Nearby download contains unexpected files; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs:53 | Nearby download file was replaced or changed; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs:68 | Archive publication verification was cancelled |  |
| src-tauri/src/localswarm/import.rs:85 | Nearby download ended before its verified size; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs:92 | Nearby download bytes changed after publication; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs:95 | Nearby download size does not match its manifest |  |
| src-tauri/src/localswarm/import.rs:129 | Nearby download history changed during import; retry the import |  |
| src-tauri/src/localswarm/import.rs:140 | Nearby download no longer exists |  |
| src-tauri/src/localswarm/import.rs:148 | Nearby publication changed during verification; retry the import |  |
| src-tauri/src/localswarm/import.rs:176 | LocalSwarm is disabled in private/read-only mode |  |
| src-tauri/src/localswarm/import.rs:179 | localswarm |  |
| src-tauri/src/localswarm/import.rs:188 | LocalSwarm state exceeds its size limit |  |
| src-tauri/src/localswarm/ios_discovery.rs:149 | Invalid library session, count or listener port |  |
| src-tauri/src/localswarm/ios_discovery.rs:260 | Bonjour operation limit reached |  |
| src-tauri/src/localswarm/ios_discovery.rs:265 | Bonjour returned no operation |  |
| src-tauri/src/localswarm/ios_discovery.rs:349 | {name}.{SERVICE} |  |
| src-tauri/src/localswarm/ios_discovery.rs:351 | The previous announcement is still stopping; stop sharing before starting it again |  |
| src-tauri/src/localswarm/ios_discovery.rs:568 | LocalSwarm TXT exceeds limit |  |
| src-tauri/src/localswarm/library.rs:29 | Missing library session |  |
| src-tauri/src/localswarm/library.rs:39 | The selected Nearby share retired | retired |
| src-tauri/src/localswarm/library.rs:51 | The selected Nearby library is no longer available |  |
| src-tauri/src/localswarm/library.rs:52 | macos |  |
| src-tauri/src/localswarm/library.rs:58 | Native Apple service endpoints are unavailable on this platform |  |
| src-tauri/src/localswarm/library.rs:88 | Nearby sharing stopped |  |
| src-tauri/src/localswarm/library.rs:92 | Archive installation is no longer shared |  |
| src-tauri/src/localswarm/library.rs:145 | Add an available archive before sharing your library |  |
| src-tauri/src/localswarm/library.rs:146 | The library exceeds the Nearby inventory limit |  |
| src-tauri/src/localswarm/library.rs:147 | Invalid installed archive identifier |  |
| src-tauri/src/localswarm/library.rs:153 | The library inventory exceeds its transport bound |  |
| src-tauri/src/localswarm/library.rs:161 | The shared library session retired | retired |
| src-tauri/src/localswarm/library.rs:226 | Invalid library completion acknowledgement |  |
| src-tauri/src/localswarm/library.rs:235 | Library session changed |  |
| src-tauri/src/localswarm/library.rs:253 | Shared archive is unavailable or changed | shared archive |
| src-tauri/src/localswarm/library.rs:303 | The selected Nearby library session changed |  |
| src-tauri/src/localswarm/library.rs:311 | Invalid library response metadata |  |
| src-tauri/src/localswarm/library.rs:322 | Unsolicited library range response |  |
| src-tauri/src/localswarm/library.rs:325 | Library range response does not match the requested validator and bounds |  |
| src-tauri/src/localswarm/library.rs:332 | Nearby library is unavailable |  |
| src-tauri/src/localswarm/library.rs:333 | Library inventory is missing |  |
| src-tauri/src/localswarm/library.rs:335 | Invalid library inventory identity |  |
| src-tauri/src/localswarm/library.rs:340 | Invalid library inventory book |  |
| src-tauri/src/localswarm/library.rs:356 | Library response was interrupted |  |
| src-tauri/src/localswarm/library.rs:368 | Shared archive changed or is no longer available | shared archive |
| src-tauri/src/localswarm/library.rs:369 | Truncated library range |  |
| src-tauri/src/localswarm/library.rs:370 | Invalid library data frame length |  |
| src-tauri/src/localswarm/library.rs:375 | Truncated library range payload |  |
| src-tauri/src/localswarm/library.rs:389 | Trailing library range payload |  |
| src-tauri/src/localswarm/library.rs:400 | Invalid library range response |  |
| src-tauri/src/localswarm/library.rs:403 | content-length |  |
| src-tauri/src/localswarm/library_route.rs:24 | The selected Nearby interface retired | retired |
| src-tauri/src/localswarm/library_route.rs:31 | The selected native QUIC interface retired | retired |
| src-tauri/src/localswarm/library_route.rs:41 | The selected local address belongs to multiple interfaces |  |
| src-tauri/src/localswarm/library_route.rs:45 | The bound QUIC local address has no live interface |  |
| src-tauri/src/localswarm/library_route.rs:59 | The selected Nearby interface was replaced |  |
| src-tauri/src/localswarm/library_route.rs:63 | The selected native interface has no index |  |
| src-tauri/src/localswarm/library_route.rs:65 | The selected Nearby local address retired; browse again | retired |
| src-tauri/src/localswarm/library_route.rs:86 | The selected Nearby route is no longer local |  |
| src-tauri/src/localswarm/library_route.rs:98 | Invalid native QUIC interface receipt |  |
| src-tauri/src/localswarm/library_route.rs:111 | The selected Nearby route changed; browse again |  |
| src-tauri/src/localswarm/library_transport.rs:31 | Nearby share retired | retired |
| src-tauri/src/localswarm/library_transport.rs:32 | Nearby joined route retired | retired |
| src-tauri/src/localswarm/library_transport.rs:33 | Nearby local policy changed |  |
| src-tauri/src/localswarm/library_transport.rs:34 | Nearby route retired | retired |
| src-tauri/src/localswarm/library_transport.rs:101 | Trailing library response payload |  |
| src-tauri/src/localswarm/library_transport.rs:103 | made no progress |  |
| src-tauri/src/localswarm/library_transport.rs:119 | Library completion failed: {reason} |  |
| src-tauri/src/localswarm/library_transport.rs:144 | Native QUIC supplied no exact route receipt |  |
| src-tauri/src/localswarm/library_transport.rs:158 | The selected native QUIC interface is unavailable |  |
| src-tauri/src/localswarm/library_transport.rs:160 | No nearby QUIC endpoint |  |
| src-tauri/src/localswarm/library_transport.rs:207 | not a local peer |  |
| src-tauri/src/localswarm/library_transport.rs:323 | The supplied library socket does not own its advertised address |  |
| src-tauri/src/localswarm/library_wire.rs:35 | Invalid library request |  |
| src-tauri/src/localswarm/library_wire.rs:41 | Library control exceeds its bound |  |
| src-tauri/src/localswarm/library_wire.rs:51 | Invalid library control length |  |
| src-tauri/src/localswarm/library_wire.rs:54 | Invalid library control: {e} |  |
| src-tauri/src/localswarm/library_wire.rs:58 | bytes= |  |
| src-tauri/src/localswarm/library_wire.rs:58 | Invalid library range |  |
| src-tauri/src/localswarm/library_wire.rs:60 | Unsatisfiable library range |  |
| src-tauri/src/localswarm/mod.rs:219 | Offered archive directory changed |  |
| src-tauri/src/localswarm/mod.rs:222 | Offered archive changed; prepare a new offer |  |
| src-tauri/src/localswarm/mod.rs:301 | Nearby sharing stopped. Start it again |  |
| src-tauri/src/localswarm/mod.rs:314 | LocalSwarm verification stopped: {e} |  |
| src-tauri/src/localswarm/mod.rs:318 | LocalSwarm operation stopped |  |
| src-tauri/src/localswarm/mod.rs:321 | Select one nonempty ZIM archive or its complete contiguous split set |  |
| src-tauri/src/localswarm/mod.rs:325 | Only one .zim or a complete .zimaa split-ZIM set is supported |  |
| src-tauri/src/localswarm/mod.rs:328 | {base}{}{} |  |
| src-tauri/src/localswarm/mod.rs:328 | Split-ZIM parts must be one exact ordered contiguous set beginning with .zimaa |  |
| src-tauri/src/localswarm/mod.rs:358 | Invalid LocalSwarm job ID |  |
| src-tauri/src/localswarm/mod.rs:359 | Invalid LocalSwarm job identity |  |
| src-tauri/src/localswarm/mod.rs:362 | .kiwix-localswarm-{suffix}.part |  |
| src-tauri/src/localswarm/mod.rs:368 | queued |  |
| src-tauri/src/localswarm/mod.rs:368 | Invalid persisted LocalSwarm destination or manifest |  |
| src-tauri/src/localswarm/mod.rs:369 | Invalid LocalSwarm completion receipt |  |
| src-tauri/src/localswarm/mod.rs:370 | Incomplete LocalSwarm job contains import paths |  |
| src-tauri/src/localswarm/mod.rs:372 | LocalSwarm retained ownership does not cover the manifest |  |
| src-tauri/src/localswarm/mod.rs:376 | This nearby download cannot be cleaned up yet. Wait for it to finish |  |
| src-tauri/src/localswarm/mod.rs:385 | LocalSwarm history exceeds the state limit; remove finished history |  |
| src-tauri/src/localswarm/mod.rs:410 | Too many saved LocalSwarm jobs |  |
| src-tauri/src/localswarm/mod.rs:425 | Invalid completed LocalSwarm file identities |  |
| src-tauri/src/localswarm/mod.rs:514 | localswarm-profiling |  |
| src-tauri/src/localswarm/mod.rs:520 | manager.lock |  |
| src-tauri/src/localswarm/mod.rs:521 | peer-id |  |
| src-tauri/src/localswarm/mod.rs:521 | Invalid LocalSwarm peer identity |  |
| src-tauri/src/localswarm/mod.rs:608 | Library generation exhausted | generation |
| src-tauri/src/localswarm/mod.rs:609 | library-generation |  |
| src-tauri/src/localswarm/mod.rs:663 | Share your library before connecting a new route |  |
| src-tauri/src/localswarm/mod.rs:666 | The shared library session changed |  |
| src-tauri/src/localswarm/mod.rs:763 | Delete retained LocalSwarm downloads and their files before starting another transfer (64 retained jobs maximum) |  |
| src-tauri/src/localswarm/mod.rs:782 | This archive is already in your library. Open the existing copy instead. |  |
| src-tauri/src/localswarm/mod.rs:784 | This archive is already in your library, but its file is unavailable. Locate or remove that entry in Library before downloading it again. |  |
| src-tauri/src/localswarm/mod.rs:821 | {}; staged files retained at {} |  |
| src-tauri/src/localswarm/mod.rs:842 | Saving LocalSwarm transfer: {e} |  |
| src-tauri/src/localswarm/mod.rs:861 | This LocalSwarm transfer was dismissed or entered file cleanup and cannot be resumed |  |
| src-tauri/src/localswarm/mod.rs:867 | Enter this protected swarm's PIN again to resume |  |
| src-tauri/src/localswarm/mod.rs:886 | LocalSwarm transfer stopped while queued |  |
| src-tauri/src/localswarm/mod.rs:910 | LocalSwarm verified size exceeds manifest |  |
| src-tauri/src/localswarm/mod.rs:1129 | LocalSwarm staged bytes changed after chunk verification |  |
| src-tauri/src/localswarm/mod.rs:1138 | LocalSwarm staging unavailable |  |
| src-tauri/src/localswarm/mod.rs:1175 | Saving LocalSwarm route receipt: {} |  |
| src-tauri/src/localswarm/mod.rs:1184 | Stop an active LocalSwarm offer before adding another |  |
| src-tauri/src/localswarm/mod.rs:1185 | Select an available library archive |  |
| src-tauri/src/localswarm/mod.rs:1207 | localswarm-seed:{} |  |
| src-tauri/src/localswarm/mod.rs:1302 | LocalSwarm offer not found |  |
| src-tauri/src/localswarm/mod.rs:1334 | LocalSwarm verified size overflow |  |
| src-tauri/src/localswarm/mod.rs:1348 | Published LocalSwarm bytes no longer match their verified manifest |  |
| src-tauri/src/localswarm/mod.rs:1376 | Skipping partial download {}: {error} |  |
| src-tauri/src/localswarm/mod.rs:1429 | LocalSwarm ZIM header is truncated |  |
| src-tauri/src/localswarm/mod.rs:1444 | This archive changed while Kiwix was checking it. Remove it from your library and add it again |  |
| src-tauri/src/localswarm/mod.rs:1467 | Kiwix could not verify this archive. Choose the file again |  |
| src-tauri/src/localswarm/mod.rs:1469 | This file changed while Kiwix was checking it. Choose it again |  |
| src-tauri/src/localswarm/mod.rs:1490 | Verified LocalSwarm source changed |  |
| src-tauri/src/localswarm/mod.rs:1495 | Empty LocalSwarm archive parts are not supported |  |
| src-tauri/src/localswarm/mod.rs:1502 | LocalSwarm source parts must exactly cover the ZIM checksum boundary |  |
| src-tauri/src/localswarm/mod.rs:1507 | Archive exceeds LocalSwarm manifest limits |  |
| src-tauri/src/localswarm/mod.rs:1527 | Archive changed while preparing LocalSwarm offer |  |
| src-tauri/src/localswarm/mod.rs:1544 | Offered archive is no longer installed |  |
| src-tauri/src/localswarm/mod.rs:1547 | Offered archive installation changed |  |
| src-tauri/src/localswarm/mod.rs:1559 | Offered external archive changed; stop and prepare a new offer |  |
| src-tauri/src/localswarm/mod.rs:1578 | Offered archive chunk no longer matches its manifest |  |
| src-tauri/src/localswarm/network.rs:81 | Truncated LocalSwarm stream |  |
| src-tauri/src/localswarm/network.rs:148 | Invalid LocalSwarm certificate chain |  |
| src-tauri/src/localswarm/network.rs:160 | LocalSwarm exact certificate pin mismatch |  |
| src-tauri/src/localswarm/network.rs:227 | LocalSwarm peer made no progress |  |
| src-tauri/src/localswarm/network.rs:237 | LocalSwarm payload progress deadline exceeded |  |
| src-tauri/src/localswarm/network.rs:242 | LocalSwarm read failed: {e} |  |
| src-tauri/src/localswarm/network.rs:243 | Truncated LocalSwarm payload |  |
| src-tauri/src/localswarm/network.rs:300 | LocalSwarm chunk checksum mismatch |  |
| src-tauri/src/localswarm/network.rs:320 | Invalid LocalSwarm frame length |  |
| src-tauri/src/localswarm/network.rs:322 | LocalSwarm frame exceeds type limit |  |
| src-tauri/src/localswarm/network.rs:323 | Unexpected LocalSwarm frame for this session phase |  |
| src-tauri/src/localswarm/network.rs:327 | Invalid manifest frame length |  |
| src-tauri/src/localswarm/network.rs:333 | Unsolicited chunk response |  |
| src-tauri/src/localswarm/network.rs:335 | Malformed chunk response |  |
| src-tauri/src/localswarm/network.rs:337 | Bad chunk index |  |
| src-tauri/src/localswarm/network.rs:338 | Bad chunk length |  |
| src-tauri/src/localswarm/network.rs:341 | Unsolicited or inconsistent chunk response |  |
| src-tauri/src/localswarm/network.rs:381 | Local network policy interrupted LocalSwarm |  |
| src-tauri/src/localswarm/network.rs:382 | Mobile network policy interrupted LocalSwarm |  |
| src-tauri/src/localswarm/network.rs:420 | LocalSwarm is not accepting new receive sessions |  |
| src-tauri/src/localswarm/network.rs:499 | LocalSwarm peer stopped before the requested chunk |  |
| src-tauri/src/localswarm/network.rs:509 | Invalid LocalSwarm chunk pipeline window |  |
| src-tauri/src/localswarm/network.rs:564 | LocalSwarm control message flood |  |
| src-tauri/src/localswarm/network.rs:593 | Unexpected LocalSwarm message while fetching chunk |  |
| src-tauri/src/localswarm/network.rs:657 | LocalSwarm prefetch budget closed |  |
| src-tauri/src/localswarm/network.rs:758 | Invalid LocalSwarm connection details |  |
| src-tauri/src/localswarm/network.rs:760 | bonjour: |  |
| src-tauri/src/localswarm/network.rs:760 | bonjour-lan: |  |
| src-tauri/src/localswarm/network.rs:775 | Local network policy interrupted address lookup |  |
| src-tauri/src/localswarm/network.rs:776 | Mobile network policy interrupted address lookup |  |
| src-tauri/src/localswarm/network.rs:778 | LocalSwarm address lookup failed: {e} |  |
| src-tauri/src/localswarm/network.rs:784 | LocalSwarm host has no usable address |  |
| src-tauri/src/localswarm/network.rs:802 | policy interrupted |  |
| src-tauri/src/localswarm/network.rs:827 | protocol error |  |
| src-tauri/src/localswarm/network.rs:841 | LocalSwarm peer did not handshake for the requested swarm |  |
| src-tauri/src/localswarm/network.rs:854 | LocalSwarm manifest swarm mismatch |  |
| src-tauri/src/localswarm/network.rs:859 | Unexpected LocalSwarm message before manifest |  |
| src-tauri/src/localswarm/network.rs:862 | LocalSwarm control message flood before manifest |  |
| src-tauri/src/localswarm/network.rs:864 | LocalSwarm bitfield count does not match manifest |  |
| src-tauri/src/localswarm/network.rs:865 | Out-of-range LocalSwarm have |  |
| src-tauri/src/localswarm/network.rs:910 | [nearby] try {address} |  |
| src-tauri/src/localswarm/network.rs:915 | [nearby] {address} bind failed: {error} |  |
| src-tauri/src/localswarm/network.rs:915 | LocalSwarm socket failed: {error} |  |
| src-tauri/src/localswarm/network.rs:924 | [nearby] {address} route failed: {error} |  |
| src-tauri/src/localswarm/network.rs:925 | LocalSwarm route selection failed: {error} |  |
| src-tauri/src/localswarm/network.rs:932 | LocalSwarm route binding failed: {error} |  |
| src-tauri/src/localswarm/network.rs:936 | The selected LocalSwarm network changed while binding |  |
| src-tauri/src/localswarm/network.rs:942 | localswarm.local |  |
| src-tauri/src/localswarm/network.rs:955 | LocalSwarm connect timed out |  |
| src-tauri/src/localswarm/network.rs:956 | LocalSwarm QUIC connection failed: {e} |  |
| src-tauri/src/localswarm/network.rs:962 | local policy interrupted |  |
| src-tauri/src/localswarm/network.rs:974 | [nearby] {address} failed: {error} |  |
| src-tauri/src/localswarm/network.rs:1021 | LocalSwarm is not accepting new transfers |  |
| src-tauri/src/localswarm/network.rs:1028 | Serving a LocalSwarm transfer |  |
| src-tauri/src/localswarm/network.rs:1110 | Invalid LocalSwarm seed credentials |  |
| src-tauri/src/localswarm/network.rs:1113 | Manifest JSON too large |  |
| src-tauri/src/localswarm/network.rs:1126 | LocalSwarm listen failed: {e} |  |
| src-tauri/src/localswarm/network.rs:1264 | Missing LocalSwarm handshake |  |
| src-tauri/src/localswarm/network.rs:1279 | Cross-swarm LocalSwarm frame |  |
| src-tauri/src/localswarm/network.rs:1297 | LocalSwarm client reader stopped |  |
| src-tauri/src/localswarm/network.rs:1317 | Apple QUIC connection failed or left the local link |  |
| src-tauri/src/localswarm/network.rs:1319 | Apple QUIC send callback lost |  |
| src-tauri/src/localswarm/network.rs:1343 | No LocalSwarm send in flight |  |
| src-tauri/src/localswarm/network.rs:1359 | LocalSwarm source changed or returned corrupt bytes |  |
| src-tauri/src/localswarm/network.rs:1407 | LocalSwarm next-request progress deadline exceeded |  |
| src-tauri/src/localswarm/network.rs:1438 | LocalSwarm authentication failed |  |
| src-tauri/src/localswarm/network.rs:1465 | Unexpected LocalSwarm client message or authentication required |  |
| src-tauri/src/localswarm/network.rs:1519 | Out-of-range LocalSwarm chunk request |  |
| src-tauri/src/localswarm/network.rs:1529 | Inconsistent LocalSwarm bitfield count |  |
| src-tauri/src/localswarm/network.rs:1533 | Unexpected LocalSwarm client frame |  |
| src-tauri/src/localswarm/network.rs:1537 | LocalSwarm duplicate-control flood |  |
| src-tauri/src/localswarm/protocol.rs:65 | manifest array exceeds limit |  |
| src-tauri/src/localswarm/protocol.rs:97 | Unsafe manifest path |  |
| src-tauri/src/localswarm/protocol.rs:100 | . |  |
| src-tauri/src/localswarm/protocol.rs:100 | .. |  |
| src-tauri/src/localswarm/protocol.rs:103 | Unsafe manifest path component |  |
| src-tauri/src/localswarm/protocol.rs:109 | Unsafe normalized manifest path |  |
| src-tauri/src/localswarm/protocol.rs:112 | con |  |
| src-tauri/src/localswarm/protocol.rs:112 | prn |  |
| src-tauri/src/localswarm/protocol.rs:112 | aux |  |
| src-tauri/src/localswarm/protocol.rs:112 | nul |  |
| src-tauri/src/localswarm/protocol.rs:112 | clock$ |  |
| src-tauri/src/localswarm/protocol.rs:112 | conin$ |  |
| src-tauri/src/localswarm/protocol.rs:112 | conout$ |  |
| src-tauri/src/localswarm/protocol.rs:119 | Reserved manifest path |  |
| src-tauri/src/localswarm/protocol.rs:139 | Unsupported LocalSwarm manifest version or hash algorithm |  |
| src-tauri/src/localswarm/protocol.rs:146 | Manifest exceeds supported bounds |  |
| src-tauri/src/localswarm/protocol.rs:150 | Manifest ZIM identity is not a canonical UUID | uuid |
| src-tauri/src/localswarm/protocol.rs:159 | Colliding manifest paths |  |
| src-tauri/src/localswarm/protocol.rs:163 | Manifest chunk range overflow |  |
| src-tauri/src/localswarm/protocol.rs:166 | Inconsistent manifest chunk ranges |  |
| src-tauri/src/localswarm/protocol.rs:168 | Manifest chunk count overflow |  |
| src-tauri/src/localswarm/protocol.rs:169 | Too many manifest chunks |  |
| src-tauri/src/localswarm/protocol.rs:170 | Manifest size overflow |  |
| src-tauri/src/localswarm/protocol.rs:174 | Manifest file/directory collision |  |
| src-tauri/src/localswarm/protocol.rs:181 | Manifest content address, totals or hashes are inconsistent |  |
| src-tauri/src/localswarm/protocol.rs:187 | Chunk index out of range |  |
| src-tauri/src/localswarm/protocol.rs:190 | Chunk has no owning file |  |
| src-tauri/src/localswarm/protocol.rs:192 | Invalid chunk offset |  |
| src-tauri/src/localswarm/protocol.rs:198 | Invalid manifest JSON |  |
| src-tauri/src/localswarm/protocol.rs:225 | Unknown LocalSwarm message type |  |
| src-tauri/src/localswarm/protocol.rs:232 | Frame length overflow |  |
| src-tauri/src/localswarm/protocol.rs:233 | Truncated frame |  |
| src-tauri/src/localswarm/protocol.rs:242 | Frame field exceeds limit |  |
| src-tauri/src/localswarm/protocol.rs:262 | Empty frame |  |
| src-tauri/src/localswarm/protocol.rs:263 | Frame exceeds type limit |  |
| src-tauri/src/localswarm/protocol.rs:272 | Bitfield too large |  |
| src-tauri/src/localswarm/protocol.rs:274 | Inconsistent bitfield length |  |
| src-tauri/src/localswarm/protocol.rs:276 | Nonzero bitfield padding |  |
| src-tauri/src/localswarm/protocol.rs:285 | Unknown frame type |  |
| src-tauri/src/localswarm/protocol.rs:287 | Trailing frame data |  |
| src-tauri/src/localswarm/protocol.rs:318 | Outbound frame exceeds limit |  |
| src-tauri/src/localswarm/protocol.rs:319 | Frame too large |  |
| src-tauri/src/localswarm/sources.rs:33 | quic |  |
| src-tauri/src/localswarm/sources.rs:33 | Unsupported LocalSwarm discovered peer |  |
| src-tauri/src/localswarm/sources.rs:46 | The discovered LocalSwarm peer has no usable advertised address |  |
| src-tauri/src/localswarm/sources.rs:59 | Invalid LocalSwarm discovery selection |  |
| src-tauri/src/localswarm/sources.rs:60 | The selected LocalSwarm peer is no longer discovered; choose it again |  |
| src-tauri/src/localswarm/sources.rs:62 | The selected LocalSwarm share changed; inspect it again |  |
| src-tauri/src/localswarm/sources.rs:74 | LocalSwarm connection stopped |  |
| src-tauri/src/localswarm/sources.rs:76 | LocalSwarm connection has no address |  |
| src-tauri/src/localswarm/sources.rs:81 | Invalid discovered LocalSwarm address count |  |
| src-tauri/src/localswarm/sources.rs:90 | LocalSwarm address attempt stopped |  |
| src-tauri/src/localswarm/sources.rs:108 | LocalSwarm advertised address connection timed out |  |
| src-tauri/src/localswarm/sources.rs:242 | LocalSwarm receive admission closed during connection |  |
| src-tauri/src/localswarm/storage.rs:38 | LocalSwarm requires regular files without hard links |  |
| src-tauri/src/localswarm/storage.rs:42 | LocalSwarm sources must be regular files |  |
| src-tauri/src/localswarm/storage.rs:60 | LocalSwarm directory identity changed |  |
| src-tauri/src/localswarm/storage.rs:68 | LocalSwarm owned directories must be private (0700) |  |
| src-tauri/src/localswarm/storage.rs:95 | .state-{} |  |
| src-tauri/src/localswarm/storage.rs:117 | LocalSwarm cleanup source directory was replaced |  |
| src-tauri/src/localswarm/storage.rs:122 | LocalSwarm cleanup captured a replaced directory; all entries were preserved |  |
| src-tauri/src/localswarm/storage.rs:137 | LocalSwarm requires safe single-component filenames |  |
| src-tauri/src/localswarm/storage.rs:139 | COM |  |
| src-tauri/src/localswarm/storage.rs:139 | LPT |  |
| src-tauri/src/localswarm/storage.rs:139 | LocalSwarm filename is reserved |  |
| src-tauri/src/localswarm/storage.rs:157 | Not enough free space for this LocalSwarm archive |  |
| src-tauri/src/localswarm/storage.rs:166 | localswarm-{}-{suffix} |  |
| src-tauri/src/localswarm/storage.rs:177 | LocalSwarm destination parent was replaced |  |
| src-tauri/src/localswarm/storage.rs:183 | LocalSwarm staging ownership changed; retained data was not touched |  |
| src-tauri/src/localswarm/storage.rs:185 | Unexpected entries in LocalSwarm staging; retained data was not touched |  |
| src-tauri/src/localswarm/storage.rs:189 | LocalSwarm file ownership or size changed |  |
| src-tauri/src/localswarm/storage.rs:198 | LocalSwarm staging capability was released for publication; reopen the staged set |  |
| src-tauri/src/localswarm/storage.rs:201 | LocalSwarm staged handles are incomplete |  |
| src-tauri/src/localswarm/storage.rs:204 | Unexpected LocalSwarm staging entries |  |
| src-tauri/src/localswarm/storage.rs:208 | LocalSwarm staged file changed |  |
| src-tauri/src/localswarm/storage.rs:238 | LocalSwarm file changed before writer retirement |  |
| src-tauri/src/localswarm/storage.rs:251 | LocalSwarm file identity changed during writer retirement |  |
| src-tauri/src/localswarm/storage.rs:263 | Seal LocalSwarm writers before native verification |  |
| src-tauri/src/localswarm/storage.rs:277 | LocalSwarm verification handle no longer identifies the staged bytes |  |
| src-tauri/src/localswarm/storage.rs:292 | Invalid LocalSwarm chunk write |  |
| src-tauri/src/localswarm/storage.rs:322 | LocalSwarm set is already published |  |
| src-tauri/src/localswarm/storage.rs:331 | Seal LocalSwarm writers before publication |  |
| src-tauri/src/localswarm/storage.rs:379 | LocalSwarm file identity changed during publication |  |
| src-tauri/src/localswarm/storage.rs:388 | LocalSwarm chunk index is outside the manifest |  |
| src-tauri/src/localswarm/storage.rs:391 | linux |  |
| src-tauri/src/map_chunk_cache.rs:132 | Invalid map chunk cache name |  |
| src-tauri/src/map_chunk_cache.rs:160 | Cannot open map chunk cache entry: {error} |  |
| src-tauri/src/map_chunk_cache.rs:162 | Cannot inspect map chunk cache entry: {e} |  |
| src-tauri/src/map_chunk_cache.rs:201 | Map chunk cache directory changed identity |  |
| src-tauri/src/map_chunk_cache.rs:210 | Cannot inspect map chunk cache entry: {error} |  |
| src-tauri/src/map_chunk_cache.rs:228 | Cannot remove map chunk cache entry: {error} |  |
| src-tauri/src/map_chunk_cache.rs:241 | Map chunk exceeds cache capacity |  |
| src-tauri/src/map_chunk_cache.rs:245 | Map chunk cache accounting mismatch |  |
| src-tauri/src/map_chunk_cache.rs:247 | Known oldest cache entry |  |
| src-tauri/src/map_chunk_cache.rs:271 | Map chunk cache name is occupied by a non-regular entry |  |
| src-tauri/src/map_chunk_cache.rs:305 | Map chunk cache publication name became non-regular |  |
| src-tauri/src/map_chunk_cache.rs:308 | Cannot publish map chunk cache: {e} |  |
| src-tauri/src/map_chunk_cache.rs:337 | {original}; cannot remove owned map cache temporary {name}: {error} |  |
| src-tauri/src/map_chunk_cache.rs:372 | Map chunk cache name and payload size differ or exceed the entry limit |  |
| src-tauri/src/maps/compose.rs:65 | Map job history exceeds its state limit; previous claims were retained |  |
| src-tauri/src/maps/compose.rs:73 | Map state file has no parent |  |
| src-tauri/src/maps/compose.rs:75 | Cannot sync map job state directory: {e} |  |
| src-tauri/src/maps/compose.rs:95 | Accept |  |
| src-tauri/src/maps/compose.rs:95 | application/json |  |
| src-tauri/src/maps/compose.rs:95 | Accept-Encoding |  |
| src-tauri/src/maps/compose.rs:95 | StreetZim request failed: {e} |  |
| src-tauri/src/maps/compose.rs:99 | StreetZim JSON returned HTTP {} |  |
| src-tauri/src/maps/compose.rs:127 | StreetZim response exceeds allowed length |  |
| src-tauri/src/maps/compose.rs:134 | Unexpected compressed StreetZim response |  |
| src-tauri/src/maps/compose.rs:190 | Map disk requirement overflow |  |
| src-tauri/src/maps/compose.rs:202 | Insufficient disk space for map and downloaded parts |  |
| src-tauri/src/maps/compose.rs:231 | Missing source offset |  |
| src-tauri/src/maps/compose.rs:232 | Missing source length |  |
| src-tauri/src/maps/compose.rs:256 | Map cache requirement overflow |  |
| src-tauri/src/maps/compose.rs:331 | LOW_DISK_SPACE:{warning} |  |
| src-tauri/src/maps/compose.rs:359 | Map block count overflow |  |
| src-tauri/src/maps/compose.rs:361 | Map recipe is too complex to count blocks safely |  |
| src-tauri/src/maps/compose.rs:379 | Saved map block checkpoint does not match the exact recipe |  |
| src-tauri/src/maps/compose.rs:392 | Verified map block is not required by its recipe |  |
| src-tauri/src/maps/compose.rs:432 | Chunk index exceeds source |  |
| src-tauri/src/maps/compose.rs:435 | Chunk offset overflow |  |
| src-tauri/src/maps/compose.rs:440 | Chunk exceeds source |  |
| src-tauri/src/maps/compose.rs:442 | {expected}-{len} |  |
| src-tauri/src/maps/compose.rs:461 | Missing source mask |  |
| src-tauri/src/maps/compose.rs:462 | base |  |
| src-tauri/src/maps/compose.rs:507 | Missing source URL |  |
| src-tauri/src/maps/compose.rs:517 | Downloaded map chunk SHA-256 mismatch |  |
| src-tauri/src/maps/compose.rs:559 | Map chunk transfer failed |  |
| src-tauri/src/maps/compose.rs:580 | Invalid StreetZim range |  |
| src-tauri/src/maps/compose.rs:592 | Missing staged map source |  |
| src-tauri/src/maps/compose.rs:597 | Staged map source size changed |  |
| src-tauri/src/maps/compose.rs:606 | Staged map source range was truncated |  |
| src-tauri/src/maps/compose.rs:617 | Range |  |
| src-tauri/src/maps/compose.rs:617 | bytes={offset}-{end} |  |
| src-tauri/src/maps/compose.rs:618 | StreetZim range request failed: {e} |  |
| src-tauri/src/maps/compose.rs:628 | StreetZim source returned an invalid HTTP range response |  |
| src-tauri/src/maps/compose.rs:643 | StreetZim source range was truncated |  |
| src-tauri/src/maps/compose.rs:660 | Cached chunk length changed |  |
| src-tauri/src/maps/manager.rs:51 | Map job history exceeds its state limit; saved claims were not changed |  |
| src-tauri/src/maps/manager.rs:76 | This map download is damaged. Delete the download to start again |  |
| src-tauri/src/maps/manager.rs:78 | This map download changed unexpectedly. Delete the download and start again |  |
| src-tauri/src/maps/manager.rs:134 | Kiwix could not clean up a partial map download. Delete the download and start again |  |
| src-tauri/src/maps/manager.rs:174 | Map replacement authorization expired; select the map again |  |
| src-tauri/src/maps/manager.rs:185 | A map is already being composed |  |
| src-tauri/src/maps/manager.rs:189 | Map choice expired; select the map again |  |
| src-tauri/src/maps/manager.rs:193 | Explicit replacement is required only for the selected installed map |  |
| src-tauri/src/maps/manager.rs:229 | Verified map output changed while authorizing its import |  |
| src-tauri/src/maps/manager.rs:249 | Map publication is no longer the current verified job |  |
| src-tauri/src/maps/manager.rs:290 | Map destination must be an absolute directory |  |
| src-tauri/src/maps/manager.rs:320 | Resume, finish importing or delete the previous map download before starting another map |  |
| src-tauri/src/maps/manager.rs:456 | Map worker stopped unexpectedly; its last durable checkpoint was retained |  |
| src-tauri/src/maps/manager.rs:481 | Map pause omitted its durable result |  |
| src-tauri/src/maps/manager.rs:483 | Map state lock poisoned |  |
| src-tauri/src/maps/manager.rs:495 | Unknown map download |  |
| src-tauri/src/maps/manager.rs:514 | This map has no retained output checkpoint to resume; select Get map instead |  |
| src-tauri/src/maps/manager.rs:516 | Map checkpoint is missing |  |
| src-tauri/src/maps/manager.rs:517 | Map checkpoint ownership is missing |  |
| src-tauri/src/maps/manager.rs:519 | This older map has no resumable recipe authorization |  |
| src-tauri/src/maps/manager.rs:522 | Map choice expired after cancellation or a privacy transition; select the map again |  |
| src-tauri/src/maps/manager.rs:578 | Missing retained terminal map job |  |
| src-tauri/src/maps/manager.rs:580 | Cannot save map job state: {error} |  |
| src-tauri/src/maps/manager.rs:593 | Map worker omitted its retirement result |  |
| src-tauri/src/maps/manager.rs:631 | Recovered map lost its publication stamp |  |
| src-tauri/src/maps/manager.rs:638 | Map file ownership changed; no bytes were deleted |  |
| src-tauri/src/maps/manager.rs:679 | Pause the map before preparing file cleanup |  |
| src-tauri/src/maps/manager.rs:683 | This older map cannot be removed from Kiwix. Remove the map from your library instead |  |
| src-tauri/src/maps/manager.rs:695 | Map publication ownership has an invalid filename |  |
| src-tauri/src/maps/manager.rs:699 | The original map directory changed; no files were authorized for deletion |  |
| src-tauri/src/maps/manager.rs:711 | The original downloaded map file changed; no files were authorized for deletion |  |
| src-tauri/src/maps/manager.rs:715 | This older map cannot be checked for damage. Remove the map from your library instead |  |
| src-tauri/src/maps/manager.rs:717 | The original downloaded map contents changed; no deletion was authorized |  |
| src-tauri/src/maps/manager.rs:720 | The original downloaded map file changed while preparing cleanup |  |
| src-tauri/src/maps/manager.rs:729 | Map download changed while preparing cleanup; no deletion was authorized |  |
| src-tauri/src/maps/manager.rs:740 | Pause the map before deleting its downloaded files |  |
| src-tauri/src/maps/manager.rs:776 | This older map has no owned publication stamp; remove the installed archive with its protected file action first |  |
| src-tauri/src/maps/manager.rs:785 | Map recovery warnings remain unresolved; this download stays visible and no unproven files were deleted |  |
| src-tauri/src/maps/manager.rs:799 | Unknown map job |  |
| src-tauri/src/maps/manager.rs:801 | Delete the retained map files instead of only removing them from the list |  |
| src-tauri/src/maps/manager.rs:804 | Only finished map downloads can be removed from history |  |
| src-tauri/src/maps/manager.rs:807 | map:{} |  |
| src-tauri/src/maps/manager.rs:808 | Resume or delete retained map data first. |  |
| src-tauri/src/maps/manager.rs:834 | This map recovery entry is no longer in the current download. Refresh and try again |  |
| src-tauri/src/maps/manager.rs:850 | Cancel the active map before clearing downloaded parts |  |
| src-tauri/src/maps/manager.rs:880 | Missing map job |  |
| src-tauri/src/maps/manager.rs:896 | Map prefix checkpoint has no owned output |  |
| src-tauri/src/maps/manager.rs:1053 | Selected map region is not in the catalog |  |
| src-tauri/src/maps/manager.rs:1058 | Selected map language is not offered |  |
| src-tauri/src/maps/manager.rs:1070 | Recipe does not match the selected catalog version |  |
| src-tauri/src/maps/manager.rs:1076 | Map version changed; select the map again before downloading |  |
| src-tauri/src/maps/manager.rs:1125 | Map recipe or source version changed; the retained prefix was not modified |  |
| src-tauri/src/maps/manager.rs:1131 | Map checkpoint lacks its verified base identity |  |
| src-tauri/src/maps/manager.rs:1147 | Missing pack URL |  |
| src-tauri/src/maps/manager.rs:1193 | Map checkpoint destination changed; no bytes were modified |  |
| src-tauri/src/maps/manager.rs:1260 | This map download does not match the selected map. Delete it and download the map again |  |
| src-tauri/src/maps/manager.rs:1263 | composing |  |
| src-tauri/src/maps/manager.rs:1267 | md5 |  |
| src-tauri/src/maps/manager.rs:1267 | Missing segment length |  |
| src-tauri/src/maps/manager.rs:1275 | Missing inline data |  |
| src-tauri/src/maps/manager.rs:1278 | Map write failed: {error} |  |
| src-tauri/src/maps/manager.rs:1289 | Composed whole-file MD5 mismatch |  |
| src-tauri/src/maps/manager.rs:1335 | Composed file size mismatch |  |
| src-tauri/src/maps/manager.rs:1336 | Cannot sync composed map: {error} |  |
| src-tauri/src/maps/manager.rs:1342 | Map writer stopped unexpectedly; its last durable checkpoint was retained |  |
| src-tauri/src/maps/manager.rs:1430 | Published map has no filename |  |
| src-tauri/src/maps/manager.rs:1435 | Published map path changed identity; verified inode {:?} in directory {:?} is retained for recovery; no output was removed |  |
| src-tauri/src/maps/manager.rs:1461 | Cannot publish verified map without overwriting: {e} |  |
| src-tauri/src/maps/manager.rs:1466 | No available map filename |  |
| src-tauri/src/maps/partials.rs:127 | Map destination directory changed identity |  |
| src-tauri/src/maps/partials.rs:144 | Map checkpoint has no resumable private output |  |
| src-tauri/src/maps/partials.rs:148 | Map checkpoint directory changed; no bytes were modified |  |
| src-tauri/src/maps/partials.rs:156 | Map checkpoint file changed identity or length; no bytes were modified |  |
| src-tauri/src/maps/partials.rs:189 | Saved map prefix failed its checksum; delete the retained download or get the map again. No bytes were modified |  |
| src-tauri/src/maps/partials.rs:202 | Map checkpoint moved or changed identity during verification; no bytes were modified |  |
| src-tauri/src/maps/partials.rs:229 | Map file length changed during ownership verification |  |
| src-tauri/src/maps/partials.rs:241 | Map publication checkpoint has an invalid filename |  |
| src-tauri/src/maps/partials.rs:245 | Map publication directory changed; retained entries were not modified |  |
| src-tauri/src/maps/partials.rs:257 | Published map changed identity during recovery; no bytes were modified |  |
| src-tauri/src/maps/partials.rs:311 | This partial map download has an unexpected name, so Kiwix left it alone. Delete the map download, then start it again |  |
| src-tauri/src/maps/partials.rs:316 | This partial map download was moved or changed, so Kiwix kept it. Delete the map download, then start it again |  |
| src-tauri/src/maps/partials.rs:342 | Invalid saved map publication filename; nothing was removed |  |
| src-tauri/src/maps/partials.rs:363 | Recovered map output changed identity; entry was preserved |  |
| src-tauri/src/maps/partials.rs:381 | Map publication remains unresolved at {} (directory {:?}, file {:?}, size {}); all entries were preserved |  |
| src-tauri/src/maps/partials.rs:432 | Missing map cleanup capability |  |
| src-tauri/src/maps/partials.rs:441 | Invalid map cleanup capability name |  |
| src-tauri/src/maps/partials.rs:451 | Map cleanup capability disappeared; remaining entries were preserved |  |
| src-tauri/src/maps/partials.rs:458 | Map cleanup capability was replaced; nothing was removed |  |
| src-tauri/src/maps/partials.rs:487 | Captured map partial changed identity or size; entry was preserved |  |
| src-tauri/src/maps/partials.rs:495 | Captured map contents changed; no bytes were deleted |  |
| src-tauri/src/maps/partials.rs:533 | {error}. Retained entries are recoverable at {} (directory {:?}). {restoration} |  |
| src-tauri/src/maps/partials.rs:552 | Published map directory changed; no bytes were deleted |  |
| src-tauri/src/maps/partials.rs:559 | Published map file changed; no bytes were deleted |  |
| src-tauri/src/maps/partials.rs:567 | This older map cannot be checked for damage. Remove the map from your library, then download it again |  |
| src-tauri/src/maps/partials.rs:573 | Published map ownership changed; no bytes were deleted |  |
| src-tauri/src/maps/recipe.rs:108 | Mask offset overflow |  |
| src-tauri/src/maps/recipe.rs:135 | StreetZim URLs must use HTTPS without credentials or fragments |  |
| src-tauri/src/maps/recipe.rs:154 | Unsupported StreetZim catalog format |  |
| src-tauri/src/maps/recipe.rs:165 | Invalid or duplicate StreetZim region |  |
| src-tauri/src/maps/recipe.rs:181 | Invalid or duplicate StreetZim language version |  |
| src-tauri/src/maps/recipe.rs:200 | This map download is damaged. Start the download again |  |
| src-tauri/src/maps/recipe.rs:205 | Invalid source SHA-256 |  |
| src-tauri/src/maps/recipe.rs:212 | Invalid source mask bounds |  |
| src-tauri/src/maps/recipe.rs:247 | pack: |  |
| src-tauri/src/maps/recipe.rs:247 | Invalid source key |  |
| src-tauri/src/maps/recipe.rs:248 | Unknown source pack |  |
| src-tauri/src/maps/recipe.rs:270 | Invalid StreetZim recipe or output filename |  |
| src-tauri/src/maps/recipe.rs:286 | Invalid inline base64 |  |
| src-tauri/src/maps/recipe.rs:291 | Inline data SHA-256 or length mismatch |  |
| src-tauri/src/maps/recipe.rs:304 | Missing source masks |  |
| src-tauri/src/maps/recipe.rs:307 | Source segment exceeds source or copies masked bytes |  |
| src-tauri/src/maps/recipe.rs:311 | Unknown segment or non-final MD5 segment |  |
| src-tauri/src/maps/recipe.rs:314 | Recipe segment size overflow |  |
| src-tauri/src/maps/recipe.rs:319 | Recipe output size mismatch |  |
| src-tauri/src/maps/recipe.rs:344 | pack |  |
| src-tauri/src/maps/recipe.rs:344 | Missing pack key |  |
| src-tauri/src/maps/recipe.rs:352 | Map recipe is too complex to estimate download size safely |  |
| src-tauri/src/maps/recipe.rs:375 | Map transfer estimate overflow |  |
| src-tauri/src/maps/recipe.rs:422 | Missing pack ID |  |
| src-tauri/src/maps/recipe.rs:424 | Segment is not a source range |  |
| src-tauri/src/maps.rs:186 | Map settings changed; select the map again before downloading or replacing it |  |
| src-tauri/src/maps.rs:200 | The installed map changed; select the map again before replacing it |  |
| src-tauri/src/maps.rs:212 | Verified map does not match the authorized recipe |  |
| src-tauri/src/maps.rs:220 | Map installation changed |  |
| src-tauri/src/maps.rs:618 | Catalog worker failed: {error} |  |
| src-tauri/src/maps.rs:658 | Catalog worker failed during retirement: {error} |  |
| src-tauri/src/maps.rs:762 | Evaluation recipe output size differs from bounded manifest |  |
| src-tauri/src/maps.rs:766 | Evaluation source size overflow |  |
| src-tauri/src/maps.rs:769 | Evaluation source budget overflow |  |
| src-tauri/src/maps.rs:771 | Evaluation staged sources exceed bounded workload budget |  |
| src-tauri/src/maps.rs:791 | Missing staged source {} |  |
| src-tauri/src/maps.rs:794 | Staged source size mismatch |  |
| src-tauri/src/maps.rs:810 | Missing staged mask |  |
| src-tauri/src/maps.rs:813 | Staged source SHA-256 mismatch |  |
| src-tauri/src/menus.rs:217 | Not an application window |  |
| src-tauri/src/menus.rs:248 | Invalid menu label |  |
| src-tauri/src/menus.rs:256 | Invalid tab selection labels |  |
| src-tauri/src/menus.rs:268 | Invalid reader menu labels |  |
| src-tauri/src/menus.rs:275 | Missing reader menu label |  |
| src-tauri/src/menus.rs:281 | Library service unavailable during menu localization |  |
| src-tauri/src/menus.rs:284 | && |  |
| src-tauri/src/migration.rs:41 | Import exceeds the 16 MiB limit; split the export into smaller files |  |
| src-tauri/src/migration.rs:45 | Import is empty |  |
| src-tauri/src/migration.rs:79 | Unsupported bookmark export version {} (expected {BOOKMARK_EXPORT_VERSION}) |  |
| src-tauri/src/migration.rs:153 | Bookmark URLs must be archive-local, not {scheme:?} |  |
| src-tauri/src/migration.rs:159 | A saved bookmark has an invalid link and was skipped |  |
| src-tauri/src/migration.rs:165 | Bookmark book ID does not match its archive URL |  |
| src-tauri/src/migration.rs:192 | Expected a libkiwix <bookmarks>, <library>, or OPDS <feed> document |  |
| src-tauri/src/migration.rs:206 | This library has no embedded bookmarks; import its separate library.bookmarks.xml file |  |
| src-tauri/src/migration.rs:210 | book |  |
| src-tauri/src/migration.rs:210 | Bookmark is missing its <book> descriptor |  |
| src-tauri/src/migration.rs:219 | Bookmark is missing its <url> |  |
| src-tauri/src/migration.rs:221 | Bookmark is missing its <title> |  |
| src-tauri/src/migration.rs:223 | tags |  |
| src-tauri/src/migration.rs:238 | Malformed or duplicate highlights extension |  |
| src-tauri/src/migration.rs:280 | Bookmark contains characters XML 1.0 cannot represent; use JSON export instead |  |
| src-tauri/src/migration.rs:382 | Library entry for {path:?} has no archive ID |  |
| src-tauri/src/migration.rs:385 | Two library entries describe the same archive. Remove one before importing bookmarks |  |
| src-tauri/src/migration.rs:408 | Legacy library size overflows bytes |  |
| src-tauri/src/migration.rs:484 | Expected a legacy <library> XML document or an OPDS <feed> |  |
| src-tauri/src/migration.rs:533 | Legacy archive path contains a NUL byte |  |
| src-tauri/src/migration.rs:550 | Legacy file URL must not contain a query or fragment |  |
| src-tauri/src/migration.rs:553 | Legacy file URL cannot be represented as a local path on this platform |  |
| src-tauri/src/mobile.rs:220 | lifecycle |  |
| src-tauri/src/mobile.rs:242 | foreground-retirement |  |
| src-tauri/src/mobile.rs:265 | activity-window |  |
| src-tauri/src/mobile.rs:275 | Application handle unavailable |  |
| src-tauri/src/mobile.rs:276 | activityState |  |
| src-tauri/src/mobile.rs:278 | windowId |  |
| src-tauri/src/mobile.rs:279 | visible |  |
| src-tauri/src/mobile.rs:279 | Native Activity visibility missing |  |
| src-tauri/src/mobile.rs:280 | foreground |  |
| src-tauri/src/mobile.rs:280 | Native Activity foreground state missing |  |
| src-tauri/src/mobile.rs:281 | Android window binding unavailable |  |
| src-tauri/src/mobile.rs:286 | Android window incarnation missing |  |
| src-tauri/src/mobile.rs:302 | Android Activity identity missing |  |
| src-tauri/src/mobile.rs:348 | Android Activity changed before window construction |  |
| src-tauri/src/mobile.rs:351 | Canonical main window configuration missing |  |
| src-tauri/src/mobile.rs:362 | retireWindow |  |
| src-tauri/src/mobile.rs:373 | batches |  |
| src-tauri/src/mobile.rs:373 | Native incoming document queue is invalid |  |
| src-tauri/src/mobile.rs:375 | requestId |  |
| src-tauri/src/mobile.rs:375 | Native incoming batch has no private ID |  |
| src-tauri/src/mobile.rs:376 | uris |  |
| src-tauri/src/mobile.rs:376 | Native incoming batch has no document identifiers |  |
| src-tauri/src/mobile.rs:391 | document-progress |  |
| src-tauri/src/mobile.rs:391 | camera-revoked |  |
| src-tauri/src/mobile.rs:391 | checkpoint-expired |  |
| src-tauri/src/mobile.rs:391 | checkpoint-retiring |  |
| src-tauri/src/mobile.rs:391 | incoming-documents |  |
| src-tauri/src/mobile.rs:391 | app-entry |  |
| src-tauri/src/mobile.rs:391 | completion-notification-tap |  |
| src-tauri/src/mobile.rs:391 | mobile-inactive |  |
| src-tauri/src/mobile.rs:391 | mobile-suspended |  |
| src-tauri/src/mobile.rs:391 | mobile-resumed |  |
| src-tauri/src/mobile.rs:391 | transfers-pause |  |
| src-tauri/src/mobile.rs:391 | transfers-timeout |  |
| src-tauri/src/mobile.rs:391 | transfers-error |  |
| src-tauri/src/mobile.rs:391 | hardware-shortcut |  |
| src-tauri/src/mobile.rs:391 | network-policy-changed |  |
| src-tauri/src/mobile.rs:391 | speech-state |  |
| src-tauri/src/mobile.rs:391 | speech-action |  |
| src-tauri/src/mobile.rs:391 | reader-audio |  |
| src-tauri/src/mobile.rs:391 | dynamic-type |  |
| src-tauri/src/mobile.rs:391 | hotspot-stopped |  |
| src-tauri/src/mobile.rs:391 | nearby-bluetooth |  |
| src-tauri/src/mobile.rs:391 | nearby-join |  |
| src-tauri/src/mobile.rs:419 | hotspot |  |
| src-tauri/src/mobile.rs:431 | network-policy |  |
| src-tauri/src/mobile.rs:435 | network-resume |  |
| src-tauri/src/mobile.rs:489 | incoming |  |
| src-tauri/src/mobile.rs:533 | local-transfers |  |
| src-tauri/src/mobile.rs:539 | live-activity |  |
| src-tauri/src/mobile.rs:544 | resume-interrupted |  |
| src-tauri/src/mobile.rs:545 | native-transfer-events |  |
| src-tauri/src/mobile.rs:547 | map_status |  |
| src-tauri/src/mobile.rs:555 | transfers |  |
| src-tauri/src/mobile.rs:619 | Kiwix has already started; the startup actions are no longer available |  |
| src-tauri/src/mobile.rs:621 | quit |  |
| src-tauri/src/mobile.rs:626 | The application storage location is unavailable |  |
| src-tauri/src/mobile.rs:633 | Unknown startup action |  |
| src-tauri/src/mobile.rs:683 | Kiwix could not find its application storage. {error} |  |
| src-tauri/src/mobile.rs:710 | Initial Android Activity identity missing |  |
| src-tauri/src/mobile.rs:718 | Canonical Android main window configuration missing |  |
| src-tauri/src/mobile.rs:729 | native-listeners |  |
| src-tauri/src/mobile.rs:734 | background-download-recovery |  |
| src-tauri/src/mobile.rs:802 | releaseIncomingOriginals |  |
| src-tauri/src/mobile.rs:805 | Releasing rejected incoming document access failed: {message} |  |
| src-tauri/src/mobile.rs:817 | deep-link |  |
| src-tauri/src/mobile.rs:831 | finish-activity |  |
| src-tauri/src/mobile.rs:834 | finishActivity |  |
| src-tauri/src/mobile_backup.rs:19 | Android archive backup is excluded by static platform rules |  |
| src-tauri/src/mobile_backup.rs:58 | iOS could not conditionally exclude the owned payload |  |
| src-tauri/src/mobile_backup.rs:84 | iOS could not inspect owned archive admission |  |
| src-tauri/src/mobile_backup.rs:104 | iOS could not apply the owned archive backup policy |  |
| src-tauri/src/mobile_policy.rs:229 | Unsupported native app entry kind |  |
| src-tauri/src/mobile_policy.rs:231 | Native app entry value is too large |  |
| src-tauri/src/mobile_policy.rs:232 | sharing |  |
| src-tauri/src/mobile_policy.rs:232 | voice-search |  |
| src-tauri/src/mobile_policy.rs:232 | bookmarks |  |
| src-tauri/src/mobile_policy.rs:233 | This native app entry does not accept a value |  |
| src-tauri/src/mobile_policy.rs:307 | MOBILE_LOCAL_NETWORK_BLOCKED |  |
| src-tauri/src/mobile_policy.rs:372 | Turn on Wi-Fi and connect both devices to the same network, or create a hotspot. Internet access is not required. |  |
| src-tauri/src/mobile_policy.rs:374 | Waiting for network status. Try again when the connection is ready. |  |
| src-tauri/src/mobile_policy.rs:392 | Network changed during transfer admission; start or resume again |  |
| src-tauri/src/mobile_policy.rs:432 | Torrent manager unavailable |  |
| src-tauri/src/mobile_policy.rs:436 | Map manager unavailable |  |
| src-tauri/src/mobile_rebase.rs:100 | Consumed acquisition receipt is not a string |  |
| src-tauri/src/mobile_rebase.rs:123 | jobs |  |
| src-tauri/src/mobile_rebase.rs:123 | Torrent state has no jobs array |  |
| src-tauri/src/mobile_rebase.rs:127 | job |  |
| src-tauri/src/mobile_rebase.rs:127 | Saved transfer history is damaged. Restore a backup |  |
| src-tauri/src/mobile_rebase.rs:128 | Torrent job has no ID |  |
| src-tauri/src/mobile_rebase.rs:129 | torrent: |  |
| src-tauri/src/mobile_rebase.rs:129 | Invalid torrent receipt job ID |  |
| src-tauri/src/mobile_rebase.rs:133 | completedPaths |  |
| src-tauri/src/mobile_rebase.rs:133 | Torrent job has no completed paths array |  |
| src-tauri/src/mobile_rebase.rs:135 | Torrent completed path is not a string |  |
| src-tauri/src/mobile_rebase.rs:142 | Torrent completed path is not UTF-8 |  |
| src-tauri/src/mobile_rebase.rs:143 | {id}:{} |  |
| src-tauri/src/mobile_rebase.rs:147 | Torrent receipt source path is not UTF-8 |  |
| src-tauri/src/mobile_rebase.rs:151 | Conflicting torrent receipt path correlations |  |
| src-tauri/src/mobile_rebase.rs:162 | Persisted filesystem path is not a string |  |
| src-tauri/src/mobile_rebase.rs:173 | Mobile path is not UTF-8 |  |
| src-tauri/src/mobile_rebase.rs:180 | Persisted relocation collection is not an array |  |
| src-tauri/src/mobile_rebase.rs:183 | Mobile filesystem path is not absolute: {} |  |
| src-tauri/src/mobile_rebase.rs:188 | Mobile filesystem path contains traversal: {} |  |
| src-tauri/src/mobile_rebase.rs:192 | Mobile filesystem path is not normalized: {} |  |
| src-tauri/src/mobile_rebase.rs:241 | Relocation directory is not a real directory: {name} |  |
| src-tauri/src/mobile_rebase.rs:247 | The storage folder changed while Kiwix was moving files. Restart the app |  |
| src-tauri/src/mobile_rebase.rs:266 | A saved file is too large to move. Restart the app |  |
| src-tauri/src/mobile_rebase.rs:272 | A saved file changed while Kiwix was moving it. Restart the app |  |
| src-tauri/src/mobile_rebase.rs:301 | Relocation publication identity changed |  |
| src-tauri/src/mobile_rebase.rs:315 | Relocation capability must be owned and not writable by other users |  |
| src-tauri/src/mobile_rebase.rs:328 | Mobile root capability must be owned and private (0700) |  |
| src-tauri/src/mobile_rebase.rs:337 | Relocation state must be a regular file without hard links |  |
| src-tauri/src/mobile_rebase.rs:405 | Kiwix: mobile root marker is unreadable: {error} |  |
| src-tauri/src/mobile_rebase.rs:442 | pending.json |  |
| src-tauri/src/mobile_rebase.rs:529 | Non-UTF-8 migration journal name |  |
| src-tauri/src/mobile_rebase.rs:530 | .json |  |
| src-tauri/src/mobile_rebase.rs:555 | Mobile relocation intent exceeds its size bound |  |
| src-tauri/src/mobile_rebase.rs:581 | Created mobile root capability disappeared |  |
| src-tauri/src/mobile_rebase.rs:675 | Recovery directory has no identity evidence |  |
| src-tauri/src/mobile_rebase.rs:680 | Invalid recovery directory component |  |
| src-tauri/src/mobile_rebase.rs:681 | Non-UTF-8 recovery directory |  |
| src-tauri/src/mobile_rebase.rs:685 | Relocated recovery directory changed identity: {} |  |
| src-tauri/src/mobile_rebase.rs:794 | Invalid relocated path component |  |
| src-tauri/src/mobile_rebase.rs:800 | Relocated path crosses a symlink: {} |  |
| src-tauri/src/mobile_rebase.rs:802 | Non-UTF-8 relocated path component |  |
| src-tauri/src/mobile_rebase.rs:803 | Relocated directory disappeared |  |
| src-tauri/src/mobile_rebase.rs:805 | Relocated path crosses a non-directory |  |
| src-tauri/src/mobile_service.rs:42 | This pinned article link is too long to open |  |
| src-tauri/src/mobile_service.rs:54 | Mobile native adapter is unavailable |  |
| src-tauri/src/mobile_service.rs:55 | crashCapturePolicy |  |
| src-tauri/src/mobile_service.rs:55 | enabled |  |
| src-tauri/src/mobile_service.rs:66 | Kiwix could not finish a previous library storage move. {error} |  |
| src-tauri/src/mobile_service.rs:94 | The saved library location was not recognized, so Kiwix uses internal app storage. Existing files were kept. |  |
| src-tauri/src/mobile_service.rs:99 | The selected library storage is unavailable. Reconnect it or choose another location in Settings. |  |
| src-tauri/src/mobile_service.rs:138 | Mobile native actions require the trusted root window |  |
| src-tauri/src/mobile_service.rs:145 | Use the native mobile document import/export action |  |
| src-tauri/src/mobile_service.rs:148 | scan_folder |  |
| src-tauri/src/mobile_service.rs:164 | OS Trash is not supported on this mobile platform |  |
| src-tauri/src/mobile_service.rs:166 | downloadDirectory |  |
| src-tauri/src/mobile_service.rs:171 | Invalid monitored mobile archive folder |  |
| src-tauri/src/mobile_service.rs:183 | Invalid mobile background request identity |  |
| src-tauri/src/mobile_service.rs:228 | Kiwix mobile admission rejected {command}: active={} desired_active={} inactive={} background={} |  |
| src-tauri/src/mobile_service.rs:245 | Cancel |  |
| src-tauri/src/mobile_service.rs:245 | copying |  |
| src-tauri/src/mobile_service.rs:245 | Copying documents… |  |
| src-tauri/src/mobile_service.rs:270 | Mobile retirement revision exhausted |  |
| src-tauri/src/mobile_service.rs:294 | audible |  |
| src-tauri/src/mobile_service.rs:294 | Invalid reader media state |  |
| src-tauri/src/mobile_service.rs:295 | pictureInPicture |  |
| src-tauri/src/mobile_service.rs:296 | release |  |
| src-tauri/src/mobile_service.rs:303 | readerAudioState |  |
| src-tauri/src/mobile_service.rs:303 | active |  |
| src-tauri/src/mobile_service.rs:341 | media-reader-retirement |  |
| src-tauri/src/mobile_service.rs:344 | media-retirement |  |
| src-tauri/src/mobile_service.rs:349 | media-spelling-retirement |  |
| src-tauri/src/mobile_service.rs:357 | exportFile |  |
| src-tauri/src/mobile_service.rs:357 | mimeType |  |
| src-tauri/src/mobile_service.rs:357 | suggestedName |  |
| src-tauri/src/mobile_service.rs:357 | open |  |
| src-tauri/src/mobile_service.rs:357 | share |  |
| src-tauri/src/mobile_service.rs:357 | labels |  |
| src-tauri/src/mobile_service.rs:358 | {error}. Prepared export retained at {} |  |
| src-tauri/src/mobile_service.rs:360 | Native export did not return a completion state; prepared export retained at {} |  |
| src-tauri/src/mobile_service.rs:362 | Native export did not return a completion state |  |
| src-tauri/src/mobile_service.rs:380 | Native incoming batch ID must be canonical |  |
| src-tauri/src/mobile_service.rs:388 | entries |  |
| src-tauri/src/mobile_service.rs:388 | Native app entry queue is invalid |  |
| src-tauri/src/mobile_service.rs:404 | library-notice |  |
| src-tauri/src/mobile_service.rs:418 | Native app entry ID must be canonical |  |
| src-tauri/src/mobile_service.rs:424 | Native app entry changed its immutable payload |  |
| src-tauri/src/mobile_service.rs:427 | Handle pending app entry requests before opening more |  |
| src-tauri/src/mobile_service.rs:430 | kiwix-mobile-entry |  |
| src-tauri/src/mobile_service.rs:430 | kind |  |
| src-tauri/src/mobile_service.rs:454 | Too many incoming documents or an oversized document identifier |  |
| src-tauri/src/mobile_service.rs:456 | Invalid incoming document identifier |  |
| src-tauri/src/mobile_service.rs:457 | content |  |
| src-tauri/src/mobile_service.rs:458 | Incoming document scheme is not supported by this native provider |  |
| src-tauri/src/mobile_service.rs:465 | Native incoming batch changed its immutable source identifiers |  |
| src-tauri/src/mobile_service.rs:468 | Confirm or dismiss pending incoming archives before opening more documents |  |
| src-tauri/src/mobile_service.rs:471 | kiwix-mobile-incoming |  |
| src-tauri/src/mobile_service.rs:471 | count |  |
| src-tauri/src/mobile_service.rs:477 | Unknown incoming document request |  |
| src-tauri/src/mobile_service.rs:518 | Archive to reconnect is no longer in the library |  |
| src-tauri/src/mobile_service.rs:520 | Leave private or read-only mode before replacing a saved external archive reference |  |
| src-tauri/src/mobile_service.rs:524 | Invalid archive replacement identifier |  |
| src-tauri/src/mobile_service.rs:526 | folder |  |
| src-tauri/src/mobile_service.rs:527 | pickExternalArchive |  |
| src-tauri/src/mobile_service.rs:529 | Open external archive |  |
| src-tauri/src/mobile_service.rs:532 | Native picker did not return cancellation state |  |
| src-tauri/src/mobile_service.rs:537 | Native external archive selection missing |  |
| src-tauri/src/mobile_service.rs:548 | Close this file and open it again after changing privacy settings |  |
| src-tauri/src/mobile_service.rs:554 | Selected file is a different archive; open it separately |  |
| src-tauri/src/mobile_service.rs:569 | Archive changed while reconnecting |  |
| src-tauri/src/mobile_service.rs:603 | {error}; releasing external selection failed: {cleanup} |  |
| src-tauri/src/mobile_service.rs:651 | Add archive folder |  |
| src-tauri/src/mobile_service.rs:651 | Add files |  |
| src-tauri/src/mobile_service.rs:659 | Native original archive selections missing |  |
| src-tauri/src/mobile_service.rs:662 | Native original archive errors missing |  |
| src-tauri/src/mobile_service.rs:672 | No complete ZIM archives were found in the selected location |  |
| src-tauri/src/mobile_service.rs:674 | books |  |
| src-tauri/src/mobile_service.rs:684 | Unknown or already handled incoming document request |  |
| src-tauri/src/mobile_service.rs:687 | native-incoming:{id} |  |
| src-tauri/src/mobile_service.rs:699 | uri |  |
| src-tauri/src/mobile_service.rs:699 | persistent |  |
| src-tauri/src/mobile_service.rs:703 | selection |  |
| src-tauri/src/mobile_service.rs:703 | Native original selection missing |  |
| src-tauri/src/mobile_service.rs:759 | root |  |
| src-tauri/src/mobile_service.rs:815 | Copying or moving archives requires explicit confirmation |  |
| src-tauri/src/mobile_service.rs:817 | bookIds |  |
| src-tauri/src/mobile_service.rs:818 | target |  |
| src-tauri/src/mobile_service.rs:830 | Stop sharing the selected archive before copying or moving it |  |
| src-tauri/src/mobile_service.rs:856 | Device scanning is unavailable in private mode. Leave private mode to look for ZIM files on this device. |  |
| src-tauri/src/mobile_service.rs:858 | scanDeviceArchives |  |
| src-tauri/src/mobile_service.rs:858 | maxEntries |  |
| src-tauri/src/mobile_service.rs:870 | Device scanning is unavailable in private mode. Leave private mode to open a ZIM file from this device. |  |
| src-tauri/src/mobile_service.rs:872 | token |  |
| src-tauri/src/mobile_service.rs:880 | Native scanned archive selection missing |  |
| src-tauri/src/mobile_service.rs:926 | {error}; releasing scanned archive selection failed: {cleanup} |  |
| src-tauri/src/mobile_service.rs:937 | Folder access cannot be saved in private mode. Leave private mode to scan this device. |  |
| src-tauri/src/mobile_service.rs:939 | pickScanFolder |  |
| src-tauri/src/mobile_service.rs:944 | Native folder picker did not return cancellation state |  |
| src-tauri/src/mobile_service.rs:1027 | Incoming batch contains no complete archive |  |
| src-tauri/src/mobile_service.rs:1065 | Invalid platform locale preservation choice |  |
| src-tauri/src/mobile_service.rs:1067 | applyLocale |  |
| src-tauri/src/mobile_service.rs:1068 | locale |  |
| src-tauri/src/mobile_service.rs:1091 | mobile_background_ack |  |
| src-tauri/src/mobile_service.rs:1132 | Pinning articles is unavailable in private or read-only mode |  |
| src-tauri/src/mobile_service.rs:1134 | bookId |  |
| src-tauri/src/mobile_service.rs:1138 | This article location cannot be pinned |  |
| src-tauri/src/mobile_service.rs:1140 | pinArticleShortcut |  |
| src-tauri/src/mobile_service.rs:1161 | pickLibraryDirectoryCopy |  |
| src-tauri/src/mobile_service.rs:1161 | pickArchiveCopies |  |
| src-tauri/src/mobile_service.rs:1161 | destinationDirectory |  |
| src-tauri/src/mobile_service.rs:1161 | Import a folder copy |  |
| src-tauri/src/mobile_service.rs:1161 | Import a managed copy |  |
| src-tauri/src/mobile_service.rs:1163 | paths |  |
| src-tauri/src/mobile_service.rs:1167 | Native directory picker returned an unexpected owned directory |  |
| src-tauri/src/mobile_service.rs:1172 | readTextDocument |  |
| src-tauri/src/mobile_service.rs:1172 | maxBytes |  |
| src-tauri/src/mobile_service.rs:1172 | Import document |  |
| src-tauri/src/mobile_service.rs:1173 | Native reader did not return cancellation state |  |
| src-tauri/src/mobile_service.rs:1174 | text |  |
| src-tauri/src/mobile_service.rs:1175 | Import document exceeds 16 MiB |  |
| src-tauri/src/mobile_service.rs:1178 | mobile_save_text |  |
| src-tauri/src/mobile_service.rs:1178 | mobile_save_bytes |  |
| src-tauri/src/mobile_service.rs:1182 | Text export exceeds 16 MiB |  |
| src-tauri/src/mobile_service.rs:1183 | .xml |  |
| src-tauri/src/mobile_service.rs:1183 | application/xml |  |
| src-tauri/src/mobile_service.rs:1183 | text/plain |  |
| src-tauri/src/mobile_service.rs:1185 | bytes |  |
| src-tauri/src/mobile_service.rs:1186 | Byte export exceeds 16 MiB; use streaming resource export |  |
| src-tauri/src/mobile_service.rs:1188 | Byte export exceeds 16 MiB |  |
| src-tauri/src/mobile_service.rs:1189 | mime |  |
| src-tauri/src/mobile_service.rs:1190 | Invalid export MIME type |  |
| src-tauri/src/mobile_service.rs:1224 | Unknown or already imported incoming document request |  |
| src-tauri/src/mobile_service.rs:1234 | copyIncomingArchives |  |
| src-tauri/src/mobile_service.rs:1235 | Native import did not return cancellation state |  |
| src-tauri/src/mobile_service.rs:1246 | Unknown mobile command: {command} |  |
| src-tauri/src/mobile_service.rs:1290 | map-checkpoint |  |
| src-tauri/src/mobile_service.rs:1405 | Transfer retirement worker stopped before completion |  |
| src-tauri/src/mobile_service.rs:1425 | Transfer retirement is still pending; foreground admission remains closed and will retry when its owner completes |  |
| src-tauri/src/mobile_service.rs:1432 | Lifecycle retirement is still pending; foreground activation will retry |  |
| src-tauri/src/mobile_service.rs:1492 | Background execution time ended; transfer retirement remains pending until execution resumes |  |
| src-tauri/src/mobile_service.rs:1586 | Native background checkpoint unavailable: {error} |  |
| src-tauri/src/mobile_service.rs:1601 | The mobile background checkpoint was not issued by its lifecycle transition |  |
| src-tauri/src/mobile_service.rs:1619 | endCheckpoint |  |
| src-tauri/src/mobile_service.rs:1655 | Native background checkpoint state unavailable: {error} |  |
| src-tauri/src/mobile_service.rs:1693 | Reader storage did not acknowledge the finite background checkpoint deadline |  |
| src-tauri/src/mobile_service.rs:1696 | The mobile background checkpoint ended without a durability acknowledgement |  |
| src-tauri/src/mobile_service.rs:1756 | discoveryState |  |
| src-tauri/src/mobile_service.rs:1801 | Archive cache unavailable |  |
| src-tauri/src/mobile_service.rs:1811 | local-transfer-retirement |  |
| src-tauri/src/mobile_service.rs:1822 | start_download |  |
| src-tauri/src/mobile_service.rs:1822 | edition_download |  |
| src-tauri/src/mobile_service.rs:1822 | resume_download |  |
| src-tauri/src/mobile_service.rs:1822 | share_torrent |  |
| src-tauri/src/mobile_service.rs:1822 | localswarm_download |  |
| src-tauri/src/mobile_service.rs:1822 | localswarm_seed |  |
| src-tauri/src/mobile_service.rs:1822 | localswarm_share_all |  |
| src-tauri/src/mobile_service.rs:1822 | map_start |  |
| src-tauri/src/mobile_service.rs:1822 | map_resume |  |
| src-tauri/src/mobile_service.rs:1822 | sharing_start |  |
| src-tauri/src/mobile_service.rs:1822 | shared_library_download |  |
| src-tauri/src/mobile_service.rs:1822 | torrent_seeding |  |
| src-tauri/src/mobile_service.rs:1827 | Transfers |  |
| src-tauri/src/mobile_service.rs:1827 | body |  |
| src-tauri/src/mobile_service.rs:1827 | Transferring archives |  |
| src-tauri/src/mobile_service.rs:1827 | pauseLabel |  |
| src-tauri/src/mobile_service.rs:1827 | Pause |  |
| src-tauri/src/mobile_service.rs:1827 | channelName |  |
| src-tauri/src/mobile_service.rs:1827 | Archive transfers |  |
| src-tauri/src/mobile_service.rs:1827 | progress |  |
| src-tauri/src/mobile_service.rs:1827 | total |  |
| src-tauri/src/mobile_service.rs:1874 | events |  |
| src-tauri/src/mobile_service.rs:1874 | Native pending transfer event queue is invalid |  |
| src-tauri/src/mobile_service.rs:1884 | Unknown native transfer control event |  |
| src-tauri/src/mobile_service.rs:1886 | Invalid native transfer event ID |  |
| src-tauri/src/mobile_service.rs:1886 | Native transfer event ID is not canonical |  |
| src-tauri/src/mobile_service.rs:1924 | transferState |  |
| src-tauri/src/mobile_service.rs:1927 | VISIBLE_ACTIVITY_REQUIRED |  |
| src-tauri/src/mobile_service.rs:1961 | A native Pause or timeout must be durably reconciled before starting transfers |  |
| src-tauri/src/mobile_service.rs:1963 | Start or resume transfers while the application is visible |  |
| src-tauri/src/mobile_service.rs:1967 | Transfer authorization was superseded by a lifecycle or native Pause action; resume after reconciliation |  |
| src-tauri/src/mobile_service.rs:2039 | The temporary Wi-Fi permission dialog was not completed. Dismiss it and try again. |  |
| src-tauri/src/mobile_service.rs:2053 | Creating the Wi-Fi hotspot failed: {error} |  |
| src-tauri/src/mobile_service.rs:2078 | {error}. {stop} |  |
| src-tauri/src/mobile_service.rs:2085 | The Wi-Fi hotspot start was stopped before it completed |  |
| src-tauri/src/mobile_service.rs:2095 | The Wi-Fi hotspot start finished after the app left the foreground |  |
| src-tauri/src/mobile_service.rs:2121 | hotspotStop |  |
| src-tauri/src/mobile_service.rs:2122 | Hotspot state unavailable |  |
| src-tauri/src/mobile_service.rs:2131 | Stopping the Wi-Fi hotspot failed: {error} |  |
| src-tauri/src/mobile_service.rs:2156 | Start the Wi-Fi hotspot before sharing over it |  |
| src-tauri/src/mobile_service.rs:2211 | completed |  |
| src-tauri/src/mobile_speech.rs:71 | SPEECH_SOURCE_RETIRING |  |
| src-tauri/src/mobile_speech.rs:77 | SPEECH_SESSION_RETIRED |  |
| src-tauri/src/mobile_speech.rs:94 | SPEECH_BUSY |  |
| src-tauri/src/mobile_speech.rs:96 | SPEECH_SESSION_REGISTERED |  |
| src-tauri/src/mobile_speech.rs:98 | SPEECH_SEQUENCE_EXHAUSTED |  |
| src-tauri/src/mobile_speech.rs:127 | SPEECH_SESSION_CONSUMED |  |
| src-tauri/src/mobile_speech.rs:128 | SPEECH_SESSION_SUPERSEDED |  |
| src-tauri/src/mobile_speech.rs:195 | Mobile native speech adapter is unavailable |  |
| src-tauri/src/mobile_speech.rs:201 | speech-retire |  |
| src-tauri/src/mobile_speech.rs:207 | kiwix-mobile-error |  |
| src-tauri/src/mobile_speech.rs:207 | operation |  |
| src-tauri/src/mobile_speech.rs:239 | Native speech requires the trusted root window |  |
| src-tauri/src/mobile_speech.rs:241 | SPEECH_FOREGROUND_REQUIRED |  |
| src-tauri/src/mobile_speech.rs:242 | Browsing session expired after a privacy transition |  |
| src-tauri/src/mobile_speech.rs:244 | SPEECH_ARCHIVE_INSTALLATION_RETIRED |  |
| src-tauri/src/mobile_speech.rs:246 | SPEECH_READER_SESSION_RETIRED |  |
| src-tauri/src/mobile_speech.rs:267 | main:{} |  |
| src-tauri/src/mobile_speech.rs:269 | SPEECH_READER_LISTENER_RETIRED |  |
| src-tauri/src/mobile_speech.rs:295 | {}:{} |  |
| src-tauri/src/mobile_speech.rs:346 | sessionId |  |
| src-tauri/src/mobile_speech.rs:380 | mobile_speech_stop |  |
| src-tauri/src/mobile_speech.rs:380 | mobile_speech_retire |  |
| src-tauri/src/mobile_speech.rs:417 | mobile_speech_pause |  |
| src-tauri/src/mobile_speech.rs:417 | speechPause |  |
| src-tauri/src/mobile_speech.rs:428 | SPEECH_INVALID_REQUEST |  |
| src-tauri/src/mobile_speech.rs:429 | offset |  |
| src-tauri/src/mobile_speech.rs:430 | mobile_speech_rewind |  |
| src-tauri/src/mobile_speech.rs:430 | mobile_speech_forward |  |
| src-tauri/src/mobile_speech.rs:431 | seconds |  |
| src-tauri/src/mobile_speech.rs:441 | mobile_speech_restore_reader |  |
| src-tauri/src/mobile_speech.rs:449 | archiveId |  |
| src-tauri/src/mobile_speech.rs:455 | mobile_speech_voices |  |
| src-tauri/src/mobile_speech.rs:457 | speechVoices |  |
| src-tauri/src/mobile_speech.rs:463 | mobile_speech_prepare |  |
| src-tauri/src/mobile_speech.rs:463 | mobile_speech_start |  |
| src-tauri/src/mobile_speech.rs:463 | Unknown native speech command |  |
| src-tauri/src/mobile_speech.rs:484 | Native speech preparation returned no capability |  |
| src-tauri/src/mobile_speech.rs:527 | Invalid speech arguments |  |
| src-tauri/src/mobile_speech.rs:529 | lang |  |
| src-tauri/src/mobile_speech.rs:529 | voice |  |
| src-tauri/src/mobile_speech.rs:529 | rate |  |
| src-tauri/src/mobile_speech.rs:530 | Missing speech argument: {key} |  |
| src-tauri/src/mobile_speech.rs:536 | Reading aloud |  |
| src-tauri/src/mobile_speech.rs:537 | pause |  |
| src-tauri/src/mobile_speech.rs:559 | accepted |  |
| src-tauri/src/mobile_speech.rs:650 | speech-reader-retirement |  |
| src-tauri/src/mobile_speech.rs:653 | speech-retirement |  |
| src-tauri/src/mobile_speech.rs:658 | speech-spelling-retirement |  |
| src-tauri/src/native_pdf/android.rs:40 | Article PDF export was cancelled |  |
| src-tauri/src/native_pdf/android.rs:47 | Android did not open the native PDF print UI |  |
| src-tauri/src/native_pdf/android.rs:52 | Android did not render the live article PDF |  |
| src-tauri/src/native_pdf/android.rs:55 | Reading article PDF: {error} |  |
| src-tauri/src/native_pdf/android.rs:57 | Android article PDF is invalid or exceeds the 64 MiB limit |  |
| src-tauri/src/native_pdf/android.rs:62 | %PDF- |  |
| src-tauri/src/native_pdf/android.rs:62 | Android did not produce a PDF document |  |
| src-tauri/src/native_pdf/android.rs:68 | Android PDF sharing returned no completion state |  |
| src-tauri/src/native_pdf/android.rs:71 | Article PDF sharing was cancelled |  |
| src-tauri/src/native_pdf/apple.rs:38 | PDF output path is not UTF-8 |  |
| src-tauri/src/native_pdf/apple.rs:38 | Invalid PDF output path |  |
| src-tauri/src/native_pdf/apple.rs:55 | PDF print path is not UTF-8 |  |
| src-tauri/src/native_pdf/apple.rs:55 | Invalid PDF print path |  |
| src-tauri/src/native_pdf/apple.rs:56 | Invalid PDF print title |  |
| src-tauri/src/native_pdf/apple.rs:62 | The native print panel could not be opened |  |
| src-tauri/src/native_pdf/apple.rs:72 | Article PDF printing was cancelled |  |
| src-tauri/src/native_pdf/apple.rs:76 | The system print dialog timed out |  |
| src-tauri/src/native_pdf/cleanup.rs:83 | Could not remove private article PDF temporary files: {error} |  |
| src-tauri/src/native_pdf/linux.rs:48 | GTK's native PDF file-print backend is unavailable; no printer was selected |  |
| src-tauri/src/native_pdf/linux.rs:86 | Native PDF resource load failed: {error} |  |
| src-tauri/src/native_pdf/linux.rs:94 | Invalid native PDF scratch path |  |
| src-tauri/src/native_pdf/linux.rs:96 | output-uri |  |
| src-tauri/src/native_pdf/linux.rs:97 | output-file-format |  |
| src-tauri/src/native_pdf/linux.rs:97 | pdf |  |
| src-tauri/src/native_pdf/linux.rs:117 | Invalid print title |  |
| src-tauri/src/native_pdf/linux.rs:119 | GTK could not open the system print dialog |  |
| src-tauri/src/native_pdf/linux.rs:131 | The system print settings are unavailable |  |
| src-tauri/src/native_pdf/linux.rs:133 | No system printer was selected |  |
| src-tauri/src/native_pdf/linux.rs:136 | Invalid printer page dimensions |  |
| src-tauri/src/native_pdf/linux.rs:153 | Native GTK PDF provider stopped |  |
| src-tauri/src/native_pdf/live.rs:27 | The article capture is empty or exceeds the 16 MiB limit |  |
| src-tauri/src/native_pdf/live.rs:30 | The live article has unsupported capture dimensions |  |
| src-tauri/src/native_pdf/live.rs:33 | The live article exceeds the 512-page PDF limit |  |
| src-tauri/src/native_pdf/live.rs:35 | Invalid live article identity |  |
| src-tauri/src/native_pdf/live.rs:73 | browser-icon |  |
| src-tauri/src/native_pdf/live.rs:131 | The live article contains an uncaptured visual resource; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs:139 | This article has too many nested elements to export as a PDF |  |
| src-tauri/src/native_pdf/live.rs:147 | iframe |  |
| src-tauri/src/native_pdf/live.rs:147 | frame |  |
| src-tauri/src/native_pdf/live.rs:147 | frameset |  |
| src-tauri/src/native_pdf/live.rs:147 | object |  |
| src-tauri/src/native_pdf/live.rs:147 | embed |  |
| src-tauri/src/native_pdf/live.rs:147 | video |  |
| src-tauri/src/native_pdf/live.rs:147 | audio |  |
| src-tauri/src/native_pdf/live.rs:147 | canvas |  |
| src-tauri/src/native_pdf/live.rs:147 | The live article contains an uncaptured frame, media surface, or canvas; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs:148 | animate |  |
| src-tauri/src/native_pdf/live.rs:148 | animateMotion |  |
| src-tauri/src/native_pdf/live.rs:148 | animateTransform |  |
| src-tauri/src/native_pdf/live.rs:148 | set |  |
| src-tauri/src/native_pdf/live.rs:148 | discard |  |
| src-tauri/src/native_pdf/live.rs:148 | The live article contains an unfrozen SVG animation; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs:162 | The live article contains an unresolved responsive image; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs:164 | style |  |
| src-tauri/src/native_pdf/live.rs:164 | fill |  |
| src-tauri/src/native_pdf/live.rs:164 | stroke |  |
| src-tauri/src/native_pdf/live.rs:164 | filter |  |
| src-tauri/src/native_pdf/live.rs:164 | clip-path |  |
| src-tauri/src/native_pdf/live.rs:164 | mask |  |
| src-tauri/src/native_pdf/live.rs:164 | cursor |  |
| src-tauri/src/native_pdf/live.rs:164 | marker-start |  |
| src-tauri/src/native_pdf/live.rs:164 | marker-mid |  |
| src-tauri/src/native_pdf/live.rs:164 | marker-end |  |
| src-tauri/src/native_pdf/live.rs:165 | =\" |  |
| src-tauri/src/native_pdf/live.rs:179 | The live article contains a style element with child elements; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs:182 | </style> |  |
| src-tauri/src/native_pdf/windows.rs:71 | Missing PDF scratch directory |  |
| src-tauri/src/native_pdf/windows.rs:71 | webview |  |
| src-tauri/src/native_pdf/windows.rs:72 | about:blank |  |
| src-tauri/src/native_pdf/windows.rs:98 | The WebView2 runtime cannot create a private article PDF view |  |
| src-tauri/src/native_pdf/windows.rs:113 | WebView2 could not load the article for PDF export |  |
| src-tauri/src/native_pdf/windows.rs:130 | The Windows print spool rejected the job ({}) |  |
| src-tauri/src/native_pdf/windows.rs:136 | Sending the article to the Windows printer: {error} |  |
| src-tauri/src/native_pdf/windows.rs:143 | Native PDF export failed: {e} |  |
| src-tauri/src/native_pdf/windows.rs:143 | WebView2 did not save the article PDF |  |
| src-tauri/src/native_pdf/windows.rs:153 | Native PDF export failed: {error} |  |
| src-tauri/src/native_pdf/windows.rs:159 | A current InPrivate-capable WebView2 runtime is required for PDF export: {error} |  |
| src-tauri/src/native_pdf/windows.rs:165 | Native WebView2 PDF provider stopped |  |
| src-tauri/src/native_pdf/windows_print.rs:48 | Article printing was cancelled |  |
| src-tauri/src/native_pdf/windows_print.rs:62 | The Windows print dialog failed ({error}) |  |
| src-tauri/src/native_pdf/windows_print.rs:64 | Cannot apply the selected printer settings: {error} |  |
| src-tauri/src/native_pdf/windows_print.rs:72 | Missing printer identity |  |
| src-tauri/src/native_pdf/windows_print.rs:78 | Invalid printer identity |  |
| src-tauri/src/native_pdf/windows_print.rs:81 | No printer was selected |  |
| src-tauri/src/native_pdf/windows_print.rs:82 | Invalid printer name |  |
| src-tauri/src/native_pdf/windows_print.rs:86 | Missing printer configuration |  |
| src-tauri/src/native_pdf/windows_print.rs:99 | Invalid print page range |  |
| src-tauri/src/native_pdf/windows_print.rs:100 | {first}-{last} |  |
| src-tauri/src/native_pdf/windows_print.rs:106 | Unsupported printer duplex setting |  |
| src-tauri/src/native_pdf/windows_print.rs:111 | Invalid printer resolution |  |
| src-tauri/src/native_pdf/windows_print.rs:114 | Invalid printer paper dimensions |  |
| src-tauri/src/native_pdf/windows_print.rs:126 | The live article is too wide for the selected printer scale |  |
| src-tauri/src/native_pdf/windows_print.rs:135 | The live article is too wide for WebView2 PDF capture |  |
| src-tauri/src/native_pdf.rs:49 | Not enough temporary storage for article PDF export |  |
| src-tauri/src/native_pdf.rs:51 | pdf-{} |  |
| src-tauri/src/native_pdf.rs:52 | the builder is only configured with a Unix mode |  |
| src-tauri/src/native_pdf.rs:63 | iOS could not exclude PDF temporary storage from backup |  |
| src-tauri/src/native_pdf.rs:66 | article.pdf |  |
| src-tauri/src/native_pdf.rs:71 | Native PDF is missing, invalid, or exceeds the 64 MiB output limit |  |
| src-tauri/src/native_pdf.rs:76 | The native provider did not produce a PDF document |  |
| src-tauri/src/native_pdf.rs:145 | native origin stopped |  |
| src-tauri/src/native_pdf.rs:145 | <article origin> |  |
| src-tauri/src/native_pdf.rs:168 | invalid archive resource path |  |
| src-tauri/src/native_pdf.rs:183 | The article image could not be read. Try the export again |  |
| src-tauri/src/native_pdf.rs:188 | The article image stopped loading. Try the export again |  |
| src-tauri/src/native_pdf.rs:188 | <article image> |  |
| src-tauri/src/native_pdf.rs:192 | content-type |  |
| src-tauri/src/native_pdf.rs:192 | invalid resource media type |  |
| src-tauri/src/native_pdf.rs:195 | missing archive resource |  |
| src-tauri/src/native_pdf.rs:218 | System sharing is unavailable on this platform |  |
| src-tauri/src/native_pdf.rs:221 | Another article PDF export is still running or finishing |  |
| src-tauri/src/native_pdf.rs:270 | Article PDF export timed out |  |
| src-tauri/src/nearby_public_link.rs:86 | The sender supplied invalid public Nearby connection information |  |
| src-tauri/src/nearby_public_link.rs:92 | The sender supplied an unsupported temporary Wi-Fi network |  |
| src-tauri/src/nearby_public_link.rs:100 | Nearby connection information is too large |  |
| src-tauri/src/nearby_route.rs:28 | Invalid native Nearby route receipt |  |
| src-tauri/src/nearby_route.rs:36 | hotspotQuicState |  |
| src-tauri/src/nearby_route.rs:36 | nearbyJoinState |  |
| src-tauri/src/nearby_route.rs:42 | Automatic hotspot connections require a supported mobile device |  |
| src-tauri/src/nearby_route.rs:44 | Native mobile state and joined-route loss callbacks revoke the route |  |
| src-tauri/src/nearby_route.rs:54 | The joined Wi-Fi route has no confirmed local IPv4 address |  |
| src-tauri/src/nearby_route.rs:57 | Automatic Nearby joining requires an on-link IPv4 endpoint |  |
| src-tauri/src/nearby_route.rs:58 | Nearby requires an on-link private network endpoint |  |
| src-tauri/src/nearby_route.rs:72 | lease |  |
| src-tauri/src/nearby_route.rs:76 | Only Android can own a temporary hotspot route |  |
| src-tauri/src/nearby_route.rs:86 | Invalid native Nearby UDP route receipt |  |
| src-tauri/src/nearby_route.rs:94 | The Nearby UDP socket does not own its confirmed local address |  |
| src-tauri/src/nearby_route.rs:110 | Invalid native hotspot interface receipt |  |
| src-tauri/src/nearby_route.rs:111 | address |  |
| src-tauri/src/nearby_route.rs:111 | Invalid native hotspot IPv4 address |  |
| src-tauri/src/nearby_route.rs:114 | Invalid native hotspot lease |  |
| src-tauri/src/nearby_route.rs:115 | interfaceName |  |
| src-tauri/src/nearby_route.rs:116 | Invalid native hotspot interface name |  |
| src-tauri/src/nearby_route.rs:118 | interfaceIndex |  |
| src-tauri/src/nearby_route.rs:119 | Invalid native hotspot interface index |  |
| src-tauri/src/nearby_route.rs:120 | prefixLength |  |
| src-tauri/src/nearby_route.rs:121 | Invalid native hotspot IPv4 prefix |  |
| src-tauri/src/nearby_route.rs:126 | The selected peer is not on the owned hotspot interface |  |
| src-tauri/src/nearby_route.rs:157 | The owned hotspot requires a UDP socket |  |
| src-tauri/src/nearby_route.rs:160 | Android cannot bind QUIC to the exact temporary-hotspot interface: {} |  |
| src-tauri/src/nearby_route.rs:166 | Android did not retain the exact hotspot interface binding |  |
| src-tauri/src/nearby_route.rs:196 | Native Nearby route adapter is unavailable |  |
| src-tauri/src/nearby_service.rs:133 | Nearby connection stopped |  |
| src-tauri/src/nearby_service.rs:137 | Nearby connection expired after a privacy transition |  |
| src-tauri/src/nearby_service.rs:143 | Nearby state unavailable |  |
| src-tauri/src/nearby_service.rs:156 | Bluetooth discovery requires the iPhone, iPad or Android application. Use the same local network on this device. |  |
| src-tauri/src/nearby_service.rs:190 | nearbyBluetoothStop |  |
| src-tauri/src/nearby_service.rs:190 | owner |  |
| src-tauri/src/nearby_service.rs:191 | nearbyJoinCancel |  |
| src-tauri/src/nearby_service.rs:211 | nearbyRetire |  |
| src-tauri/src/nearby_service.rs:224 | Nearby radio owner is already active |  |
| src-tauri/src/nearby_service.rs:244 | The temporary hotspot is not ready |  |
| src-tauri/src/nearby_service.rs:245 | Native adapter unavailable |  |
| src-tauri/src/nearby_service.rs:251 | EXACT_INTERFACE_UNAVAILABLE |  |
| src-tauri/src/nearby_service.rs:251 | NEARBY_VISIBLE_ACTIVITY_REQUIRED |  |
| src-tauri/src/nearby_service.rs:259 | Invalid hotspot interface lease |  |
| src-tauri/src/nearby_service.rs:265 | Nearby sharing was retired | retired |
| src-tauri/src/nearby_service.rs:302 | Native Nearby listeners are not ready yet. Try again. |  |
| src-tauri/src/nearby_service.rs:304 | [nearby] publish library gate acquired |  |
| src-tauri/src/nearby_service.rs:326 | [nearby] publish failed: {error} |  |
| src-tauri/src/nearby_service.rs:356 | Choose and connect to the nearby sender first |  |
| src-tauri/src/nearby_service.rs:357 | Choose and connect to this nearby sender first |  |
| src-tauri/src/nearby_service.rs:358 | The Nearby sender is not connected |  |
| src-tauri/src/nearby_service.rs:361 | The Nearby connection ended. Connect again. |  |
| src-tauri/src/nearby_service.rs:382 | Open Nearby to find the sender again |  |
| src-tauri/src/nearby_service.rs:383 | The selected nearby sender is no longer available |  |
| src-tauri/src/nearby_service.rs:384 | A Nearby connection is already in progress |  |
| src-tauri/src/nearby_service.rs:395 | Nearby connection ended |  |
| src-tauri/src/nearby_service.rs:397 | nearby_discover |  |
| src-tauri/src/nearby_service.rs:397 | Unknown Nearby command |  |
| src-tauri/src/nearby_service.rs:446 | The nearby sender did not respond. Try again. |  |
| src-tauri/src/nearby_service.rs:447 | Bluetooth connection ended |  |
| src-tauri/src/nearby_service.rs:469 | nearbyBluetoothDisconnect |  |
| src-tauri/src/nearby_service.rs:469 | connection |  |
| src-tauri/src/nearby_service.rs:475 | connected |  |
| src-tauri/src/nearby_service.rs:526 | advertising |  |
| src-tauri/src/nearby_service.rs:544 | The temporary-network response is too large |  |
| src-tauri/src/nearby_service.rs:560 | Bluetooth is unavailable. Check its permissions. |  |
| src-tauri/src/nearby_service.rs:567 | This sender cannot join a receiver-hosted temporary network |  |
| src-tauri/src/nearby_service.rs:569 | Library sharing stopped |  |
| src-tauri/src/nearby_service.rs:575 | joining |  |
| src-tauri/src/nearby_service.rs:576 | nearbyJoin |  |
| src-tauri/src/nearby_service.rs:576 | reservationId |  |
| src-tauri/src/nearby_service.rs:589 | Invalid temporary-network endpoint |  |
| src-tauri/src/nearby_service.rs:641 | Nearby discovery stopped |  |
| src-tauri/src/nearby_service.rs:666 | Bluetooth discovery stopped |  |
| src-tauri/src/nearby_service.rs:668 | peer |  |
| src-tauri/src/nearby_service.rs:669 | lost |  |
| src-tauri/src/nearby_service.rs:675 | Bluetooth discovery stopped; the accepted library connection remains available. |  |
| src-tauri/src/nearby_service.rs:676 | Bluetooth discovery is unavailable |  |
| src-tauri/src/nearby_service.rs:718 | Nearby sender is no longer available |  |
| src-tauri/src/nearby_service.rs:738 | [nearby] connect exchange failed: {error} |  |
| src-tauri/src/nearby_service.rs:745 | [nearby] transient outer route proof failed: {error} |  |
| src-tauri/src/nearby_service.rs:749 | Nearby sender stopped |  |
| src-tauri/src/nearby_service.rs:752 | peerId |  |
| src-tauri/src/nearby_service.rs:752 | librarySessionId |  |
| src-tauri/src/nearby_service.rs:752 | publisherName |  |
| src-tauri/src/nearby_service.rs:769 | done |  |
| src-tauri/src/nearby_service.rs:793 | The sender has no common local-network endpoint |  |
| src-tauri/src/nearby_service.rs:809 | The selected Nearby network connection ended. Connect again. |  |
| src-tauri/src/nearby_service.rs:813 | The sender is not reachable on the current Wi-Fi |  |
| src-tauri/src/nearby_service.rs:832 | [nearby] direct route failed: {error} |  |
| src-tauri/src/nearby_service.rs:854 | [nearby] transient final route proof failed: {error} |  |
| src-tauri/src/nearby_service.rs:864 | [nearby] hosting request failed: {error} |  |
| src-tauri/src/nearby_service.rs:867 | [nearby] hosting request sent |  |
| src-tauri/src/nearby_service.rs:878 | Android did not finish creating the temporary Wi-Fi network |  |
| src-tauri/src/nearby_service.rs:884 | Android has no app-owned temporary hotspot |  |
| src-tauri/src/nearby_service.rs:886 | [nearby] reverse hotspot ready; sending join request |  |
| src-tauri/src/nearby_service.rs:889 | The temporary hotspot stopped |  |
| src-tauri/src/nearby_service.rs:900 | The temporary Wi-Fi connection ended while the sender was joining |  |
| src-tauri/src/nearby_service.rs:903 | Invalid sender temporary-network response |  |
| src-tauri/src/nearby_service.rs:907 | The sender could not join the temporary Wi-Fi network: {error} |  |
| src-tauri/src/nearby_service.rs:911 | The sender's public library changed while connecting |  |
| src-tauri/src/nearby_service.rs:917 | This sender uses an unsupported Nearby version |  |
| src-tauri/src/nearby_service.rs:918 | Invalid public Nearby share identity |  |
| src-tauri/src/nearby_service.rs:919 | packetBudget |  |
| src-tauri/src/nearby_service.rs:919 | Invalid Bluetooth packet budget |  |
| src-tauri/src/nearby_service.rs:940 | The selected sender disconnected before sending connection information |  |
| src-tauri/src/nearby_service.rs:975 | Bluetooth packet is missing |  |
| src-tauri/src/nearby_service.rs:976 | Invalid Bluetooth packet encoding |  |
| src-tauri/src/nearby_service.rs:977 | Invalid Bluetooth packet length or encoding |  |
| src-tauri/src/notifications.rs:55 | Allocating native notification reply |  |
| src-tauri/src/notifications.rs:64 | value |  |
| src-tauri/src/notifications.rs:64 | Missing native notification reply |  |
| src-tauri/src/notifications.rs:121 | Notification policy revision exhausted |  |
| src-tauri/src/notifications.rs:143 | Invalid native notification profile identity |  |
| src-tauri/src/notifications.rs:146 | Native notification profile was not admitted |  |
| src-tauri/src/notifications.rs:160 | Leave private or read-only mode before enabling notifications |  |
| src-tauri/src/notifications.rs:177 | Notification permission expired after a policy transition |  |
| src-tauri/src/notifications.rs:207 | Invalid acquisition notification receipt |  |
| src-tauri/src/notifications.rs:209 | Notification profile is unavailable |  |
| src-tauri/src/notifications.rs:211 | command |  |
| src-tauri/src/notifications.rs:211 | post |  |
| src-tauri/src/notifications.rs:212 | notice |  |
| src-tauri/src/notifications.rs:212 | receipt |  |
| src-tauri/src/notifications.rs:214 | Missing native notification admission result |  |
| src-tauri/src/notifications.rs:257 | Notifications are off in private or read-only mode |  |
| src-tauri/src/notifications.rs:259 | ack |  |
| src-tauri/src/notifications.rs:276 | Native notification authorization failed |  |
| src-tauri/src/notifications_service.rs:22 | Completion notifications are unavailable |  |
| src-tauri/src/notifications_service.rs:23 | {prefix}: {} |  |
| src-tauri/src/notifications_service.rs:52 | Native completion notifications are unavailable |  |
| src-tauri/src/notifications_service.rs:140 | Archive ready |  |
| src-tauri/src/notifications_service.rs:141 | A verified archive was added to your library. |  |
| src-tauri/src/os_open.rs:16 | The location is not valid UTF-8 |  |
| src-tauri/src/os_open.rs:24 | macOS rejected the link |  |
| src-tauri/src/os_open.rs:30 | macOS could not find an application to open it |  |
| src-tauri/src/partial_donor.rs:34 | Partial download has no parent directory |  |
| src-tauri/src/partial_donor.rs:35 | Partial download has no filename |  |
| src-tauri/src/partial_donor.rs:52 | Partial download is not a matching regular file |  |
| src-tauri/src/partial_donor.rs:61 | Partial download offset overflow |  |
| src-tauri/src/partial_donor.rs:67 | Partial download ended early |  |
| src-tauri/src/pdf_export.rs:98 | mobile_save_pdf |  |
| src-tauri/src/pdf_export.rs:98 | share_pdf |  |
| src-tauri/src/pdf_export.rs:98 | Use the native mobile PDF export action |  |
| src-tauri/src/pdf_export.rs:100 | save_pdf |  |
| src-tauri/src/pdf_export.rs:100 | print_pdf |  |
| src-tauri/src/pdf_export.rs:100 | Mobile PDF export is unavailable on this platform |  |
| src-tauri/src/pdf_export.rs:109 | window- |  |
| src-tauri/src/pdf_export.rs:109 | Open the article in the main window to export a PDF |  |
| src-tauri/src/pdf_export.rs:111 | The app window is unavailable. Reopen the app to continue |  |
| src-tauri/src/pdf_export.rs:114 | This window is out of date. Reopen the app to continue |  |
| src-tauri/src/pdf_export.rs:142 | Your privacy settings changed during PDF export. Start the export again |  |
| src-tauri/src/pdf_export.rs:147 | The window you started this from is out of date. Reopen the app and try again |  |
| src-tauri/src/pdf_export.rs:155 | The live article reader has retired; capture it again | retired |
| src-tauri/src/pdf_export.rs:167 | The article was closed or changed. Open it again before exporting |  |
| src-tauri/src/pdf_export.rs:169 | The live article changed during capture; export it again |  |
| src-tauri/src/pdf_export.rs:171 | The PDF capture identity is already in use |  |
| src-tauri/src/pdf_export.rs:177 | Article PDF export requires a native application window |  |
| src-tauri/src/pdf_export.rs:179 | .pdf |  |
| src-tauri/src/pdf_export.rs:181 | Invalid PDF filename suggestion |  |
| src-tauri/src/pdf_export.rs:183 | liveSnapshot |  |
| src-tauri/src/pdf_export.rs:184 | Capture the current article before exporting a PDF |  |
| src-tauri/src/pdf_export.rs:185 | Kiwix could not capture the current article: {error} |  |
| src-tauri/src/pdf_export.rs:194 | Article PDF export requires an HTML article |  |
| src-tauri/src/pdf_export.rs:204 | Native Android cache root is missing |  |
| src-tauri/src/pdf_export.rs:206 | Native Android cache root must be an existing absolute directory |  |
| src-tauri/src/pdf_export.rs:218 | Unsafe PDF temporary storage directory |  |
| src-tauri/src/pdf_export.rs:221 | Resolving PDF temporary storage: {error} |  |
| src-tauri/src/pdf_export.rs:225 | PDF export stopped because the app is closing |  |
| src-tauri/src/pdf_export.rs:251 | Share |  |
| src-tauri/src/pdf_export.rs:251 | Save export |  |
| src-tauri/src/pdf_export.rs:273 | The native print provider did not complete its system print operation |  |
| src-tauri/src/pdf_export.rs:292 | Kiwix could not share the article PDF. Try sharing it again |  |
| src-tauri/src/pdf_export.rs:294 | windows |  |
| src-tauri/src/pdf_export.rs:311 | Android PDF export must use the native system print UI |  |
| src-tauri/src/pdf_export.rs:321 | application/pdf |  |
| src-tauri/src/pdf_export.rs:321 | pdfAuthority |  |
| src-tauri/src/pdf_export.rs:333 | cancelPdfExport |  |
| src-tauri/src/pdf_export.rs:335 | The native PDF export picker timed out |  |
| src-tauri/src/pdf_export.rs:340 | The native PDF picker returned no completion state |  |
| src-tauri/src/permissions.rs:160 | This archive cannot use device sensors in this window. Open the article again |  |
| src-tauri/src/permissions.rs:162 | Sensor state unavailable |  |
| src-tauri/src/permissions.rs:218 | Retained sensor session is no longer owned by this reader |  |
| src-tauri/src/permissions.rs:294 | Archive sensor session is no longer active |  |
| src-tauri/src/permissions.rs:302 | Reset this sensor's denied decision before asking again |  |
| src-tauri/src/permissions.rs:446 | Location consent was revoked |  |
| src-tauri/src/permissions.rs:467 | Use the browser geolocation provider on this platform |  |
| src-tauri/src/permissions.rs:491 | Location permission has not been granted for this archive |  |
| src-tauri/src/permissions.rs:498 | Duplicate location request |  |
| src-tauri/src/permissions.rs:502 | Too many pending location requests |  |
| src-tauri/src/permissions.rs:508 | Location request identifiers exhausted |  |
| src-tauri/src/permissions.rs:544 | Native location provider stopped |  |
| src-tauri/src/permissions.rs:656 | Native location provider returned an invalid position |  |
| src-tauri/src/portable.rs:19 | Portable directory must not be a symbolic link: {} |  |
| src-tauri/src/portable.rs:32 | APPIMAGE has no distribution directory |  |
| src-tauri/src/portable.rs:35 | APPIMAGE must name the absolute regular, non-symlink distributed AppImage |  |
| src-tauri/src/portable.rs:37 | Executable has no distribution directory |  |
| src-tauri/src/portable.rs:39 | MacOS |  |
| src-tauri/src/portable.rs:40 | Contents |  |
| src-tauri/src/portable.rs:42 | App bundle has no distribution directory |  |
| src-tauri/src/portable.rs:65 | Cannot inspect portable marker: {error} |  |
| src-tauri/src/portable.rs:67 | Portable marker .portable must be a regular file, not a directory or symbolic link |  |
| src-tauri/src/portable.rs:75 | Portable marker changed while opening |  |
| src-tauri/src/portable.rs:78 | data |  |
| src-tauri/src/portable.rs:112 | Portable data has no parent |  |
| src-tauri/src/portable.rs:116 | Portable data must be a real non-symlink directory |  |
| src-tauri/src/portable.rs:134 | Cannot create portable data directory: {error} |  |
| src-tauri/src/portable.rs:136 | Cannot open portable data directory: {error} |  |
| src-tauri/src/portable.rs:137 | Portable data must be a real directory, not a symbolic link |  |
| src-tauri/src/portable.rs:144 | Portable app-owned directory {name} must be a real directory |  |
| src-tauri/src/portable.rs:158 | Portable automatic directory {root}/{child} must be a real directory |  |
| src-tauri/src/portable.rs:167 | Portable app-owned file {name} must be a regular non-symlink file |  |
| src-tauri/src/portable.rs:186 | Invalid portable webview label |  |
| src-tauri/src/portable.rs:191 | Portable webview root must be a real directory |  |
| src-tauri/src/portable.rs:198 | Portable webview profile must be a real directory |  |
| src-tauri/src/portable.rs:231 | Unsupported or invalid portable root history |  |
| src-tauri/src/portable.rs:235 | Portable directory must be valid Unicode |  |
| src-tauri/src/portable.rs:237 | Portable root history has reached its safety limit |  |
| src-tauri/src/portable.rs:241 | Portable root history exceeds its safety size limit |  |
| src-tauri/src/portable.rs:242 | .portable-roots-{}.tmp |  |
| src-tauri/src/portable.rs:265 | Portable torrent root must be a real directory |  |
| src-tauri/src/portable.rs:268 | jobs.json |  |
| src-tauri/src/portable.rs:273 | Invalid torrent job ID |  |
| src-tauri/src/portable.rs:275 | Torrent job has no completed paths |  |
| src-tauri/src/portable.rs:277 | Portable torrent completed path is outside portable data |  |
| src-tauri/src/portable.rs:285 | Conflicting portable torrent receipt correlations |  |
| src-tauri/src/portable.rs:300 | Portable filesystem path contains NUL |  |
| src-tauri/src/portable.rs:302 | / |  |
| src-tauri/src/portable.rs:303 | Portable filesystem path contains traversal |  |
| src-tauri/src/portable.rs:358 | Saved automatic job points outside portable data: {original} |  |
| src-tauri/src/portable.rs:379 | Edition acquisition receipt is not a string |  |
| src-tauri/src/portable.rs:402 | Portable root history contains an invalid absolute path |  |
| src-tauri/src/portable.rs:418 | Portable archive pathname contains an invalid native component |  |
| src-tauri/src/portable.rs:422 | Portable archive pathname cannot be represented on Windows |  |
| src-tauri/src/portable.rs:430 | Portable path is not a normalized relative pathname |  |
| src-tauri/src/portable.rs:431 | Portable path is not Unicode |  |
| src-tauri/src/portable.rs:441 | A saved file in the portable folder is not valid. Replace it from a backup |  |
| src-tauri/src/portable.rs:447 | A saved file changed while Kiwix was reading it. Restart the app |  |
| src-tauri/src/portable.rs:450 | A saved file is larger than expected. Restart the app |  |
| src-tauri/src/portable.rs:458 | Archive path is not a string |  |
| src-tauri/src/portable.rs:460 | Archive path is not Unicode |  |
| src-tauri/src/portable.rs:461 | portableComponents |  |
| src-tauri/src/power_activity.rs:170 | wifiPerformance |  |
| src-tauri/src/power_activity.rs:170 | held |  |
| src-tauri/src/private_sync.rs:37 | Too many connected private devices |  |
| src-tauri/src/private_sync.rs:45 | Invalid connected private device |  |
| src-tauri/src/private_sync.rs:53 | Invalid private device identity |  |
| src-tauri/src/private_sync.rs:98 | The nearby device identity does not match the scanned invitation |  |
| src-tauri/src/private_sync.rs:100 | Private handshake failed |  |
| src-tauri/src/private_sync.rs:110 | Private device did not authenticate an identity |  |
| src-tauri/src/private_sync.rs:113 | Unexpected private device identity |  |
| src-tauri/src/private_sync.rs:128 | Invalid private encrypted frame length |  |
| src-tauri/src/private_sync.rs:138 | Private reading data exceeds the supported transfer bound. Remove unused connected devices or reduce the reading list before syncing |  |
| src-tauri/src/private_sync.rs:147 | Private message exceeds the supported transfer bound |  |
| src-tauri/src/private_sync.rs:151 | Invalid private message framing |  |
| src-tauri/src/private_sync.rs:154 | Invalid private reading-data message |  |
| src-tauri/src/private_sync.rs:158 | Invalid private encrypted frame |  |
| src-tauri/src/private_sync_identity.rs:49 | profile directory was substituted |  |
| src-tauri/src/private_sync_identity.rs:89 | key file was substituted or modified |  |
| src-tauri/src/private_sync_identity.rs:100 | profile must be owned by this user and not writable by other users |  |
| src-tauri/src/private_sync_identity.rs:109 | identity directory must be owned by this user with permissions 0700 |  |
| src-tauri/src/private_sync_identity.rs:115 | identity must be a regular, single-link, exactly 64-byte file |  |
| src-tauri/src/private_sync_identity.rs:120 | identity file must be owned by this user with permissions 0600 |  |
| src-tauri/src/private_sync_identity.rs:129 | this platform has no supported private-file permission implementation |  |
| src-tauri/src/private_sync_identity.rs:149 | identity changed while opening |  |
| src-tauri/src/private_sync_identity.rs:156 | identity grew while reading |  |
| src-tauri/src/private_sync_identity.rs:160 | identity changed while reading |  |
| src-tauri/src/private_sync_identity.rs:165 | X25519 is unavailable |  |
| src-tauri/src/private_sync_identity.rs:169 | identity public key does not match its secret |  |
| src-tauri/src/private_sync_identity.rs:197 | Noise generated an invalid secret key length |  |
| src-tauri/src/private_sync_identity.rs:233 | concurrent identity creation disappeared |  |
| src-tauri/src/private_sync_identity.rs:280 | could not determine the Windows user SID |  |
| src-tauri/src/private_sync_identity.rs:318 | Windows private DACL is unavailable |  |
| src-tauri/src/private_sync_identity.rs:347 | Windows identity storage must be owned by this user with a restrictive DACL |  |
| src-tauri/src/private_sync_identity.rs:354 | Windows identity storage must have a protected, single-user DACL |  |
| src-tauri/src/private_sync_identity.rs:360 | Windows identity storage has an invalid ACE |  |
| src-tauri/src/private_sync_identity.rs:363 | Windows identity storage has an unexpected ACE type |  |
| src-tauri/src/private_sync_identity.rs:370 | Windows profile grants access to another user or group |  |
| src-tauri/src/private_sync_identity.rs:374 | Windows identity permissions are not current-user-only full control |  |
| src-tauri/src/private_sync_service.rs:104 | Private sync is paused. Return to the app and try again |  |
| src-tauri/src/private_sync_service.rs:107 | Private pairing unavailable |  |
| src-tauri/src/private_sync_service.rs:109 | This connection invitation has expired. Create a new invitation |  |
| src-tauri/src/private_sync_service.rs:125 | Private sync admission was retired. Return to the app and try again | retired |
| src-tauri/src/private_sync_service.rs:128 | Private sync is paused in private or read-only mode |  |
| src-tauri/src/private_sync_service.rs:131 | Return to the app before syncing private reading data |  |
| src-tauri/src/private_sync_service.rs:132 | Private reading sync is disabled |  |
| src-tauri/src/private_sync_service.rs:139 | Private sync is paused |  |
| src-tauri/src/private_sync_service.rs:176 | Connect both devices to the same Wi-Fi or Ethernet network before using private Nearby sync |  |
| src-tauri/src/private_sync_service.rs:184 | Private sync startup was cancelled |  |
| src-tauri/src/private_sync_service.rs:231 | Private sync is unavailable in private or read-only mode |  |
| src-tauri/src/private_sync_service.rs:243 | Choose whether to enable private sync |  |
| src-tauri/src/private_sync_service.rs:244 | categories |  |
| src-tauri/src/private_sync_service.rs:244 | Choose reading data to sync |  |
| src-tauri/src/private_sync_service.rs:244 | Invalid private sync categories |  |
| src-tauri/src/private_sync_service.rs:262 | Connected device no longer exists |  |
| src-tauri/src/private_sync_service.rs:264 | Device name is too long or empty |  |
| src-tauri/src/private_sync_service.rs:276 | alternativeIds |  |
| src-tauri/src/private_sync_service.rs:276 | Review the conflict alternatives before resolving |  |
| src-tauri/src/private_sync_service.rs:276 | Invalid conflict alternatives |  |
| src-tauri/src/private_sync_service.rs:278 | This conflict changed. Review its current alternatives before resolving |  |
| src-tauri/src/private_sync_service.rs:281 | This conflict changed. Review every current alternative before resolving |  |
| src-tauri/src/private_sync_service.rs:289 | This reading position is no longer available |  |
| src-tauri/src/private_sync_service.rs:290 | Add this archive to your local library before resuming this reading position |  |
| src-tauri/src/private_sync_service.rs:291 | scroll |  |
| src-tauri/src/private_sync_service.rs:293 | private_sync_pair_cancel |  |
| src-tauri/src/private_sync_service.rs:303 | confirmed |  |
| src-tauri/src/private_sync_service.rs:303 | Confirm that all connected devices must reconnect before resetting sync history |  |
| src-tauri/src/private_sync_service.rs:305 | private_sync_clear_positions |  |
| src-tauri/src/private_sync_service.rs:321 | private_sync_status |  |
| src-tauri/src/private_sync_service.rs:321 | private_sync_configure |  |
| src-tauri/src/private_sync_service.rs:321 | Enable private reading sync first |  |
| src-tauri/src/private_sync_service.rs:353 | Connection invitation is too large |  |
| src-tauri/src/private_sync_service.rs:354 | Invalid private connection invitation |  |
| src-tauri/src/private_sync_service.rs:356 | Invalid invitation expiry |  |
| src-tauri/src/private_sync_service.rs:360 | This invitation is invalid or expired. Create a new invitation on the other device |  |
| src-tauri/src/private_sync_service.rs:361 | Invitation expired |  |
| src-tauri/src/private_sync_service.rs:362 | waiting |  |
| src-tauri/src/private_sync_service.rs:372 | private_sync_pair_confirm |  |
| src-tauri/src/private_sync_service.rs:375 | peerIdentity |  |
| src-tauri/src/private_sync_service.rs:375 | Verify the current comparison code on both devices before connecting |  |
| src-tauri/src/private_sync_service.rs:380 | Unknown private sync command: {command} |  |
| src-tauri/src/private_sync_service.rs:405 | This private device is disabled or no longer connected |  |
| src-tauri/src/private_sync_service.rs:407 | Private sync status unavailable |  |
| src-tauri/src/private_sync_service.rs:414 | Private sync was cancelled |  |
| src-tauri/src/private_sync_service.rs:414 | Private sync timed out. Keep both apps open on the same network |  |
| src-tauri/src/private_sync_service.rs:430 | The connected device is unavailable. Keep both apps open on the same Wi-Fi network; create a new invitation if its network address changed |  |
| src-tauri/src/private_sync_service.rs:439 | Invalid private device greeting |  |
| src-tauri/src/private_sync_service.rs:448 | Private invitation was cancelled |  |
| src-tauri/src/private_sync_service.rs:449 | pair |  |
| src-tauri/src/private_sync_service.rs:460 | No active private connection invitation |  |
| src-tauri/src/private_sync_service.rs:465 | Invalid or already used private invitation |  |
| src-tauri/src/private_sync_service.rs:474 | sync |  |
| src-tauri/src/private_sync_service.rs:480 | Unknown private reading protocol |  |
| src-tauri/src/private_sync_service.rs:493 | Private invitation expired or was cancelled |  |
| src-tauri/src/private_sync_service.rs:498 | Private pairing was cancelled |  |
| src-tauri/src/private_sync_service.rs:499 | Private invitation expired |  |
| src-tauri/src/private_sync_service.rs:512 | Invalid private device confirmation |  |
| src-tauri/src/private_sync_service.rs:520 | Disconnect an unused private device before connecting another |  |
| src-tauri/src/private_sync_service.rs:528 | Private device did not finish confirmation |  |
| src-tauri/src/private_sync_service.rs:538 | This private device is not connected or was revoked |  |
| src-tauri/src/private_sync_service.rs:569 | Private device sent an unexpected response |  |
| src-tauri/src/private_sync_service.rs:573 | Private device did not acknowledge durable sync |  |
| src-tauri/src/private_sync_service.rs:575 | Private device did not send reading data |  |
| src-tauri/src/private_sync_service.rs:588 | Private device was revoked |  |
| src-tauri/src/profile_paths.rs:26 | Persisted path list is not an array |  |
| src-tauri/src/profile_paths.rs:51 | Saved edition records are not an array | record |
| src-tauri/src/profile_paths.rs:52 | editionIntents |  |
| src-tauri/src/profile_paths.rs:53 | destination |  |
| src-tauri/src/profile_paths.rs:75 | Download state is not an object |  |
| src-tauri/src/profile_paths.rs:81 | staging |  |
| src-tauri/src/profile_paths.rs:81 | directory |  |
| src-tauri/src/profile_paths.rs:81 | metadata |  |
| src-tauri/src/profile_paths.rs:90 | LocalSwarm state is not an object |  |
| src-tauri/src/profile_paths.rs:92 | owned |  |
| src-tauri/src/profile_paths.rs:92 | parent |  |
| src-tauri/src/profile_paths.rs:112 | Retained map publications are not an array |  |
| src-tauri/src/profile_paths.rs:113 | published |  |
| src-tauri/src/profile_paths.rs:113 | record |  |
| src-tauri/src/profile_paths.rs:120 | A saved map download could not be read. Restart the app |  |
| src-tauri/src/provider_reference.rs:18 | Kiwix cannot reopen this file from the other app. Choose it again |  |
| src-tauri/src/provider_reference.rs:28 | External archive references cannot be managed filesystem paths |  |
| src-tauri/src/provider_reference.rs:35 | Invalid external archive display name |  |
| src-tauri/src/provider_reference.rs:41 | Invalid external archive part count |  |
| src-tauri/src/provider_reference.rs:45 | Split external archives require an explicitly selected same-folder capability |  |
| src-tauri/src/provider_reference.rs:53 | External archive parts are missing, duplicated, or out of order |  |
| src-tauri/src/provider_reference.rs:80 | Native provider returned an invalid acquisition error |  |
| src-tauri/src/provider_reference.rs:81 | PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT |  |
| src-tauri/src/provider_reference.rs:83 | This archive is temporarily unavailable. Reconnect its storage, then reopen or reload the article. |  |
| src-tauri/src/provider_reference.rs:90 | code |  |
| src-tauri/src/provider_reference.rs:90 | message |  |
| src-tauri/src/provider_reference.rs:90 | retry |  |
| src-tauri/src/provider_reference.rs:90 | sourceReference |  |
| src-tauri/src/provider_reference.rs:109 | Native provider resolver is not initialized |  |
| src-tauri/src/provider_reference.rs:118 | prepareExternalArchiveMove |  |
| src-tauri/src/provider_reference.rs:118 | reference |  |
| src-tauri/src/provider_reference.rs:119 | The provider has not authorized moving this archive: {error}. Choose Copy to keep the original. |  |
| src-tauri/src/provider_reference.rs:120 | Native provider move receipt missing |  |
| src-tauri/src/provider_reference.rs:121 | Native provider move receipt invalid |  |
| src-tauri/src/provider_reference.rs:130 | deleteExternalArchiveMove |  |
| src-tauri/src/provider_reference.rs:130 | parts |  |
| src-tauri/src/provider_reference.rs:131 | The managed copy is ready, but the provider original was preserved: {error} |  |
| src-tauri/src/provider_reference.rs:132 | deleted |  |
| src-tauri/src/provider_reference.rs:133 | Provider deletion was not confirmed; the managed copy and recovery journal were preserved |  |
| src-tauri/src/provider_reference.rs:174 | Native provider lease missing |  |
| src-tauri/src/provider_reference.rs:175 | Native provider lease invalid |  |
| src-tauri/src/provider_reference.rs:177 | files |  |
| src-tauri/src/provider_reference.rs:177 | Native provider files missing |  |
| src-tauri/src/provider_reference.rs:182 | Native provider descriptor invalid |  |
| src-tauri/src/provider_reference.rs:189 | This provider does not offer a seekable regular file. Choose Import archives to make a managed copy. |  |
| src-tauri/src/publication.rs:37 | Verified publication requires a regular file of the expected size without hard links |  |
| src-tauri/src/publication.rs:108 | Publication path has no filename |  |
| src-tauri/src/publication.rs:127 | Verified publication requires sibling filenames |  |
| src-tauri/src/publication.rs:300 | Duplicate directory publication child |  |
| src-tauri/src/publication.rs:351 | Confined publication requires a single filename |  |
| src-tauri/src/publication.rs:420 | Invalid confined publication filename |  |
| src-tauri/src/publication.rs:435 | Publication source must be a regular file |  |
| src-tauri/src/publication.rs:549 | Publication path contains NUL |  |
| src-tauri/src/quit.rs:50 | Quit flush is restricted to application windows |  |
| src-tauri/src/quit.rs:68 | Quit flush is restricted to the trusted application origin |  |
| src-tauri/src/quit.rs:93 | Quit manager unavailable |  |
| src-tauri/src/quit.rs:99 | Cannot open a window while application Quit is pending |  |
| src-tauri/src/quit.rs:119 | No application quit is pending |  |
| src-tauri/src/quit.rs:124 | Unknown, stale, or already acknowledged quit request |  |
| src-tauri/src/quit.rs:195 | Quit request was cancelled |  |
| src-tauri/src/quit.rs:210 | Application quit is already pending |  |
| src-tauri/src/quit.rs:263 | kiwix-quit-cancelled |  |
| src-tauri/src/quit.rs:292 | kiwix-quit-flush |  |
| src-tauri/src/quit.rs:295 | Cannot request session flush for {}: {error} |  |
| src-tauri/src/quit.rs:309 | Quit cancelled: session flush timed out for {}. Retry Quit or close the affected window explicitly. |  |
| src-tauri/src/quit.rs:309 | , |  |
| src-tauri/src/quit.rs:318 | Library service unavailable during Quit |  |
| src-tauri/src/reading_data.rs:105 | Reading sync contains an inconsistent causal event |  |
| src-tauri/src/reading_data.rs:265 | Reading sync contains invalid or oversized text |  |
| src-tauri/src/reading_data.rs:270 | Reading sync contains an empty identifier |  |
| src-tauri/src/reading_data.rs:275 | Reading sync contains an invalid replica identity |  |
| src-tauri/src/reading_data.rs:280 | Reading sync contains an invalid generation | generation |
| src-tauri/src/reading_data.rs:291 | Reading sync generation has no causal creation event | generation |
| src-tauri/src/reading_data.rs:301 | Reading sync locations must be archive-relative references |  |
| src-tauri/src/reading_data.rs:316 | Reading sync contains an empty causal register |  |
| src-tauri/src/reading_data.rs:317 | Reading sync contains an invalid causal counter |  |
| src-tauri/src/reading_data.rs:320 | Reading sync contains an invalid causal event |  |
| src-tauri/src/reading_data.rs:327 | Reading sync has reached its reset safety limit. Use a new profile; retired identities were not discarded | retired |
| src-tauri/src/reading_data.rs:332 | Reading sync contains an invalid canonical bookmark identity |  |
| src-tauri/src/reading_data.rs:333 | Reading sync contains an empty bookmark generation set | generation |
| src-tauri/src/reading_data.rs:344 | Reading sync bookmark does not match its canonical article identity |  |
| src-tauri/src/reading_data.rs:349 | Reading sync has too many tags |  |
| src-tauri/src/reading_data.rs:356 | Reading sync contains an invalid highlight identity |  |
| src-tauri/src/reading_data.rs:357 | Reading sync contains an empty highlight generation set | generation |
| src-tauri/src/reading_data.rs:367 | Reading sync comment exceeds its allowed bounds |  |
| src-tauri/src/reading_data.rs:380 | Reading sync requires finite nonnegative scroll positions |  |
| src-tauri/src/reading_data.rs:381 | Reading sync contains an inconsistent article identity |  |
| src-tauri/src/reading_data.rs:384 | Reading sync position has the wrong replica owner |  |
| src-tauri/src/reading_data.rs:401 | Reading sync has inconsistent local bookmark identities |  |
| src-tauri/src/reading_data.rs:404 | Reading sync has no local bookmark projection identity |  |
| src-tauri/src/reading_data.rs:415 | Reading sync replica counter rolled back; rebootstrap this device |  |
| src-tauri/src/reading_data.rs:477 | Reading sync contains duplicate local bookmark IDs |  |
| src-tauri/src/reading_data.rs:666 | Resolve all reading conflicts before rebootstrap so no alternatives are discarded |  |
| src-tauri/src/reading_data.rs:712 | Unsupported reading sync snapshot version | snapshot |
| src-tauri/src/reading_data.rs:828 | The selected conflict alternative no longer exists |  |
| src-tauri/src/reading_data.rs:838 | The reading conflict no longer exists |  |
| src-tauri/src/reading_data.rs:845 | The selected recreation no longer exists |  |
| src-tauri/src/sandbox.rs:269 | The saved bookmark is corrupt |  |
| src-tauri/src/sandbox.rs:482 | The path is not valid UTF-8 |  |
| src-tauri/src/sandbox.rs:513 | The system refused access |  |
| src-tauri/src/sandbox.rs:515 | The bookmark does not name a file |  |
| src-tauri/src/search.rs:35 | Unknown search mode |  |
| src-tauri/src/search.rs:139 | hasMore |  |
| src-tauri/src/search.rs:139 | hits |  |
| src-tauri/src/server.rs:104 | Archive location contains a malformed percent escape |  |
| src-tauri/src/server.rs:193 | Archive worker failed: {error} |  |
| src-tauri/src/server.rs:250 | Reading stopped. Reload the article |  |
| src-tauri/src/server.rs:418 | Invalid storage mutation batch |  |
| src-tauri/src/server.rs:420 | Invalid storage mutation batch fields |  |
| src-tauri/src/server.rs:427 | Invalid storage {field} |  |
| src-tauri/src/server.rs:430 | documentId |  |
| src-tauri/src/server.rs:435 | Invalid storage mutation sequence |  |
| src-tauri/src/server.rs:437 | operations |  |
| src-tauri/src/server.rs:440 | Invalid storage mutations |  |
| src-tauri/src/server.rs:442 | Storage mutation batch is too large |  |
| src-tauri/src/server.rs:445 | Storage mutation queue unavailable |  |
| src-tauri/src/server.rs:459 | Reader privacy session expired |  |
| src-tauri/src/server.rs:461 | Storage mutation rejected |  |
| src-tauri/src/server.rs:463 | Reader storage is read-only |  |
| src-tauri/src/server.rs:472 | Storage mutation sequence is out of order |  |
| src-tauri/src/server.rs:475 | Unknown or exhausted storage document stream |  |
| src-tauri/src/server.rs:624 | Offline archive server panicked: {message} |  |
| src-tauri/src/server.rs:631 | {server}; {workers} |  |
| src-tauri/src/server.rs:665 | Offline archive shutdown task failed: {error} |  |
| src-tauri/src/server.rs:668 | {failure}; {workers} |  |
| src-tauri/src/server.rs:733 | host |  |
| src-tauri/src/server.rs:734 | Unknown archive host |  |
| src-tauri/src/server.rs:745 | Archive-controlled service workers are not supported |  |
| src-tauri/src/server.rs:750 | Foreign archive origin |  |
| src-tauri/src/server.rs:753 | /.kiwix/{}/ |  |
| src-tauri/src/server.rs:758 | Archives are read-only |  |
| src-tauri/src/server.rs:762 | Archive URL is not valid UTF-8 |  |
| src-tauri/src/server.rs:787 | Archive worker stopped |  |
| src-tauri/src/server.rs:823 | read_article |  |
| src-tauri/src/server.rs:1122 | Truncated archive document |  |
| src-tauri/src/server.rs:1267 | Content-Range |  |
| src-tauri/src/server.rs:1267 | bytes */{} |  |
| src-tauri/src/server.rs:1305 | read_resource |  |
| src-tauri/src/server.rs:1312 | Truncated archive resource |  |
| src-tauri/src/server.rs:1350 | Archive resources are read-only |  |
| src-tauri/src/server.rs:1355 | Invalid resource path |  |
| src-tauri/src/server.rs:1368 | HTML documents must use the instrumented reader |  |
| src-tauri/src/server.rs:1398 | Reader origin unavailable |  |
| src-tauri/src/server.rs:1411 | Unknown reader resource |  |
| src-tauri/src/server.rs:1438 | This link must be opened inside Kiwix |  |
| src-tauri/src/server.rs:1444 | This link can only be opened, not changed |  |
| src-tauri/src/server.rs:1458 | This link is out of date. Reload the article |  |
| src-tauri/src/server.rs:1530 | Invalid storage mutation |  |
| src-tauri/src/server.rs:1532 | local-set |  |
| src-tauri/src/server.rs:1532 | key |  |
| src-tauri/src/server.rs:1533 | local-remove |  |
| src-tauri/src/server.rs:1533 | cookie-remove |  |
| src-tauri/src/server.rs:1536 | Unknown storage mutation |  |
| src-tauri/src/server.rs:1539 | Invalid storage mutation fields |  |
| src-tauri/src/server.rs:1546 | Invalid storage key |  |
| src-tauri/src/server.rs:1554 | Invalid local storage value |  |
| src-tauri/src/server.rs:1562 | cookie |  |
| src-tauri/src/server.rs:1562 | Missing cookie |  |
| src-tauri/src/server.rs:1563 | Invalid cookie |  |
| src-tauri/src/server.rs:1565 | expires |  |
| src-tauri/src/server.rs:1569 | Invalid cookie fields |  |
| src-tauri/src/server.rs:1575 | Invalid cookie name |  |
| src-tauri/src/server.rs:1580 | Invalid cookie path |  |
| src-tauri/src/server.rs:1585 | Invalid cookie value |  |
| src-tauri/src/server.rs:1590 | Invalid cookie expiry |  |
| src-tauri/src/server.rs:1591 | {path}\n{name} |  |
| src-tauri/src/server.rs:1592 | Cookie identity mismatch |  |
| src-tauri/src/server.rs:1608 | localStorage |  |
| src-tauri/src/server.rs:1608 | cookies |  |
| src-tauri/src/server.rs:1610 | Archive storage quota exceeded |  |
| src-tauri/src/server.rs:1732 | Unsupported range unit |  |
| src-tauri/src/server.rs:1734 | Only one nonempty byte range is supported |  |
| src-tauri/src/server.rs:1736 | Invalid range |  |
| src-tauri/src/server.rs:1738 | Invalid suffix |  |
| src-tauri/src/server.rs:1740 | Empty suffix |  |
| src-tauri/src/server.rs:1747 | Range outside resource |  |
| src-tauri/src/server.rs:1755 | Inverted range |  |
| src-tauri/src/server.rs:1819 | Unknown preview host |  |
| src-tauri/src/server.rs:1821 | service-worker |  |
| src-tauri/src/server.rs:1829 | Shell service workers are not supported |  |
| src-tauri/src/server.rs:1837 | Foreign preview origin |  |
| src-tauri/src/server.rs:1839 | /api |  |
| src-tauri/src/server.rs:1843 | Preview origin required |  |
| src-tauri/src/server.rs:1845 | authorization |  |
| src-tauri/src/server.rs:1851 | Preview authorization required |  |
| src-tauri/src/server.rs:1855 | Invalid command JSON |  |
| src-tauri/src/server.rs:1858 | Missing command |  |
| src-tauri/src/server.rs:1867 | Untrusted reader parent origin |  |
| src-tauri/src/server.rs:1888 | Read-only application assets |  |
| src-tauri/src/server.rs:1895 | Application asset not found |  |
| src-tauri/src/server.rs:1950 | <pre>{}</pre> |  |
| src-tauri/src/service/dispatch.rs:113 | Saved map download state could not be read: {error} |  |
| src-tauri/src/service/dispatch.rs:171 | “{title}” is not available in this app’s storage. Choose Locate in the library to reconnect it. |  |
| src-tauri/src/service/dispatch.rs:211 | Too many launch acknowledgements |  |
| src-tauri/src/service/dispatch.rs:248 | preference |  |
| src-tauri/src/service/dispatch.rs:248 | Missing speech preference |  |
| src-tauri/src/service/dispatch.rs:249 | Invalid speech preference: {error} |  |
| src-tauri/src/service/dispatch.rs:263 | zoom |  |
| src-tauri/src/service/dispatch.rs:263 | Missing book zoom |  |
| src-tauri/src/service/dispatch.rs:270 | Book zoom must be from 25% to 500% |  |
| src-tauri/src/service/dispatch.rs:277 | Archive is not in the library |  |
| src-tauri/src/service/dispatch.rs:297 | Search history query is invalid or too large |  |
| src-tauri/src/service/dispatch.rs:315 | Article preview location is too long |  |
| src-tauri/src/service/dispatch.rs:322 | Article preview MIME type is invalid |  |
| src-tauri/src/service/dispatch.rs:353 | Resolved article preview location is too long |  |
| src-tauri/src/service/dispatch.rs:356 | excerpt |  |
| src-tauri/src/service/dispatch.rs:356 | thumbnail |  |
| src-tauri/src/service/dispatch.rs:386 | Missing restricted state |  |
| src-tauri/src/service/dispatch.rs:452 | Stop sharing this archive before relinking it |  |
| src-tauri/src/service/dispatch.rs:465 | Selected file is a different archive; import it separately |  |
| src-tauri/src/service/dispatch.rs:480 | Unknown archive |  |
| src-tauri/src/service/dispatch.rs:492 | Archive installation changed while relinking |  |
| src-tauri/src/service/dispatch.rs:522 | Stop sharing this archive before removing it from the library |  |
| src-tauri/src/service/dispatch.rs:524 | cleanupSource |  |
| src-tauri/src/service/dispatch.rs:527 | Acquisition cleanup requires explicit file deletion |  |
| src-tauri/src/service/dispatch.rs:539 | External provider content is read-only; remove its library reference instead |  |
| src-tauri/src/service/dispatch.rs:551 | Archive installation changed while confirming removal; select it again |  |
| src-tauri/src/service/dispatch.rs:554 | Session archive changed while removing |  |
| src-tauri/src/service/dispatch.rs:580 | deleteFile |  |
| src-tauri/src/service/dispatch.rs:605 | Stop the integrity check before deleting this archive |  |
| src-tauri/src/service/dispatch.rs:749 | Invalid search error response |  |
| src-tauri/src/service/dispatch.rs:782 | Select an archive to suggest titles from |  |
| src-tauri/src/service/dispatch.rs:785 | suggestions |  |
| src-tauri/src/service/dispatch.rs:785 | nextToken |  |
| src-tauri/src/service/dispatch.rs:788 | Suggestion prefix is too long |  |
| src-tauri/src/service/dispatch.rs:814 | This window's session has expired. Reopen the window to continue |  |
| src-tauri/src/service/dispatch.rs:823 | tabs |  |
| src-tauri/src/service/dispatch.rs:826 | Too many tabs |  |
| src-tauri/src/service/dispatch.rs:828 | referenceRevision |  |
| src-tauri/src/service/dispatch.rs:835 | Invalid window surface |  |
| src-tauri/src/service/dispatch.rs:897 | Missing visit creation intent |  |
| src-tauri/src/service/dispatch.rs:1009 | bookmark |  |
| src-tauri/src/service/dispatch.rs:1009 | Missing bookmark |  |
| src-tauri/src/service/dispatch.rs:1018 | highlightChange and highlights cannot be supplied together |  |
| src-tauri/src/service/dispatch.rs:1021 | highlightChange |  |
| src-tauri/src/service/dispatch.rs:1051 | The bookmark no longer exists |  |
| src-tauri/src/service/dispatch.rs:1062 | New bookmarks require a title |  |
| src-tauri/src/service/dispatch.rs:1150 | The original library directory must be absolute |  |
| src-tauri/src/service/dispatch.rs:1302 | {error}; background download policy: {policy} |  |
| src-tauri/src/service/dispatch.rs:1323 | Background download policy: {error} |  |
| src-tauri/src/service/dispatch.rs:1324 | {error}; background download policy: {background} |  |
| src-tauri/src/service/dispatch.rs:1334 | {error}; crash capture policy: {policy} |  |
| src-tauri/src/service/dispatch.rs:1499 | Spelling correction is disabled in private/read-only mode or preferences |  |
| src-tauri/src/service/dispatch.rs:1523 | reveal_book |  |
| src-tauri/src/service/dispatch.rs:1547 | Moving archives requires explicit confirmation |  |
| src-tauri/src/service/dispatch.rs:1550 | fromDirectory |  |
| src-tauri/src/service/dispatch.rs:1579 | Stop the integrity check before moving the library |  |
| src-tauri/src/service/dispatch.rs:1582 | Stop sharing before moving the library |  |
| src-tauri/src/service/dispatch.rs:1648 | Opening an attachment requires explicit confirmation |  |
| src-tauri/src/service/dispatch.rs:1660 | Invalid attachment export name |  |
| src-tauri/src/service/dispatch.rs:1662 | Opening the attachment in its default application: {e} |  |
| src-tauri/src/service/dispatch.rs:1709 | Cannot monitor portable data directory {directory}: {error} |  |
| src-tauri/src/service/dispatch.rs:1823 | Missing download cleanup installations |  |
| src-tauri/src/service/dispatch.rs:1824 | Too many archive cleanup claims |  |
| src-tauri/src/service/dispatch.rs:1827 | download |  |
| src-tauri/src/service/dispatch.rs:1827 | map |  |
| src-tauri/src/service/dispatch.rs:1827 | Unknown download cleanup source |  |
| src-tauri/src/service/dispatch.rs:1845 | This download has no file that can be unlinked here; remove its archive from the library first, then delete the download files |  |
| src-tauri/src/service/dispatch.rs:1851 | Archive installation changed while confirming cleanup |  |
| src-tauri/src/service/dispatch.rs:1857 | The installed archive list changed; close this dialog and try again |  |
| src-tauri/src/service/dispatch.rs:1878 | Creating a Wi-Fi hotspot is only available on Android |  |
| src-tauri/src/service/dispatch.rs:1880 | localswarm_hotspot_stop |  |
| src-tauri/src/service/dispatch.rs:1890 | Missing discovery enabled flag |  |
| src-tauri/src/service/dispatch.rs:1901 | shared_library_browse |  |
| src-tauri/src/service/dispatch.rs:2018 | NEARBY_LAN_REQUIRED |  |
| src-tauri/src/service/dispatch.rs:2028 | Nearby sharing ended before it was ready |  |
| src-tauri/src/service/dispatch.rs:2036 | [nearby] share all failed: {error} |  |
| src-tauri/src/service/dispatch.rs:2039 | localswarm_stop_library |  |
| src-tauri/src/service/dispatch.rs:2280 | Missing download limit |  |
| src-tauri/src/service/dispatch.rs:2282 | upload |  |
| src-tauri/src/service/dispatch.rs:2284 | Missing upload limit |  |
| src-tauri/src/service/dispatch.rs:2329 | Map settings changed; select the map again |  |
| src-tauri/src/service/dispatch.rs:2333 | Map choice expired after a privacy policy transition |  |
| src-tauri/src/service/dispatch.rs:2461 | status |  |
| src-tauri/src/service/dispatch.rs:2461 | rejected |  |
| src-tauri/src/service/dispatch.rs:2465 | batch |  |
| src-tauri/src/service/dispatch.rs:2465 | Missing archive storage batch |  |
| src-tauri/src/service/dispatch.rs:2489 | retired |  |
| src-tauri/src/service/dispatch.rs:2490 | privacyGeneration |  |
| src-tauri/src/service/dispatch.rs:2498 | sensor_activate |  |
| src-tauri/src/service/dispatch.rs:2508 | This application window has closed |  |
| src-tauri/src/service/dispatch.rs:2511 | {label}:{book} |  |
| src-tauri/src/service/dispatch.rs:2513 | Sensor origin is not an active archive listener for this window |  |
| src-tauri/src/service/dispatch.rs:2533 | Sensor installation is not an active archive listener for this window |  |
| src-tauri/src/service/dispatch.rs:2553 | sensor |  |
| src-tauri/src/service/dispatch.rs:2553 | Missing sensor |  |
| src-tauri/src/service/dispatch.rs:2555 | sensor_reset |  |
| src-tauri/src/service/dispatch.rs:2574 | Native location requires an active Apple application event loop; use the browser geolocation provider |  |
| src-tauri/src/service/dispatch.rs:2583 | Missing request ID |  |
| src-tauri/src/service/dispatch.rs:2584 | highAccuracy |  |
| src-tauri/src/service/dispatch.rs:2611 | Sharing is disabled in private mode |  |
| src-tauri/src/service/dispatch.rs:2613 | scope |  |
| src-tauri/src/service/dispatch.rs:2622 | Library sharing serves every available archive; do not select individual archives |  |
| src-tauri/src/service/dispatch.rs:2631 | Select at least one archive to share |  |
| src-tauri/src/service/dispatch.rs:2634 | binding |  |
| src-tauri/src/service/dispatch.rs:2650 | Invalid sharing port |  |
| src-tauri/src/service/dispatch.rs:2654 | Stop sharing before changing its selection or binding |  |
| src-tauri/src/service/dispatch.rs:2681 | Resource exceeds in-memory export limit; use the archive resource URL to save large media |  |
| src-tauri/src/service/dispatch.rs:2681 | article.html |  |
| src-tauri/src/service/dispatch.rs:2681 | fileName |  |
| src-tauri/src/service/dispatch.rs:2683 | save_resource |  |
| src-tauri/src/service/dispatch.rs:2683 | save_mhtml |  |
| src-tauri/src/service/dispatch.rs:2763 | Native windows require the desktop application |  |
| src-tauri/src/service/dispatch.rs:2764 | window-{} |  |
| src-tauri/src/service/dispatch.rs:2771 | At most 16 native windows can be open |  |
| src-tauri/src/service/dispatch.rs:2793 | Restore previous session is unavailable in private mode |  |
| src-tauri/src/service/dispatch.rs:2797 | Restore previous session is unavailable in a read-only profile |  |
| src-tauri/src/service/dispatch.rs:2801 | No saved session is available to restore |  |
| src-tauri/src/service/dispatch.rs:2804 | The saved session has no open tabs to restore |  |
| src-tauri/src/service/dispatch.rs:2840 | The saved session is already open |  |
| src-tauri/src/service/dispatch.rs:3000 | Debug crash trigger is disabled |  |
| src-tauri/src/service/dispatch.rs:3004 | Debug crash archive is not in the library |  |
| src-tauri/src/service/dispatch.rs:3005 | debug_crash |  |
| src-tauri/src/service/dispatch.rs:3016 | Unknown application command: {command} |  |
| src-tauri/src/service.rs:148 | Missing string argument: {key} |  |
| src-tauri/src/service.rs:375 | Too many zim:// links in one request |  |
| src-tauri/src/service.rs:383 | No archive in the library has ID {}; import it before opening this link |  |
| src-tauri/src/service.rs:389 | Archive “{}” is unavailable; locate it in the library before opening this link |  |
| src-tauri/src/service.rs:669 | Reopen the app to continue |  |
| src-tauri/src/service.rs:676 | This saved window is out of date. Close it and open a new window |  |
| src-tauri/src/service.rs:682 | Android window retirement failed: {error} |  |
| src-tauri/src/service.rs:683 | The previous Android window has not drained |  |
| src-tauri/src/service.rs:686 | Window state unavailable |  |
| src-tauri/src/service.rs:707 | This window is out of date. Close it and open a new window |  |
| src-tauri/src/service.rs:766 | Open Kiwix from the app window to continue |  |
| src-tauri/src/service.rs:814 | Too many search windows |  |
| src-tauri/src/service.rs:828 | This search is no longer available. Start a new search |  |
| src-tauri/src/service.rs:831 | search_owner |  |
| src-tauri/src/service.rs:832 | Too many search consumers |  |
| src-tauri/src/service.rs:835 | ownerId |  |
| src-tauri/src/service.rs:846 | Missing search request identity |  |
| src-tauri/src/service.rs:908 | The application is in strict read-only mode |  |
| src-tauri/src/service.rs:923 | Missing settings object |  |
| src-tauri/src/service.rs:924 | speechPreferences |  |
| src-tauri/src/service.rs:925 | Speech preferences require a language-level edit |  |
| src-tauri/src/service.rs:929 | Reader preferences expired after a privacy transition |  |
| src-tauri/src/service.rs:933 | This profile was opened with --read-only; restart without that flag to change settings |  |
| src-tauri/src/service.rs:939 | Leave read-only mode before changing settings |  |
| src-tauri/src/service.rs:942 | Settings are not an object |  |
| src-tauri/src/service.rs:950 | Enable completion notifications through the explicit permission action |  |
| src-tauri/src/service.rs:957 | This operation is disabled in private mode |  |
| src-tauri/src/service.rs:1025 | Return to the foreground before starting a LocalSwarm operation |  |
| src-tauri/src/service.rs:1041 | Nearby sharing was stopped. Select the share or resume the download again. |  |
| src-tauri/src/service.rs:1044 | Nearby sharing stopped. Turn it on again |  |
| src-tauri/src/service.rs:1090 | Invalid LocalSwarm port |  |
| src-tauri/src/service.rs:1092 | discoveryId |  |
| src-tauri/src/service.rs:1094 | The selected LocalSwarm peer is no longer discovered; start discovery and choose it again |  |
| src-tauri/src/service.rs:1095 | swarmId |  |
| src-tauri/src/service.rs:1163 | This file is outside Kiwix's library storage. Import it into your library, then open it |  |
| src-tauri/src/service.rs:1213 | Archive file identity changed; import it separately or locate the original archive |  |
| src-tauri/src/service.rs:1234 | Archives are being moved; try again when the move finishes |  |
| src-tauri/src/service.rs:1260 | Imported archive disappeared |  |
| src-tauri/src/service.rs:1656 | This file belongs to an archive edition decision. Review it in Downloads instead of importing it separately |  |
| src-tauri/src/service.rs:1664 | Another copy of this archive is already in your library. Open that copy, or remove it before adding this file |  |
| src-tauri/src/service.rs:1791 | Open the archive before changing restricted mode |  |
| src-tauri/src/service.rs:1793 | restricted |  |
| src-tauri/src/service.rs:1859 | Archive installation changed while opening the reader; open it again |  |
| src-tauri/src/service.rs:1910 | The application is shutting down |  |
| src-tauri/src/service.rs:2160 | localTransferState |  |
| src-tauri/src/service.rs:2161 | local-transfer-idle-release |  |
| src-tauri/src/service.rs:2290 | local-sharing-admission |  |
| src-tauri/src/service.rs:2314 | localswarm_status |  |
| src-tauri/src/service.rs:2314 | localswarm_inspect |  |
| src-tauri/src/service.rs:2314 | integrity_status |  |
| src-tauri/src/service.rs:2314 | crash_report |  |
| src-tauri/src/service.rs:2314 | crash_report_acknowledge |  |
| src-tauri/src/service.rs:2314 | crash_report_delete |  |
| src-tauri/src/service.rs:2343 | An acquisition must identify exactly one archive for verification |  |
| src-tauri/src/service.rs:2347 | Download ownership is unavailable; reopen Downloads and retry |  |
| src-tauri/src/service.rs:2351 | Invalid torrent publication receipt |  |
| src-tauri/src/service.rs:2444 | A download must have exactly one verified archive |  |
| src-tauri/src/service.rs:2446 | Download verification omitted its original publication authority | authority |
| src-tauri/src/service.rs:2467 | Nearby download ownership is unavailable; retry the import |  |
| src-tauri/src/service.rs:2518 | Map choice changed while waiting for its installation gate; retry verification with the current explicit choice |  |
| src-tauri/src/service.rs:2521 | A map receipt must contain exactly one verified archive |  |
| src-tauri/src/service.rs:2840 | transfer-idle-release |  |
| src-tauri/src/service.rs:2861 | Stopping archive verification: {error} |  |
| src-tauri/src/service.rs:2866 | Import manager unavailable |  |
| src-tauri/src/service.rs:2871 | Stopping archive imports: {error} |  |
| src-tauri/src/service.rs:2875 | Stopping spelling dictionary: {error} |  |
| src-tauri/src/service.rs:2879 | Sensor manager unavailable |  |
| src-tauri/src/service.rs:2886 | Pausing downloads: {error} |  |
| src-tauri/src/service.rs:2891 | Stopping LocalSwarm: {error} |  |
| src-tauri/src/service.rs:2900 | Pausing maps: {error} |  |
| src-tauri/src/service.rs:2905 | transfer-background-release |  |
| src-tauri/src/service.rs:2913 | Stopping torrents: {error} |  |
| src-tauri/src/service.rs:2923 | Stopping archive sharing: {error} |  |
| src-tauri/src/service.rs:2929 | Stopping archive readers: {error} |  |
| src-tauri/src/service.rs:2937 | Releasing session archive providers: {error} |  |
| src-tauri/src/service.rs:2941 | Stopping native transfer service: {error} |  |
| src-tauri/src/service.rs:2954 | Sharing over a Wi-Fi hotspot is only available on Android |  |
| src-tauri/src/service.rs:3071 | External links are disabled in Settings. This address has not been opened. |  |
| src-tauri/src/service.rs:3074 | Opening an external URL requires explicit confirmation |  |
| src-tauri/src/service.rs:3084 | External URL scheme or embedded credentials are not permitted |  |
| src-tauri/src/service.rs:3096 | Reading saved state: {e} |  |
| src-tauri/src/service.rs:3099 | Saved transfer state exceeds 16 MiB |  |
| src-tauri/src/service.rs:3102 | Invalid saved transfer state: {e} |  |
| src-tauri/src/service.rs:3109 | This saved transfer item cannot be read. Restart the app, then try again |  |
| src-tauri/src/share.rs:52 | Shared path is not UTF-8 |  |
| src-tauri/src/share.rs:53 | Invalid shared path |  |
| src-tauri/src/share.rs:66 | Sharing was cancelled |  |
| src-tauri/src/share.rs:81 | The archive is no longer in the library |  |
| src-tauri/src/share.rs:83 | The archive file is missing; relocate it before sharing |  |
| src-tauri/src/share.rs:89 | Only a regular archive file can be shared |  |
| src-tauri/src/share.rs:104 | The archive name is not UTF-8 |  |
| src-tauri/src/shared_library.rs:63 | Shared library access is unavailable in private or read-only mode |  |
| src-tauri/src/shared_library.rs:110 | Invalid shared library session |  |
| src-tauri/src/shared_library.rs:117 | Invalid shared library book |  |
| src-tauri/src/shared_library.rs:133 | Too many shared libraries are open; close discovery and try again |  |
| src-tauri/src/shared_library.rs:142 | Shared library did not respond |  |
| src-tauri/src/shared_library.rs:155 | Book is not in this shared library |  |
| src-tauri/src/sharing.rs:138 | Connect to a trusted local Wi-Fi or Ethernet network before sharing, or choose an advanced network binding |  |
| src-tauri/src/sharing.rs:171 | Sharing over a Wi-Fi hotspot requires the running application's hotspot reservation |  |
| src-tauri/src/sharing.rs:175 | The hotspot interface has no usable IPv4 address; restart the hotspot and try again |  |
| src-tauri/src/sharing.rs:185 | The selected IP address is not an available local interface; refresh interfaces and choose again |  |
| src-tauri/src/sharing.rs:273 | Sharing stopped |  |
| src-tauri/src/sharing.rs:422 | This selected archive is no longer shared |  |
| src-tauri/src/sharing.rs:432 | This shared file is no longer available. Stop sharing and start again |  |
| src-tauri/src/sharing.rs:528 | This archive file was replaced; import the new archive separately |  |
| src-tauri/src/sharing.rs:540 | This archive is no longer in the library |  |
| src-tauri/src/sharing.rs:544 | This archive is no longer available in the library |  |
| src-tauri/src/sharing.rs:549 | This archive was relinked or replaced; reopen it from the library index |  |
| src-tauri/src/sharing.rs:646 | Sharing has stopped |  |
| src-tauri/src/sharing.rs:738 | Joining library book listener: {error} |  |
| src-tauri/src/sharing.rs:772 | The sharing PIN must contain 4–12 digits |  |
| src-tauri/src/sharing.rs:835 | Joining sharing listener: {error} |  |
| src-tauri/src/sharing.rs:905 | One or more files you chose to share are missing. Choose the files again and start sharing |  |
| src-tauri/src/sharing.rs:1061 | Import an available archive before serving the library |  |
| src-tauri/src/sharing.rs:1244 | Article URL is too long |  |
| src-tauri/src/sharing.rs:1249 | Invalid article target |  |
| src-tauri/src/sharing.rs:1362 | Bootstrap body is too large |  |
| src-tauri/src/sharing.rs:1363 | Bootstrap body timed out |  |
| src-tauri/src/sharing.rs:1378 | Invalid bootstrap JSON |  |
| src-tauri/src/sharing.rs:1381 | Use JSON or a PIN form |  |
| src-tauri/src/sharing.rs:1479 | <!doctype html><html lang=\"{}\" dir=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style> |  |
| src-tauri/src/sharing.rs:1479 | Shared library · Kiwix |  |
| src-tauri/src/sharing.rs:1506 | Search is busy; try again shortly |  |
| src-tauri/src/sharing.rs:1604 | <p class=\"error\">{}</p> |  |
| src-tauri/src/sharing.rs:1605 | This archive could not be searched. |  |
| src-tauri/src/sharing.rs:1614 | Archive search worker stopped |  |
| src-tauri/src/sharing.rs:1910 | Catalog query is too long |  |
| src-tauri/src/sharing.rs:1920 | Invalid catalog filter |  |
| src-tauri/src/sharing.rs:1923 | q |  |
| src-tauri/src/sharing.rs:1933 | Unknown or repeated catalog parameter |  |
| src-tauri/src/sharing.rs:1945 | Catalog count must be between 1 and 200 |  |
| src-tauri/src/sharing.rs:1952 | Catalog filter is too long |  |
| src-tauri/src/sharing.rs:2246 | Archive file identity changed |  |
| src-tauri/src/sharing.rs:2250 | Archive source changed while opening |  |
| src-tauri/src/sharing.rs:2274 | Invalid archive identifier |  |
| src-tauri/src/sharing.rs:2304 | Archive is not available |  |
| src-tauri/src/sharing.rs:2360 | This archive is no longer shared by its library owner |  |
| src-tauri/src/sharing.rs:2372 | bytes */{size} |  |
| src-tauri/src/sharing.rs:2446 | /.kiwix-access/{} |  |
| src-tauri/src/sharing.rs:2474 | Invalid UTF-8 archive path |  |
| src-tauri/src/sharing_download.rs:23 | Archive parts do not match the admitted ZIM length |  |
| src-tauri/src/sharing_download.rs:31 | Archive cohort does not end at its declared ZIM checksum boundary |  |
| src-tauri/src/sharing_download.rs:34 | Archive changed while opening |  |
| src-tauri/src/sharing_download.rs:60 | Archive source identity changed |  |
| src-tauri/src/sharing_download.rs:65 | Archive part length changed |  |
| src-tauri/src/sharing_download.rs:68 | Invalid logical archive range |  |
| src-tauri/src/sharing_download.rs:74 | Archive source identity changed during read |  |
| src-tauri/src/sharing_download.rs:89 | Truncated ZIM cohort |  |
| src-tauri/src/sharing_reader.rs:23 | Archive is not shared |  |
| src-tauri/src/sharing_reader.rs:25 | Archive is no longer shared |  |
| src-tauri/src/sharing_reader.rs:54 | Search URL is too long |  |
| src-tauri/src/sharing_reader.rs:57 | mode |  |
| src-tauri/src/sharing_reader.rs:58 | Unknown or repeated search parameter |  |
| src-tauri/src/sharing_reader.rs:62 | Search must be at most 256 characters and 1024 bytes, without control characters |  |
| src-tauri/src/sharing_reader.rs:64 | Archive identifier is too long |  |
| src-tauri/src/sharing_reader.rs:66 | Invalid search offset |  |
| src-tauri/src/sharing_reader.rs:68 | Invalid search continuation |  |
| src-tauri/src/sharing_reader.rs:120 | <!doctype html><html lang=\"{}\" dir=\"{}\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>:root{{color-scheme:light;--ink:#183b35;--paper:#f6f2e8}}body{{max-width:900px;margin:auto;padding:2rem 1.5rem;background:var(--paper);color:var(--ink);font:18px/1.6 Georgia,serif}}h1{{border-top:8px solid var(--ink);padding-top:1rem;font-size:3rem;line-height:1.1}}a{{color:var(--ink)}}form{{display:flex;gap:1rem;flex-wrap:wrap;border-block:1px solid #c3cdc1;padding:1rem 0}}label{{display:flex;flex-direction:column}}input,select,button{{font:inherit;padding:.5rem;max-width:100%}}button{{cursor:pointer}}li{{padding:.5rem 0}}small{{display:block}}[hidden]{{display:none}}</style><a href=\"/{}/\">{}</a><h1>{}</h1><form id=\"search\" role=\"search\"><label>{}<input id=\"q\" name=\"q\" type=\"search\" maxlength=\"256\" autocomplete=\"off\" aria-controls=\"suggestions\" required></label><label>{}<select id=\"book\"><option value=\"\">{}</option> |  |
| src-tauri/src/sharing_reader.rs:120 | Search articles |  |
| src-tauri/src/sharing_reader.rs:120 | Browse the collection |  |
| src-tauri/src/sharing_reader.rs:120 | Search |  |
| src-tauri/src/sharing_reader.rs:120 | Archive |  |
| src-tauri/src/sharing_reader.rs:120 | All shared items |  |
| src-tauri/src/sharing_reader.rs:121 | <option value=\"{}\">{}</option> |  |
| src-tauri/src/sharing_reader.rs:122 | </select></label><label>{}<select id=\"mode\"><option value=\"combined\">{}</option><option value=\"title\">{}</option><option value=\"fulltext\">{}</option></select></label><button>{}</button></form><ul id=\"suggestions\" aria-label=\"{}\"></ul><p id=\"status\" role=\"status\" aria-live=\"polite\"></p><ol id=\"results\"></ol><button id=\"more\" hidden>{}</button><script src=\"/{}/reader.js\" defer></script></html> |  |
| src-tauri/src/sharing_reader.rs:122 | Search mode |  |
| src-tauri/src/sharing_reader.rs:122 | Titles and full text |  |
| src-tauri/src/sharing_reader.rs:122 | Titles |  |
| src-tauri/src/sharing_reader.rs:122 | Full text |  |
| src-tauri/src/sharing_reader.rs:122 | Title suggestions |  |
| src-tauri/src/sharing_reader.rs:122 | More results |  |
| src-tauri/src/spelling.rs:183 | Invalid or oversized spelling cache file |  |
| src-tauri/src/spelling.rs:270 | Spelling builds are disabled in private or read-only mode |  |
| src-tauri/src/spelling.rs:279 | Spelling job identity changed |  |
| src-tauri/src/spelling.rs:281 | Another spelling dictionary is already building |  |
| src-tauri/src/spelling.rs:310 | Spelling load identity changed |  |
| src-tauri/src/spelling.rs:389 | Spelling cache loading cannot be cancelled |  |
| src-tauri/src/spelling.rs:407 | No spelling job exists for this archive |  |
| src-tauri/src/spelling.rs:454 | Spelling build cancelled |  |
| src-tauri/src/spelling.rs:612 | Invalid, changing, or oversized spelling title listing (maximum 20 million titles) |  |
| src-tauri/src/spelling.rs:618 | Spelling dictionary exceeds 2 million distinct words |  |
| src-tauri/src/spelling.rs:663 | Spelling cache publication disabled by privacy policy |  |
| src-tauri/src/spelling.rs:666 | {key}.sqlite |  |
| src-tauri/src/spelling.rs:717 | Invalid or oversized spelling cache |  |
| src-tauri/src/spelling.rs:724 | Spelling cache integrity verification failed |  |
| src-tauri/src/spelling.rs:726 | SELECT         (SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('words','deletes','identity'))=3         AND (SELECT group_concat(name,',') FROM (SELECT name FROM pragma_index_info('word_lengths') ORDER BY seqno))='length,word'         AND EXISTS(SELECT 1 FROM pragma_table_info('words') WHERE name='word' AND pk=1)         AND EXISTS(SELECT 1 FROM pragma_table_info('deletes') WHERE name='key' AND pk=1)         AND EXISTS(SELECT 1 FROM pragma_table_info('deletes') WHERE name='word' AND pk=2) |  |
| src-tauri/src/spelling.rs:734 | The spelling dictionary is damaged. Rebuild it in Settings |  |
| src-tauri/src/spelling.rs:748 | Spelling cache archive identity/version or bounds mismatch |  |
| src-tauri/src/spelling.rs:751 | SELECT count(*) FROM words |  |
| src-tauri/src/spelling.rs:754 | Incomplete spelling cache dictionary |  |
| src-tauri/src/spelling.rs:811 | Spelling candidate limit exceeded |  |
| src-tauri/src/spelling.rs:815 | Invalid spelling dictionary word |  |
| src-tauri/src/storage.rs:14 | Directory path must be normalized and absolute |  |
| src-tauri/src/storage.rs:23 | Native iOS container identity changed |  |
| src-tauri/src/storage.rs:26 | Directory {} is outside native iOS container {} |  |
| src-tauri/src/storage.rs:44 | Directory path contains a non-normal component |  |
| src-tauri/src/storage.rs:74 | Resolved directory path is not terminated |  |
| src-tauri/src/storage.rs:154 | Invalid native iOS container capability |  |
| src-tauri/src/storage.rs:169 | Directory capability does not refer to a directory |  |
| src-tauri/src/storage.rs:179 | Readable directory descriptor changed capability identity |  |
| src-tauri/src/storage.rs:184 | Directory capability changed identity during synchronization |  |
| src-tauri/src/storage.rs:248 | Capability rename target must be a single filename |  |
| src-tauri/src/storage.rs:413 | {} uses FAT/FAT32, which cannot store one file larger than 4 GiB − 1 byte. Choose an exFAT, NTFS, APFS or other large-file-capable download directory. |  |
| src-tauri/src/storage.rs:442 | Not a download directory: {} |  |
| src-tauri/src/storage.rs:446 | Inspecting download filesystem {}: {e} |  |
| src-tauri/src/storage.rs:452 | Download directory has no accessible filesystem ancestor |  |
| src-tauri/src/storage.rs:464 | Inspecting download filesystem: {} |  |
| src-tauri/src/store.rs:101 | Library entry in shell payload is not an object |  |
| src-tauri/src/store.rs:102 | mainPath |  |
| src-tauri/src/store.rs:104 | Library entry in shell payload is missing {key} |  |
| src-tauri/src/store.rs:208 | The highlight no longer exists |  |
| src-tauri/src/store.rs:317 | Unknown window surface |  |
| src-tauri/src/store.rs:355 | Unsupported saved-session version |  |
| src-tauri/src/store.rs:358 | The saved session has too many windows |  |
| src-tauri/src/store.rs:361 | The saved session has no main window |  |
| src-tauri/src/store.rs:369 | The saved session names an unknown window |  |
| src-tauri/src/store.rs:396 | Library filter selections are invalid or too large |  |
| src-tauri/src/store.rs:558 | A speech edit requires a language and an edited voice or speed |  |
| src-tauri/src/store.rs:560 | * |  |
| src-tauri/src/store.rs:570 | Sharing name must be at most 160 UTF-8 bytes, without surrounding whitespace, controls or direction overrides |  |
| src-tauri/src/store.rs:605 | Invalid {name}: {value:?} |  |
| src-tauri/src/store.rs:614 | {name} must be a finite number from {min} to {max} |  |
| src-tauri/src/store.rs:621 | Too many speech language preferences |  |
| src-tauri/src/store.rs:629 | Speech preferences require a language, an installed voice identifier and a speed from 0.25 to 2 |  |
| src-tauri/src/store.rs:639 | Book zoom requires an archive identity and a value from 25% to 500% |  |
| src-tauri/src/store.rs:647 | locale must be a nonempty language tag of at most 128 bytes |  |
| src-tauri/src/store.rs:665 | Catalog/proxy URLs must be HTTP(S) URLs without embedded credentials |  |
| src-tauri/src/store.rs:672 | Directory paths cannot contain NUL bytes |  |
| src-tauri/src/store.rs:680 | Remembered picker folders must be absolute paths of at most 32768 bytes without NUL |  |
| src-tauri/src/store.rs:759 | This profile was saved by a different version of Kiwix (version {}; this app reads version {}). Open it with that version, or choose another profile. Your saved file was not changed. |  |
| src-tauri/src/store.rs:767 | Recent searches exceed the supported history bounds |  |
| src-tauri/src/store.rs:791 | A saved library entry is damaged. Remove that archive from your library, then add it again |  |
| src-tauri/src/store.rs:795 | Book path must be nonempty and contain no NUL |  |
| src-tauri/src/store.rs:814 | A saved {kind} item could not be read and was skipped. |  |
| src-tauri/src/store.rs:855 | Book identity is invalid |  |
| src-tauri/src/store.rs:877 | Tab identity is invalid |  |
| src-tauri/src/store.rs:881 | Tab paths cannot contain NUL |  |
| src-tauri/src/store.rs:897 | State file must be a JSON object |  |
| src-tauri/src/store.rs:903 | State file is missing its version |  |
| src-tauri/src/store.rs:906 | This profile was saved by a different version of Kiwix (version {version}; this app reads version {}). Open it with that version, or choose another profile. Your saved file was not changed. |  |
| src-tauri/src/store.rs:945 | Duplicate book identity |  |
| src-tauri/src/store.rs:948 | Duplicate book installation |  |
| src-tauri/src/store.rs:957 | Duplicate bookmark identity |  |
| src-tauri/src/store.rs:964 | The saved reading list is damaged. Its causal sync history was preserved; restore or repair this profile before syncing so recovery cannot become a deletion |  |
| src-tauri/src/store.rs:970 | History identity is invalid |  |
| src-tauri/src/store.rs:974 | Duplicate history identity |  |
| src-tauri/src/store.rs:1006 | Duplicate tab identity |  |
| src-tauri/src/store.rs:1133 | Invalid saved reference revision |  |
| src-tauri/src/store.rs:1161 | activeTab must identify an existing tab in its window |  |
| src-tauri/src/store.rs:1171 | Invalid or duplicate {kind} ID {id:?} |  |
| src-tauri/src/store.rs:1178 | Scroll position must be a finite nonnegative number |  |
| src-tauri/src/store.rs:1184 | Article identity requires a nonempty book ID and a path without NUL |  |
| src-tauri/src/store.rs:1190 | A saved highlight is damaged. Remove the highlight, then add it again |  |
| src-tauri/src/store.rs:1197 | A bookmark may contain at most 200 highlights |  |
| src-tauri/src/store.rs:1203 | A bookmark contains the same highlight twice. Remove one and save again |  |
| src-tauri/src/store.rs:1212 | Highlight text exceeds its allowed bounds |  |
| src-tauri/src/store.rs:1219 | Highlight offsets must match the UTF-16 quote length |  |
| src-tauri/src/store.rs:1233 | Highlight documentPath must be a bounded archive-relative location |  |
| src-tauri/src/store.rs:1244 | Bookmark ID must be nonempty and contain no control characters |  |
| src-tauri/src/store.rs:1248 | Invalid bookmark createdAt {:?}; expected RFC3339 |  |
| src-tauri/src/store.rs:1481 | Portable data path is not Unicode |  |
| src-tauri/src/store.rs:1628 | Cannot open {}: read-only startup requires an existing initialized profile |  |
| src-tauri/src/store.rs:1705 | Saved settings are not a JSON object |  |
| src-tauri/src/store.rs:1708 | activeTab |  |
| src-tauri/src/store.rs:1734 | State exceeds 128 MiB; transaction was not committed |  |
| src-tauri/src/store.rs:1818 | Portable mode requires the adjacent data directory; conflicting --data-dir is not permitted |  |
| src-tauri/src/store.rs:1862 | {} exceeds the 128 MiB state limit; preserve the file and export/reduce it with a compatible application |  |
| src-tauri/src/store.rs:1864 | Cannot parse {}: {error}. The original is untouched; restore a known-good backup or select another profile, not an empty replacement. |  |
| src-tauri/src/store.rs:1898 | Cannot open {}: {e} |  |
| src-tauri/src/store.rs:1983 | This profile was opened read-only; state changes are disabled |  |
| src-tauri/src/store.rs:2037 | Portable mode fixes download and monitored folders to the adjacent data directory |  |
| src-tauri/src/store.rs:2055 | Portable downloads must use the adjacent data directory |  |
| src-tauri/src/store.rs:2085 | Profile no longer identifies this application's retained process lock |  |
| src-tauri/src/store.rs:2197 | Cannot open {}: {error} |  |
| src-tauri/src/store.rs:2200 | The saved session exceeds the supported size |  |
| src-tauri/src/store.rs:2278 | Acquisition destination must be an absolute verified path |  |
| src-tauri/src/store.rs:2283 | Acquisition destination must be valid Unicode without NUL |  |
| src-tauri/src/store.rs:2291 | Cannot resolve legacy library paths outside a Unicode working directory |  |
| src-tauri/src/store.rs:2294 | Leave read-only mode before publishing an acquisition |  |
| src-tauri/src/store.rs:2297 | validated Unicode destination |  |
| src-tauri/src/store.rs:2353 | Read-only mode prevents saving files |  |
| src-tauri/src/store.rs:2358 | The archive installation changed or is unavailable; open it again |  |
| src-tauri/src/store.rs:2366 | Provider source required |  |
| src-tauri/src/store.rs:2368 | This archive can only be used for the current session. Reopen it from its original app |  |
| src-tauri/src/store.rs:2372 | Transient provider admission requires private or read-only mode |  |
| src-tauri/src/store.rs:2374 | Close an external archive before opening another (32 session archives maximum) |  |
| src-tauri/src/store.rs:2378 | This archive or installation already exists in the library |  |
| src-tauri/src/store.rs:2399 | Transient provider replacement requires private or read-only mode |  |
| src-tauri/src/store.rs:2403 | The session archive changed or is no longer available |  |
| src-tauri/src/store.rs:2412 | Replacement must create a unique new archive installation |  |
| src-tauri/src/store.rs:2537 | Private reading sync is unavailable in private or read-only mode |  |
| src-tauri/src/store.rs:2550 | Leave read-only mode before changing this profile |  |
| src-tauri/src/store.rs:2617 | Session-only provider references cannot be saved to the profile |  |
| src-tauri/src/store.rs:2632 | Session-only external archive activity cannot be saved to the profile |  |
| src-tauri/src/store.rs:2741 | Cannot inspect saved policy in {}: {error} |  |
| src-tauri/src/store.rs:2749 | {} must be a bounded regular non-symlink file |  |
| src-tauri/src/store.rs:2768 | {} must be a regular file |  |
| src-tauri/src/store.rs:2776 | {} exceeds the 128 MiB state limit; the original is untouched |  |
| src-tauri/src/store.rs:2986 | The saved session exceeds the supported size of {MAX_SESSION_BYTES} bytes. The previous saved session was retained. |  |
| src-tauri/src/torrent/manager.rs:12 | Torrent download no longer exists |  |
| src-tauri/src/torrent/manager.rs:26 | Torrent publication changed during verification; retry the import |  |
| src-tauri/src/torrent/manager.rs:64 | This archive is already being shared |  |
| src-tauri/src/torrent/manager.rs:94 | Unsupported torrent monitor history version |  |
| src-tauri/src/torrent/manager.rs:120 | Unsupported torrent history version |  |
| src-tauri/src/torrent/manager.rs:161 | Missing torrent filename |  |
| src-tauri/src/torrent/manager.rs:262 | Choose an existing download directory |  |
| src-tauri/src/torrent/manager.rs:265 | .kiwix-torrent-{suffix} |  |
| src-tauri/src/torrent/manager.rs:336 | The selected archive is unavailable; reconnect it before sharing |  |
| src-tauri/src/torrent/manager.rs:340 | Only a single complete .zim archive can be shared |  |
| src-tauri/src/torrent/manager.rs:343 | This file name cannot be shared. Rename the file and try again |  |
| src-tauri/src/torrent/manager.rs:349 | Split archives cannot be shared as a single torrent |  |
| src-tauri/src/torrent/manager.rs:351 | Archive has no primary file |  |
| src-tauri/src/torrent/manager.rs:353 | Only the complete .zim archive can be shared |  |
| src-tauri/src/torrent/manager.rs:363 | {suffix}.torrent |  |
| src-tauri/src/torrent/manager.rs:453 | Torrent metadata is no longer available |  |
| src-tauri/src/torrent/manager.rs:484 | Torrent was cancelled |  |
| src-tauri/src/torrent/manager.rs:506 | Torrent worker failed before acknowledging its pause |  |
| src-tauri/src/torrent/manager.rs:526 | Torrent was removed from history or file cleanup has started; retry deletion instead |  |
| src-tauri/src/torrent/manager.rs:528 | Start a new job to acquire a cancelled torrent |  |
| src-tauri/src/torrent/manager.rs:583 | The transfer did not stop cleanly. Restart the app, then retry |  |
| src-tauri/src/torrent/manager.rs:586 | Torrent changed while its history was being removed |  |
| src-tauri/src/torrent/manager.rs:609 | The transfer did not stop cleanly, so Kiwix did not delete any files. Restart the app, then retry |  |
| src-tauri/src/torrent/manager.rs:613 | Only finished torrents can authorize cleanup |  |
| src-tauri/src/torrent/manager.rs:616 | seeding |  |
| src-tauri/src/torrent/manager.rs:618 | The transfer is still stopping. Wait a moment, then try again |  |
| src-tauri/src/torrent/manager.rs:634 | Torrent changed during cleanup authorization; retry with its current state |  |
| src-tauri/src/torrent/manager.rs:644 | The transfer did not stop cleanly. No files were deleted. Restart Kiwix, then try again |  |
| src-tauri/src/torrent/manager.rs:661 | Only finished, non-seeding torrents can be removed from history |  |
| src-tauri/src/torrent/manager.rs:669 | The transfer is already stopping. Wait a moment |  |
| src-tauri/src/torrent/manager.rs:709 | Unknown torrent job |  |
| src-tauri/src/torrent/manager.rs:726 | Missing archive filename |  |
| src-tauri/src/torrent/manager.rs:735 | Missing torrent archive filename |  |
| src-tauri/src/torrent/manager.rs:741 | Published archive has no ownership intent |  |
| src-tauri/src/torrent/manager.rs:743 | Publication cleanup journal does not match the original download |  |
| src-tauri/src/torrent/manager.rs:745 | Missing publication size |  |
| src-tauri/src/torrent/manager.rs:763 | Cannot prove ownership of legacy torrent staging; files and history were preserved |  |
| src-tauri/src/torrent/manager.rs:765 | Torrent staging directory was replaced; files and history were preserved |  |
| src-tauri/src/torrent/manager.rs:790 | Torrent staging subdirectory was replaced; directory preserved |  |
| src-tauri/src/torrent/manager.rs:795 | Staging has no parent |  |
| src-tauri/src/torrent/manager.rs:797 | Staging has no filename |  |
| src-tauri/src/torrent/manager.rs:797 | Removing owned empty torrent staging (unknown files are preserved): {error} |  |
| src-tauri/src/torrent/manager.rs:807 | Torrent staging is no longer an owned directory; entry and history were preserved |  |
| src-tauri/src/torrent/manager.rs:817 | Cannot prove ownership of legacy torrent metadata; file and history were preserved |  |
| src-tauri/src/torrent/manager.rs:849 | Torrent was removed from history or file cleanup has started |  |
| src-tauri/src/torrent/manager.rs:876 | Torrent manager has shut down |  |
| src-tauri/src/torrent/manager.rs:916 | Torrent worker failed during suspension: {error} |  |
| src-tauri/src/torrent/manager.rs:1031 | Torrent worker failed during shutdown: {error} |  |
| src-tauri/src/torrent/manager.rs:1047 | Torrent manager is suspended |  |
| src-tauri/src/torrent/manager.rs:1154 | The transfer did not stop cleanly. Restart Kiwix, then resume |  |
| src-tauri/src/torrent/manager.rs:1163 | Torrent worker is still retiring |  |
| src-tauri/src/torrent/manager.rs:1232 | The transfer stopped unexpectedly |  |
| src-tauri/src/torrent/manager.rs:1298 | The previous transfer of this torrent is still stopping. Try again in a moment |  |
| src-tauri/src/torrent/manager.rs:1426 | LOW_DISK_SPACE: |  |
| src-tauri/src/torrent/manager.rs:1603 | This torrent is already managed by another acquisition job |  |
| src-tauri/src/torrent/manager.rs:1606 | Torrent metadata exceeds 16 MiB |  |
| src-tauri/src/torrent/manager.rs:1615 | Torrent symlink entries are not permitted |  |
| src-tauri/src/torrent/manager.rs:1620 | Torrent contains duplicate paths |  |
| src-tauri/src/torrent/manager.rs:1631 | Torrent file has no staging directory |  |
| src-tauri/src/torrent/manager.rs:1634 | Cannot stage {}: {e} |  |
| src-tauri/src/torrent/manager.rs:1642 | Torrent size overflow |  |
| src-tauri/src/torrent/manager.rs:1647 | Torrent has an invalid number of files |  |
| src-tauri/src/torrent/manager.rs:1651 | Torrent does not contain a complete .zim archive |  |
| src-tauri/src/torrent/manager.rs:1654 | Torrent metadata changed since this job was created |  |
| src-tauri/src/torrent/manager.rs:1659 | Torrent archives have colliding destination filenames |  |
| src-tauri/src/torrent/manager.rs:1915 | This file cannot be shared because it has no name. Rename it and try again |  |
| src-tauri/src/torrent/manager.rs:1935 | Created torrent info hash is inconsistent |  |
| src-tauri/src/torrent/manager.rs:1959 | Torrent metadata changed since this share was created |  |
| src-tauri/src/torrent/manager.rs:1963 | Torrent metadata does not describe the selected archive |  |
| src-tauri/src/torrent/manager.rs:1994 | This shared file is no longer where Kiwix last saw it. Add it to the library again |  |
| src-tauri/src/torrent/manager.rs:2043 | The shared file changed, so sharing stopped. Start sharing again |  |
| src-tauri/src/torrent/manager.rs:2117 | The archive was removed from the library |  |
| src-tauri/src/torrent/manager.rs:2122 | The library archive changed or was relinked after selection |  |
| src-tauri/src/torrent/manager.rs:2126 | The library archive changed after selection |  |
| src-tauri/src/torrent/manager.rs:2133 | The share was removed from history |  |
| src-tauri/src/torrent/manager.rs:2139 | This file is no longer in your library, so sharing stopped |  |
| src-tauri/src/torrent/manager.rs:2144 | The library archive changed or was relinked |  |
| src-tauri/src/torrent/manager.rs:2147 | The library archive identity or size changed |  |
| src-tauri/src/torrent/publication.rs:11 | This torrent publication is no longer available for import |  |
| src-tauri/src/torrent/publication.rs:14 | Torrent publication has no filename |  |
| src-tauri/src/torrent/publication.rs:19 | Torrent import path does not match its published archive |  |
| src-tauri/src/torrent/publication.rs:22 | Kiwix cannot verify this torrent's original publication. Add the archive to your library manually |  |
| src-tauri/src/torrent/publication.rs:23 | Torrent publication ownership is ambiguous |  |
| src-tauri/src/torrent/publication.rs:35 | Unsupported torrent backup evidence version |  |
| src-tauri/src/torrent/publication.rs:39 | Missing completed torrent archive filename |  |
| src-tauri/src/torrent/publication.rs:78 | Torrent publication cancelled; staging was retained |  |
| src-tauri/src/torrent/publication.rs:129 | Cannot publish {final_path}: an unowned destination already exists |  |
| src-tauri/src/torrent/publication.rs:137 | Missing engine-verified torrent publication source |  |
| src-tauri/src/torrent/publication.rs:155 | {} is not a ZIM archive |  |
| src-tauri/src/torrent/publication.rs:158 | .kiwix-publish-{} |  |
| src-tauri/src/torrent/publication.rs:170 | {error}; temporary cleanup: {cleanup} |  |
| src-tauri/src/torrent/publication.rs:200 | Torrent publication cancelled |  |
| src-tauri/src/torrent/publication.rs:245 | Invalid torrent temporary name |  |
| src-tauri/src/torrent/publication.rs:248 | A saved transfer is damaged. Download the file again |  |
| src-tauri/src/torrent/publication.rs:269 | Torrent publication directory was replaced; files were preserved |  |
| src-tauri/src/torrent/publication.rs:287 | Torrent publication requires directories owned by the effective user without group/other write access; this destination permits unsupported mutation risk |  |
| src-tauri/src/torrent/publication.rs:389 | Torrent temporary capture directory is missing; file preserved |  |
| src-tauri/src/torrent/publication.rs:395 | Torrent temporary capture directory was replaced; files preserved |  |
| src-tauri/src/torrent/publication.rs:397 | payload |  |
| src-tauri/src/torrent/publication.rs:422 | Torrent temporary was replaced; unrelated entry preserved{} |  |
| src-tauri/src/torrent/publication.rs:434 | Torrent temporary capture directory changed; files preserved |  |
| src-tauri/src/torrent/publication.rs:442 | Captured download no longer matches its original publication bytes; file preserved{} |  |
| src-tauri/src/torrent/publication.rs:443 | in {}: {error} |  |
| src-tauri/src/torrent/publication.rs:580 | Cannot safely recover torrent publication: {error} |  |
| src-tauri/src/torrent/publication.rs:592 | Torrent publication destination is not the intended copied file; it was not changed |  |
| src-tauri/src/torrent/publication.rs:599 | Torrent publication destination does not match its durable SHA-256 intent; it was not changed |  |
| src-tauri/src/torrent/publication.rs:608 | Torrent publication destination was replaced during recovery; it was not changed |  |
| src-tauri/src/torrent/publication.rs:636 | The downloaded file changed while Kiwix was checking it. Download it again |  |
| src-tauri/src/torrent/publication.rs:643 | Torrent publication file was truncated |  |
| src-tauri/src/torrent/storage.rs:35 | Torrent storage is not a singly linked regular file |  |
| src-tauri/src/torrent/storage.rs:52 | Torrent file path or advertised length changed |  |
| src-tauri/src/torrent/storage.rs:87 | Symlink in torrent storage |  |
| src-tauri/src/torrent/storage.rs:121 | Owned torrent staging file was replaced; refusing to resume writes |  |
| src-tauri/src/torrent/storage.rs:180 | This file contains a shortcut and cannot be shared. Download it again |  |
| src-tauri/src/torrent/storage.rs:185 | Torrent paths changed or collide on a case-insensitive filesystem |  |
| src-tauri/src/torrent/storage.rs:202 | Sharing stopped because the app is closing |  |
| src-tauri/src/torrent/storage.rs:207 | Unknown torrent file |  |
| src-tauri/src/torrent/storage.rs:231 | Nonzero torrent padding |  |
| src-tauri/src/torrent/storage.rs:265 | Engine file deletion is disabled |  |
| src-tauri/src/torrent/storage.rs:268 | Engine directory deletion is disabled |  |
| src-tauri/src/torrent/storage.rs:326 | This file cannot be shared because its contents are not valid. Download it again |  |
| src-tauri/src/torrent/storage.rs:334 | Torrent metadata does not describe the selected library archive |  |
| src-tauri/src/torrent/storage.rs:339 | This file cannot be shared because it has no content. Download it again |  |
| src-tauri/src/torrent/storage.rs:356 | Kiwix could not open the shared file. Add it to the library again |  |
| src-tauri/src/torrent/storage.rs:359 | The shared file changed while sharing started. Start sharing again |  |
| src-tauri/src/torrent/storage.rs:393 | Shared library archives are read-only |  |
| src-tauri/src/torrent/storage.rs:417 | The shared library archive no longer matches its torrent |  |
| src-tauri/src/torrent/storage.rs:498 | Skipping LocalSwarm partial {}: {error} |  |
| src-tauri/src/torrent/storage.rs:518 | Partial range is outside the file |  |
| src-tauri/src/torrent.rs:187 | This file cannot be shared because it is not a normal file. Copy it into your library and try again |  |
| src-tauri/src/torrent.rs:304 | Invalid published torrent filename |  |
| src-tauri/src/torrent.rs:305 | Legacy torrent publication lacks original directory ownership proof; library deletion was not authorized |  |
| src-tauri/src/torrent.rs:357 | Cleanup file has no parent |  |
| src-tauri/src/torrent.rs:358 | Invalid cleanup filename |  |
| src-tauri/src/torrent.rs:428 | Owned download file has no parent |  |
| src-tauri/src/torrent.rs:441 | Owned download filename does not match its journal |  |
| src-tauri/src/torrent.rs:445 | Owned download directory was replaced; cleanup was not authorized |  |
| src-tauri/src/torrent.rs:455 | Download file is not its originally owned inode; cleanup was not authorized |  |
| src-tauri/src/torrent.rs:459 | Owned download file changed during provenance validation |  |
| src-tauri/src/torrent.rs:466 | Owned cleanup file has no parent |  |
| src-tauri/src/torrent.rs:473 | Cleanup directory was replaced; no files were authorized for deletion |  |
| src-tauri/src/torrent.rs:477 | Owned cleanup entry disappeared during validation |  |
| src-tauri/src/torrent.rs:484 | .kiwix-publish-cleanup- |  |
| src-tauri/src/torrent.rs:484 | Invalid cleanup capture name |  |
| src-tauri/src/torrent.rs:488 | Invalid cleanup capture ownership journal |  |
| src-tauri/src/torrent.rs:493 | Cannot validate owned cleanup capture: {error} |  |
| src-tauri/src/torrent.rs:496 | Cleanup capture directory was replaced; files were preserved |  |
| src-tauri/src/torrent.rs:501 | An unexpected file appeared in a temporary folder. Kiwix left it in place; restart the app |  |
| src-tauri/src/torrent.rs:507 | Cleanup capture contains a replaced payload |  |
| src-tauri/src/torrent.rs:511 | Captured publication no longer matches its original bytes |  |
| src-tauri/src/torrent.rs:532 | The downloaded file changed since it was saved. Remove it from the library and delete the file yourself |  |
| src-tauri/src/torrent.rs:537 | Invalid download cleanup capture |  |
| src-tauri/src/torrent.rs:541 | Invalid download cleanup ownership journal |  |
| src-tauri/src/torrent.rs:576 | Missing new cleanup capture |  |
| src-tauri/src/torrent.rs:580 | New cleanup capture was replaced; unrelated directory preserved |  |
| src-tauri/src/torrent.rs:592 | {error}{}{} |  |
| src-tauri/src/torrent.rs:593 | ; removing empty owned capture: {error} |  |
| src-tauri/src/torrent.rs:594 | ; restoring ownership checkpoint: {error} |  |
| src-tauri/src/torrent.rs:598 | Missing cleanup capture |  |
| src-tauri/src/torrent.rs:646 | Invalid torrent job identifier |  |
| src-tauri/src/torrent.rs:718 | Saved transfer history is too large to read. Clear it from Troubleshooting or start with a new profile |  |
| src-tauri/src/torrent.rs:721 | Invalid saved transfer history JSON: {error} |  |
| src-tauri/src/torrent.rs:731 | Invalid saved transfer history: expected an object |  |
| src-tauri/src/torrent.rs:734 | version |  |
| src-tauri/src/torrent.rs:736 | Invalid saved transfer history: missing version |  |
| src-tauri/src/torrent.rs:738 | Saved transfer history was written by a different version of Kiwix ({version}). Open it with that version, or start with a new profile |  |
| src-tauri/src/torrent.rs:746 | A saved transfer could not be read and was skipped. |  |
| src-tauri/src/torrent.rs:759 | Some saved transfers could not be read and were skipped. |  |
| src-tauri/src/torrent.rs:783 | The same transfer appears twice in saved history. Kiwix skipped the duplicate |  |
| src-tauri/src/torrent.rs:801 | A saved transfer is damaged. Kiwix set it aside; download the file again if it is missing |  |
| src-tauri/src/torrent.rs:826 | A saved shared file entry is damaged. Kiwix set it aside; share the file again if it is missing |  |
| src-tauri/src/torrent.rs:833 | Invalid torrent share fields |  |
| src-tauri/src/torrent.rs:844 | A saved transfer detail is damaged. Kiwix set it aside; download the file again if it is missing |  |
| src-tauri/src/torrent.rs:858 | Invalid torrent publication intent |  |
| src-tauri/src/torrent.rs:865 | The same temporary transfer appears twice in saved history. Kiwix skipped the duplicate |  |
| src-tauri/src/torrent.rs:903 | Cannot set aside unreadable transfer history {}: {rename_error} |  |
| src-tauri/src/torrent.rs:910 | Kiwix could not move a damaged saved transfer file aside. Restart the app; if this keeps happening, free some storage |  |
| src-tauri/src/torrent.rs:950 | Torrent publication size overflow |  |
| src-tauri/src/torrent.rs:973 | Torrent capacity requirement overflow |  |
| src-tauri/src/torrent.rs:1149 | Torrent bandwidth must be zero (unlimited) or 16384–4294967295 bytes/second |  |
| src-tauri/src/torrent.rs:1164 | Torrent URLs cannot contain credentials |  |
| src-tauri/src/torrent.rs:1169 | Magnet must contain a BitTorrent v1 info hash |  |
| src-tauri/src/torrent.rs:1174 | Torrent metadata requires HTTPS (literal loopback HTTP is allowed for local testing), or a magnet URI |  |
| src-tauri/src/torrent.rs:1185 | Too many torrent metadata redirects |  |
| src-tauri/src/torrent.rs:1224 | Torrent metadata is missing |  |
| src-tauri/src/torrent.rs:1226 | Torrent metadata has no parent directory |  |
| src-tauri/src/torrent.rs:1229 | Invalid torrent metadata filename |  |
| src-tauri/src/torrent.rs:1235 | Torrent metadata is not a regular file of at most 16 MiB |  |
| src-tauri/src/torrent.rs:1247 | Torrent metadata no longer matches its complete durable publication |  |
| src-tauri/src/torrent.rs:1262 | Torrent metadata is empty or exceeds 16 MiB |  |
| src-tauri/src/torrent.rs:1277 | {error}; metadata temporary cleanup: {cleanup} |  |
| src-tauri/src/torrent.rs:1320 | Torrent metadata publication ownership does not match its temporary |  |
| src-tauri/src/torrent.rs:1324 | Published torrent metadata no longer matches its complete durable bytes |  |
| src-tauri/src/torrent.rs:1330 | Complete torrent metadata temporary is missing; its journal was preserved |  |
| src-tauri/src/torrent.rs:1332 | Torrent metadata temporary no longer matches its complete durable bytes |  |
| src-tauri/src/torrent.rs:1335 | Missing metadata size |  |
| src-tauri/src/torrent.rs:1357 | Torrent contains an absolute path or traversal |  |
| src-tauri/src/torrent.rs:1363 | Torrent filename is not UTF-8 |  |
| src-tauri/src/torrent.rs:1374 | Torrent contains an unsafe portable filename |  |
| src-tauri/src/torrent.rs:1384 | Torrent staging lacks its original directory ownership proof |  |
| src-tauri/src/torrent.rs:1386 | Torrent staging directory was replaced; cleanup was not authorized |  |
| src-tauri/src/torrent.rs:1404 | Kiwix found an unexpected temporary folder and left it alone. Restart the app |  |
| src-tauri/src/torrent.rs:1407 | Torrent cleanup capture directory was replaced |  |
| src-tauri/src/torrent.rs:1416 | Torrent cleanup capture contains an unowned entry |  |
| src-tauri/src/torrent.rs:1421 | Kiwix found an unexpected temporary file and left it alone. Restart the app |  |
| src-tauri/src/torrent.rs:1423 | Owned staging file disappeared during provenance validation |  |
| src-tauri/src/torrent.rs:1436 | Torrent staging is not its owned directory; cleanup was not authorized |  |
| src-tauri/src/torrent.rs:1468 | Legacy torrent metadata lacks original file ownership proof |  |
| src-tauri/src/torrent.rs:1469 | Torrent metadata disappeared during provenance validation |  |
| src-tauri/src/torrent.rs:1479 | Missing published torrent filename |  |
| src-tauri/src/torrent.rs:1487 | Published torrent archive lacks its original ownership proof; library deletion was not authorized |  |
| src-tauri/src/torrent.rs:1490 | Published torrent archive disappeared during authorization |  |
| src-tauri/src/torrent.rs:1492 | Published torrent archive disappeared during checksum verification |  |
| src-tauri/src/torrent.rs:1494 | Published torrent archive was replaced; library deletion was not authorized |  |
| src-tauri/src/torrent.rs:1504 | Completed torrent path has no original publication proof; library deletion was not authorized |  |
| src-tauri/src/torrent.rs:1512 | Torrent staging has no parent |  |
| src-tauri/src/torrent.rs:1513 | Torrent staging has no filename |  |
| src-tauri/src/torrent.rs:1542 | No metadata parent |  |
| src-tauri/src/transfer_policy.rs:129 | The bound Wi-Fi or Ethernet route is no longer available |  |
| src-tauri/src/transfer_policy.rs:135 | Native transfer policy is not installed |  |
| src-tauri/src/transfer_policy.rs:136 | isLocalPeer |  |
| src-tauri/src/transfer_policy.rs:136 | route |  |
| src-tauri/src/transfer_policy.rs:137 | local |  |
| src-tauri/src/transfer_policy.rs:138 | Native bound-route locality reply is invalid |  |
| src-tauri/src/transfer_policy.rs:197 | A confirmed Wi-Fi or Ethernet route is required for local sharing |  |
| src-tauri/src/transfer_policy.rs:205 | bindLocalSocket |  |
| src-tauri/src/transfer_policy.rs:205 | fd |  |
| src-tauri/src/transfer_policy.rs:207 | Native LAN binding has no route receipt |  |
| src-tauri/src/update.rs:358 | KIWIX_UPDATER_PUBKEY |  |
| src-tauri/src/update.rs:374 | The update key in this build is not valid. Ask whoever supplied Kiwix for an updated build. |  |
| src-tauri/src/update.rs:413 | Updates are not available until Kiwix finishes starting. Try again in a moment. |  |
| src-tauri/src/update.rs:553 | Kiwix could not check whether a transfer is in progress. Wait a moment, then try again. |  |
| src-tauri/src/update.rs:606 | Kiwix does not check for updates automatically on a metered connection. Connect to Wi-Fi or allow metered downloads in Settings. |  |
| src-tauri/src/update.rs:723 | The available update changed. Check for updates again. |  |
| src-tauri/src/update.rs:840 | Installing an update requires confirmation. |  |
| src-tauri/src/update.rs:843 | Update installation admission is unavailable |  |
| src-tauri/src/update.rs:847 | Unknown update action |  |
| src-tauri/src/watch.rs:336 | Monitored folders: native watching is unavailable, falling back to periodic scans: {error} |  |
| src-tauri/src/watch.rs:428 | Monitored folders: cannot watch {}: {error} |  |
| src-tauri/src/windows.rs:70 | Window |  |
| src-tauri/src/windows.rs:70 | Show Previous Tab |  |
| src-tauri/src/windows.rs:70 | Show Next Tab |  |
| src-tauri/src/windows.rs:70 | Merge All Windows |  |
| src-tauri/src/windows.rs:71 | Move Tab to New Window |  |
| src-tauri/src/windows.rs:71 | Show Tab Bar |  |
| src-tauri/src/windows.rs:71 | Hide Tab Bar |  |
| src-tauri/src/windows.rs:71 | Minimize |  |
| src-tauri/src/windows.rs:98 | Native window tab groups could not be restored |  |
| src-tauri/src/windows.rs:110 | Native window tab groups could not be captured |  |
| src-tauri/src/windows.rs:113 | Kiwix could not read the current window groups: {e} |  |
| src-tauri/src/windows.rs:347 | A window could not be restored: {error} |  |
| src-tauri/src/zim_scheme.rs:48 | Empty zim:// link |  |
| src-tauri/src/zim_scheme.rs:51 | zim:// link is too long |  |
| src-tauri/src/zim_scheme.rs:53 | Malformed zim:// link |  |
| src-tauri/src/zim_scheme.rs:54 | zim |  |
| src-tauri/src/zim_scheme.rs:55 | Only zim:// links are accepted |  |
| src-tauri/src/zim_scheme.rs:58 | zim:// link must name an archive and path |  |
| src-tauri/src/zim_scheme.rs:61 | zim:// links cannot carry credentials |  |
| src-tauri/src/zim_scheme.rs:64 | zim:// links cannot carry a port |  |
| src-tauri/src/zim_scheme.rs:68 | This link is missing its archive. Open the article from your library |  |
| src-tauri/src/zim_scheme.rs:84 | zim:// article path is too long |  |
| src-tauri/src/zim_scheme.rs:90 | zim:// article query is too long |  |
| src-tauri/src/zim_scheme.rs:98 | zim:// article fragment is too long |  |
| src-tauri/src/zim_scheme.rs:116 | // |  |
| src-tauri/src/zim_scheme.rs:128 | zim:// article path contains control characters |  |
| src-tauri/src/zim_scheme.rs:133 | zim:// article path contains a backslash |  |
| src-tauri/src/zim_scheme.rs:139 | zim:// article path contains a traversal segment |  |
| src-tauri/src/zim_scheme.rs:146 | zim:// article query or fragment contains control characters |  |

## Frontend error and notice candidates

| Source | Extracted literal | Vocabulary review candidates |
| --- | --- | --- |
| src/App.tsx:1459 | Could not reconcile restored archive editions. Retry restoring the session. |  |
| src/App.tsx:1789 | Session flush was cancelled. |  |
| src/api.ts:448 | Search is only available inside the Kiwix window. |  |
| src/api.ts:498 | Search cancelled |  |
| src/api.ts:499 | Search request identity exhausted |  |
| src/api.ts:913 | Download cancelled. |  |
| src/api.ts:1051 | Leave private or read-only mode before downloading an edition. |  |
| src/api.ts:1101 | Disk space confirmation is already registered |  |
| src/api.ts:1345 | Use Add files to read originals, or explicitly import a managed copy. |  |
| src/api.ts:1393 | Choose an app storage location in Settings, or use Add folder to read originals. |  |
| src/api.ts:1523 | Article PDF export requires the native application. |  |
| src/components/Catalog.tsx:188 | Browsing session expired after a privacy transition |  |
| src/components/Catalog.tsx:279 | The catalog source URL is invalid. |  |
| src/components/Catalog.tsx:298 | External links are disabled in Settings. |  |
| src/components/Catalog.tsx:317 | Online preview requires HTTPS. |  |
| src/components/CrashReport.tsx:52 | The report could not be copied. |  |
| src/components/Downloads.tsx:673 | This download changed. Close this dialog and try again. |  |
| src/components/Downloads.tsx:792 | The installed archive changed. Check for editions again. |  |
| src/components/Downloads.tsx:809 | This edition decision changed. Check for editions again. |  |
| src/components/Downloads.tsx:824 | Cancel this edition update? Your original archive remains installed and its files are kept. |  |
| src/components/Downloads.tsx:856 | Roll back to the retained original archive? Reading references move back to that edition; both archive files are kept. |  |
| src/components/Library.tsx:452 | No archive files were found. Nothing was deleted. |  |
| src/components/LocalSwarmControls.tsx:132 | This share changed or is no longer available. Choose the sender again. |  |
| src/components/LocalSwarmControls.tsx:273 | This share is not a supported ZIM archive or split archive. |  |
| src/components/LocalSwarmControls.tsx:429 | Use a shorter device name without control or direction-override characters. |  |
| src/components/Maps.tsx:432 | Map catalog changed. Reload the map registry before downloading. |  |
| src/components/Maps.tsx:480 | Download settings changed. Start this map download again. |  |
| src/components/Maps.tsx:607 | The download or saved map changed. Close this dialog and try again. |  |
| src/components/Maps.tsx:1006 | Clear the map assembly cache? Installed maps will not be removed. |  |
| src/components/PrivateReadingSync.tsx:315 | Disconnect this device? It will no longer receive private reading data. Data already saved on either device is kept. |  |
| src/components/PrivateReadingSync.tsx:315 | Disconnect device |  |
| src/components/PrivateReadingSync.tsx:326 | Reset private sync history and disconnect every device? Your reading list and this device’s reading positions are kept. Positions received from other devices are removed. Reset private sync history on your other devices too before reconnecting. |  |
| src/components/PrivateReadingSync.tsx:326 | Reset private sync history |  |
| src/components/PrivateReadingSync.tsx:333 | Forget all saved sync positions and disconnect every device? Reading history, reading list and open tabs are kept. Saved sync position candidates are removed and will not be restored from unchanged history or tabs. Reset your other devices too before reconnecting. |  |
| src/components/PrivateReadingSync.tsx:333 | Clear saved sync positions and reset connections |  |
| src/components/Reader.tsx:1063 | The article changed before export completed. |  |
| src/components/Reader.tsx:1704 | Archive storage is waiting for persistence acknowledgement |  |
| src/components/Reader.tsx:2852 | The live page could not be captured. |  |
| src/components/Reader.tsx:2952 | The article attempted to leave its archive. |  |
| src/components/Reader.tsx:3255 | This type of external link is not permitted. |  |
| src/components/Reader.tsx:3870 | The archive changed; reopen the article before exporting. |  |
| src/components/Reader.tsx:5009 | Dismiss error |  |
| src/components/SelectedStorageActions.tsx:76 | The managed copies are ready. Some originals were kept; restart Kiwix to finish cleanup. |  |
| src/components/SettingsAutosave.tsx:128 | Enter a font size from 10 to 48. |  |
| src/components/SettingsAutosave.tsx:139 | Enter a complete HTTP or HTTPS URL without embedded credentials. |  |
| src/components/SettingsAutosave.tsx:281 | The download directory or privacy settings changed. Previous downloads have not been moved. |  |
| src/main.tsx:51 | Missing application root |  |
| src/native-speech.ts:158 | The article did not provide text for read-aloud. |  |
| src/private-sync-api.ts:29 | Private device sync is unavailable in private or read-only mode. |  |
| src/reader-storage.ts:389 | Kiwix could not finish closing the reader. Close and reopen this window to continue. |  |
| src/reader-storage.ts:406 | Invalid native storage acknowledgement |  |
| src/reader-storage.ts:646 | Archive storage rejected an invalid mutation. |  |
| src/reader-storage.ts:1092 | The archive did not acknowledge its storage drain. |  |

## Native prompt candidates

| Source | Extracted literal | Vocabulary review candidates |
| --- | --- | --- |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:63 | \|\| name.startsWith( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:63 | ) \|\| name.startsWith( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:74 | quic interface candidates=${candidates.size} current=$current before=$beforeInterfaces client=$clientInterfaces beforeClient=$beforeClientInterfaces |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:84 | )) nearbyRequire(args.getString( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:84 | ) == quicLease, |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:97 | , iface.address.hostAddress).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:106 | , iface != null && args.getString( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:119 | Android did not publish the active hotspot's network name and password |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:123 | The Wi-Fi hotspot is already starting |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:133 | calling startLocalOnlyHotspot |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:139 | Android refused the hotspot. Allow nearby Wi-Fi access and turn on Location services, then try again. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:143 | Android refused the hotspot: ${error.message ?: |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:147 | Android refused the hotspot: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:152 | stop active=${reservation != null} pending=${pending != null} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:174 | Android did not release the hotspot: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:177 | The Wi-Fi hotspot was stopped before it finished starting |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:211 | onStarted pending=${pending != null} stopRequested=$stopRequested |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:230 | Android started a hotspot but did not publish its network name and password |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:239 | onStopped closing=$closing |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:247 | Android stopped the hotspot before it started |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:255 | onFailed reason=$reason |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:273 | This device cannot create a Wi-Fi hotspot |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:275 | Wi-Fi is busy with another mode. Turn off Wi-Fi Direct or tethering and try again. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:277 | Hotspot use is blocked by device policy |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:278 | Android could not start the Wi-Fi hotspot |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:291 | , legacySsid).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:297 | , ssid).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt:307 | ') \|\| !value.endsWith(' |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:82 | Android 13 and later only exposes shared-storage files you grant a folder for. Add folder access to scan Downloads or Documents. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:86 | No folder access has been granted yet. Add folder access to include ZIM files outside app storage. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:88 | Stopped after $cap archive candidates; narrow the granted folders. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:106 | , entries).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:132 | ${MediaStore.MediaColumns.DISPLAY_NAME} LIKE ? |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:168 | Shared-storage access was denied; only app-visible files were searched. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:170 | MediaStore could not be searched: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:192 | Shared storage could not be searched: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:206 | , true) \|\| name.equals( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:247 | Folder access expired for ${tree.authority ?: |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:249 | Could not scan ${tree.lastPathSegment ?: tree.authority ?: |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:331 | Split archive is incomplete or has extra parts; import a copy. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:356 | ${('a'.code + index / 26).toChar()}${('a'.code + index % 26).toChar()} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:389 | ) -> path.removePrefix( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:405 | ) -> normalized.removePrefix( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:406 | + rest.substringBefore('/') + |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt:436 | This location is not readable in place; import a copy. |  |
