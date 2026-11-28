# Error copy inventory

Generated from current sources by `node scripts/generate-error-docs.mjs`;
`node scripts/generate-error-docs.mjs --check` reports whether it is current.
This is a heuristic candidate-literal inventory, not proof that a string reaches
a user, is translated, or has a particular runtime severity. Extraction can
include code, tests and interpolation fragments; review candidates in context.
No replacement history is inferred by pairing unrelated added/deleted strings.
The Source column names the file that holds the literal, without a line number;
a literal found in several files is listed once, under the first of them.

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
| src-tauri/src/acquisition.rs |  |  |
| src-tauri/src/acquisition.rs | .kiwix-{}.part |  |
| src-tauri/src/acquisition.rs | .meta4 |  |
| src-tauri/src/acquisition.rs | .metalink |  |
| src-tauri/src/acquisition.rs | A download must have exactly one published archive |  |
| src-tauri/src/acquisition.rs | AUX |  |
| src-tauri/src/acquisition.rs | Acquisition monitor claim has a non-Unicode filename |  |
| src-tauri/src/acquisition.rs | Acquisition monitor claim has an invalid filename |  |
| src-tauri/src/acquisition.rs | Acquisition monitor claim requires a Unicode path without NUL |  |
| src-tauri/src/acquisition.rs | Acquisition monitor claim requires an absolute path |  |
| src-tauri/src/acquisition.rs | Adopting native download bytes: {error} |  |
| src-tauri/src/acquisition.rs | CON |  |
| src-tauri/src/acquisition.rs | Cancelled download file was left in place because Kiwix cannot prove it created it |  |
| src-tauri/src/acquisition.rs | Download destination is not absolute |  |
| src-tauri/src/acquisition.rs | Download has no destination directory |  |
| src-tauri/src/acquisition.rs | Download has no native transport lease |  |
| src-tauri/src/acquisition.rs | Download part changed identity during a segmented transfer |  |
| src-tauri/src/acquisition.rs | Download part is already in use: {e} |  |
| src-tauri/src/acquisition.rs | Download state exceeds size limit |  |
| src-tauri/src/acquisition.rs | Filename is reserved by the operating system |  |
| src-tauri/src/acquisition.rs | Filename must be a single safe .zim filename, at most 240 UTF-8 bytes |  |
| src-tauri/src/acquisition.rs | Inspecting {}: {e} |  |
| src-tauri/src/acquisition.rs | Invalid acquisition history component |  |
| src-tauri/src/acquisition.rs | Invalid completed HTTP publication size |  |
| src-tauri/src/acquisition.rs | Kiwix cannot verify this download's original file. Add the archive to your library manually |  |
| src-tauri/src/acquisition.rs | Malformed percent-encoded filename |  |
| src-tauri/src/acquisition.rs | Missing acquisition history filename |  |
| src-tauri/src/acquisition.rs | NUL |  |
| src-tauri/src/acquisition.rs | Native background download failed |  |
| src-tauri/src/acquisition.rs | Native completion has no device receipt |  |
| src-tauri/src/acquisition.rs | Native completion has no durable stage |  |
| src-tauri/src/acquisition.rs | Native completion has no inode receipt |  |
| src-tauri/src/acquisition.rs | Native download cannot replace an unowned canonical part |  |
| src-tauri/src/acquisition.rs | Native download did not provide a complete successful archive response |  |
| src-tauri/src/acquisition.rs | Native download has no provable change time |  |
| src-tauri/src/acquisition.rs | Native download has no resolved URL |  |
| src-tauri/src/acquisition.rs | Native download has no stable inode receipt |  |
| src-tauri/src/acquisition.rs | Native download length differs from its authoritative archive size |  |
| src-tauri/src/acquisition.rs | Native download no longer matches its completed transport receipt |  |
| src-tauri/src/acquisition.rs | Native download returned an unknown transport state |  |
| src-tauri/src/acquisition.rs | Native download stage changed before adoption |  |
| src-tauri/src/acquisition.rs | Native download stage changed while its bytes were being adopted |  |
| src-tauri/src/acquisition.rs | Native download stage ended before its completed size |  |
| src-tauri/src/acquisition.rs | Native staging and adoption size overflow |  |
| src-tauri/src/acquisition.rs | Native staging and canonical download part must be distinct files |  |
| src-tauri/src/acquisition.rs | Native staging directory has no accessible filesystem ancestor |  |
| src-tauri/src/acquisition.rs | Noncanonical persisted download ID |  |
| src-tauri/src/acquisition.rs | Opening completed native download: {error} |  |
| src-tauri/src/acquisition.rs | Opening download part: {e} |  |
| src-tauri/src/acquisition.rs | Owned native part was replaced; refusing to start transport |  |
| src-tauri/src/acquisition.rs | PRN |  |
| src-tauri/src/acquisition.rs | Persisted destination does not match filename |  |
| src-tauri/src/acquisition.rs | Persisted destination is not normalized |  |
| src-tauri/src/acquisition.rs | Persisted download key/ID mismatch |  |
| src-tauri/src/acquisition.rs | Published archive disappeared during recovery |  |
| src-tauri/src/acquisition.rs | Refusing non-regular file: {} |  |
| src-tauri/src/acquisition.rs | Retaining native download receipt: {error} |  |
| src-tauri/src/acquisition.rs | Temporary download file changed identity or has hard links |  |
| src-tauri/src/acquisition.rs | Temporary download file disappeared |  |
| src-tauri/src/acquisition.rs | Temporary download file is missing |  |
| src-tauri/src/acquisition.rs | The download worker stopped unexpectedly. Resume to try again |  |
| src-tauri/src/acquisition.rs | The downloaded archive changed. Kiwix left the file unchanged; add it to your library manually if needed |  |
| src-tauri/src/acquisition.rs | The downloaded archive was replaced or changed. Kiwix left the file unchanged; add it to your library manually if needed |  |
| src-tauri/src/acquisition.rs | The downloaded archive was replaced. Kiwix left the replacement file unchanged; add it to your library manually if needed |  |
| src-tauri/src/acquisition.rs | This download is no longer available for import |  |
| src-tauri/src/acquisition.rs | URL has no filename; specify one explicitly |  |
| src-tauri/src/acquisition.rs | Unknown download ID |  |
| src-tauri/src/acquisition.rs | cancelled |  |
| src-tauri/src/acquisition.rs | complete |  |
| src-tauri/src/acquisition.rs | failed |  |
| src-tauri/src/acquisition.rs | ios |  |
| src-tauri/src/acquisition.rs | missing |  |
| src-tauri/src/acquisition.rs | paused |  |
| src-tauri/src/acquisition.rs | verifying |  |
| src-tauri/src/acquisition/finalize.rs | Archive added, but Kiwix could not finish saving its download history. Restart the app to finish tidying up |  |
| src-tauri/src/acquisition/finalize.rs | Destination has no parent |  |
| src-tauri/src/acquisition/finalize.rs | Download changed identity after its checksum pass; verify it again |  |
| src-tauri/src/acquisition/finalize.rs | Download changed identity during checksum verification |  |
| src-tauri/src/acquisition/finalize.rs | Download finalizer did not start |  |
| src-tauri/src/acquisition/finalize.rs | Download part changed before publication |  |
| src-tauri/src/acquisition/finalize.rs | Download receipt does not match the durable byte count |  |
| src-tauri/src/acquisition/finalize.rs | Downloaded archive failed its advertised SHA-256 checksum; cancel and start a new download |  |
| src-tauri/src/acquisition/finalize.rs | Downloaded file is too short to be a ZIM archive |  |
| src-tauri/src/acquisition/finalize.rs | Downloaded resource is not a ZIM archive (invalid magic) |  |
| src-tauri/src/acquisition/finalize.rs | Final download length {downloaded} differs from expected {expected} |  |
| src-tauri/src/acquisition/finalize.rs | Publishing verified archive without overwrite: {error} |  |
| src-tauri/src/acquisition/finalize.rs | Saved download layout is incomplete |  |
| src-tauri/src/acquisition/finalize.rs | The archive is available, but Kiwix could not confirm it will survive a sudden power loss. Restart the app to check the download. |  |
| src-tauri/src/acquisition/finalize.rs | The downloaded file is not the expected size any more; resume to download it again |  |
| src-tauri/src/acquisition/finalize.rs | The downloaded file was replaced before it could be finished; resume to download it again |  |
| src-tauri/src/acquisition/finalize.rs | downloads |  |
| src-tauri/src/acquisition/http_meta.rs | Advertised download size overflows available-space calculation |  |
| src-tauri/src/acquisition/http_meta.rs | Checking free space: {e} |  |
| src-tauri/src/acquisition/http_meta.rs | Expected an IETF Metalink document |  |
| src-tauri/src/acquisition/http_meta.rs | Invalid HTTP {name} |  |
| src-tauri/src/acquisition/http_meta.rs | Invalid Metalink SHA-256 |  |
| src-tauri/src/acquisition/http_meta.rs | Metalink file has no name |  |
| src-tauri/src/acquisition/http_meta.rs | Metalink has no eligible HTTPS mirror (loopback HTTP is fixture-only) |  |
| src-tauri/src/acquisition/http_meta.rs | Metalink has no file |  |
| src-tauri/src/acquisition/http_meta.rs | Metalink has no valid file size |  |
| src-tauri/src/acquisition/http_meta.rs | Metalink resolution stopped |  |
| src-tauri/src/acquisition/http_meta.rs | Metalink response exceeds size limit |  |
| src-tauri/src/acquisition/http_meta.rs | Metalink server returned HTTP {} |  |
| src-tauri/src/acquisition/http_meta.rs | Multi-file Metalinks are not supported |  |
| src-tauri/src/acquisition/http_meta.rs | Not enough disk space: need {needed} bytes, have {free} |  |
| src-tauri/src/acquisition/http_meta.rs | file |  |
| src-tauri/src/acquisition/http_meta.rs | name |  |
| src-tauri/src/acquisition/manager.rs | A download already owns this URL or destination; resume or cancel it instead |  |
| src-tauri/src/acquisition/manager.rs | Cannot prove download ownership of {}; files and history were preserved |  |
| src-tauri/src/acquisition/manager.rs | Cannot prove original download ownership of {}; no installed archive was authorized for deletion |  |
| src-tauri/src/acquisition/manager.rs | Cannot prove ownership of {}; files and download history were preserved. Remove installed archives through the library |  |
| src-tauri/src/acquisition/manager.rs | Completed downloads cannot be paused or cancelled; manage the archive in the library |  |
| src-tauri/src/acquisition/manager.rs | Destination already exists; existing files are never overwritten |  |
| src-tauri/src/acquisition/manager.rs | Destination already exists; refusing to overwrite it |  |
| src-tauri/src/acquisition/manager.rs | Destination is not valid UTF-8 |  |
| src-tauri/src/acquisition/manager.rs | Download changed during cleanup authorization |  |
| src-tauri/src/acquisition/manager.rs | Download changed during cleanup authorization; retry with its current state |  |
| src-tauri/src/acquisition/manager.rs | Download changed while its files were being deleted |  |
| src-tauri/src/acquisition/manager.rs | Download failed while stopping |  |
| src-tauri/src/acquisition/manager.rs | Download is still stopping; await pause/cancel before resuming |  |
| src-tauri/src/acquisition/manager.rs | Download ownership changed during verification; retry the import |  |
| src-tauri/src/acquisition/manager.rs | Download ownership ledger is full; explicitly delete retained downloads and files before adding jobs |  |
| src-tauri/src/acquisition/manager.rs | Download was removed during cleanup authorization |  |
| src-tauri/src/acquisition/manager.rs | Download was removed from history or file cleanup has started; retry deletion instead |  |
| src-tauri/src/acquisition/manager.rs | Downloaded archive is no longer available |  |
| src-tauri/src/acquisition/manager.rs | Encoding download request: {e} |  |
| src-tauri/src/acquisition/manager.rs | Expected size must be positive when supplied |  |
| src-tauri/src/acquisition/manager.rs | Native download reconciliation requires normal writable mode |  |
| src-tauri/src/acquisition/manager.rs | Only finished complete, failed or cancelled downloads can be forgotten |  |
| src-tauri/src/acquisition/manager.rs | Only finished complete, failed or cancelled downloads can be removed from history |  |
| src-tauri/src/acquisition/manager.rs | Only finished downloads can have their files deleted |  |
| src-tauri/src/acquisition/manager.rs | Only paused or failed downloads can resume |  |
| src-tauri/src/acquisition/manager.rs | Only terminal downloads can authorize cleanup |  |
| src-tauri/src/acquisition/manager.rs | Owned download disappeared during cleanup authorization |  |
| src-tauri/src/acquisition/manager.rs | Published archive no longer belongs to this download; library deletion was not authorized |  |
| src-tauri/src/acquisition/manager.rs | Published download changed; file and history were preserved |  |
| src-tauri/src/acquisition/manager.rs | Published download could not be verified |  |
| src-tauri/src/acquisition/manager.rs | Published download disappeared before cleanup |  |
| src-tauri/src/acquisition/manager.rs | Published download has no durable checksum proof |  |
| src-tauri/src/acquisition/manager.rs | Published download has no original inode proof |  |
| src-tauri/src/acquisition/manager.rs | Shared library grant lock is poisoned |  |
| src-tauri/src/acquisition/manager.rs | Some saved downloads could not be restored and are shown as failed. |  |
| src-tauri/src/acquisition/manager.rs | The shared archive changed; cancel the previous download before downloading it again | shared archive |
| src-tauri/src/acquisition/manager.rs | kiwix-library: |  |
| src-tauri/src/acquisition/manager.rs | {SHARED_EARLIER_SESSION}: {} |  |
| src-tauri/src/acquisition/persistence.rs | A saved download could not be read and was skipped. |  |
| src-tauri/src/acquisition/persistence.rs | Invalid native download adoption checkpoint |  |
| src-tauri/src/acquisition/persistence.rs | Invalid persisted download status |  |
| src-tauri/src/acquisition/persistence.rs | Invalid persistent download state: expected an object |  |
| src-tauri/src/acquisition/persistence.rs | Invalid persistent download state: {e} |  |
| src-tauri/src/acquisition/persistence.rs | Invalid shared library history URL |  |
| src-tauri/src/acquisition/persistence.rs | Invalid transfer history filename |  |
| src-tauri/src/acquisition/persistence.rs | Missing transfer history folder |  |
| src-tauri/src/acquisition/persistence.rs | Opening transfer history: {e} |  |
| src-tauri/src/acquisition/persistence.rs | The transfer history must be a regular file |  |
| src-tauri/src/acquisition/persistence.rs | Too many persisted download jobs |  |
| src-tauri/src/acquisition/persistence.rs | {stem}.unreadable-{}.json |  |
| src-tauri/src/acquisition/segments.rs | 206 Content-Length does not match Content-Range |  |
| src-tauri/src/acquisition/segments.rs | 206 range or object total changed during a segmented transfer |  |
| src-tauri/src/acquisition/segments.rs | 416 response ended a segmented transfer |  |
| src-tauri/src/acquisition/segments.rs | Download segment ended before its declared length |  |
| src-tauri/src/acquisition/segments.rs | Download segment exceeds its declared length |  |
| src-tauri/src/acquisition/segments.rs | Download segment task failed: {error} |  |
| src-tauri/src/acquisition/segments.rs | Download server returned HTTP {other} |  |
| src-tauri/src/acquisition/segments.rs | Download server returned HTTP {status} for a segment |  |
| src-tauri/src/acquisition/segments.rs | Encoded HTTP download cannot be resumed safely |  |
| src-tauri/src/acquisition/segments.rs | Invalid 206 Content-Range |  |
| src-tauri/src/acquisition/segments.rs | Object validator changed during a segmented transfer |  |
| src-tauri/src/acquisition/segments.rs | Receiving download segment: {error} |  |
| src-tauri/src/acquisition/segments.rs | Requesting download segment: {error} |  |
| src-tauri/src/acquisition/segments.rs | Segmented download ended with an incomplete range |  |
| src-tauri/src/acquisition/segments.rs | Segmented download layout changed while it was running |  |
| src-tauri/src/acquisition/segments.rs | Segmented download layout is missing |  |
| src-tauri/src/acquisition/segments.rs | Server ignored a range request during a segmented transfer |  |
| src-tauri/src/acquisition/transfer.rs | 206 range start or object validator does not match the partial file |  |
| src-tauri/src/acquisition/transfer.rs | 416 response does not prove that the partial file is complete |  |
| src-tauri/src/acquisition/transfer.rs | 416 response has no valid bytes */length Content-Range |  |
| src-tauri/src/acquisition/transfer.rs | Download body exceeds its declared length |  |
| src-tauri/src/acquisition/transfer.rs | Download ended before the expected object size |  |
| src-tauri/src/acquisition/transfer.rs | Download has no transfer URL |  |
| src-tauri/src/acquisition/transfer.rs | Download length overflow |  |
| src-tauri/src/acquisition/transfer.rs | Download response ended before its declared length |  |
| src-tauri/src/acquisition/transfer.rs | Download response length overflow |  |
| src-tauri/src/acquisition/transfer.rs | Download server kept ending partial responses before the expected object size |  |
| src-tauri/src/acquisition/transfer.rs | Download server returned HTTP {status} |  |
| src-tauri/src/acquisition/transfer.rs | Download size differs from expected size: {actual} versus {expected} |  |
| src-tauri/src/acquisition/transfer.rs | Flushing download: {error} |  |
| src-tauri/src/acquisition/transfer.rs | Invalid download range |  |
| src-tauri/src/acquisition/transfer.rs | Invalid download validator |  |
| src-tauri/src/acquisition/transfer.rs | MOBILE_NETWORK_BLOCKED |  |
| src-tauri/src/acquisition/transfer.rs | Owned HTTP part was replaced; refusing to resume writes |  |
| src-tauri/src/acquisition/transfer.rs | Partial transfer lacks a safe resume validator |  |
| src-tauri/src/acquisition/transfer.rs | Receiving download: {error} |  |
| src-tauri/src/acquisition/transfer.rs | Requesting download: |  |
| src-tauri/src/acquisition/transfer.rs | Resumed object total changed |  |
| src-tauri/src/acquisition/transfer.rs | Server sent an unsolicited partial response |  |
| src-tauri/src/acquisition/transfer.rs | Too many HTTP redirects |  |
| src-tauri/src/acquisition/transfer.rs | Unsafe HTTP redirect |  |
| src-tauri/src/acquisition/transfer.rs | bytes={downloaded}- |  |
| src-tauri/src/acquisition/transfer.rs | identity |  |
| src-tauri/src/agent_connector.rs | Agent connector stopped |  |
| src-tauri/src/agent_connector.rs | Agent connector unavailable |  |
| src-tauri/src/agent_connector.rs | Agent connector: cannot open another channel instance: {error} |  |
| src-tauri/src/agent_connector.rs | Cannot identify this Kiwix profile |  |
| src-tauri/src/agent_connector.rs | Kiwix is busy with other agent requests; try again shortly. |  |
| src-tauri/src/agent_connector.rs | No available archive has that ID. Call list_archives for valid IDs. |  |
| src-tauri/src/agent_connector.rs | No matching archive is available. Call list_archives for valid IDs. |  |
| src-tauri/src/agent_connector.rs | None of the requested archives could be searched right now. |  |
| src-tauri/src/agent_connector.rs | Refused an agent connector client from another account |  |
| src-tauri/src/agent_connector.rs | Request too large |  |
| src-tauri/src/agent_connector.rs | That entry is not an article (it is media or another binary file). |  |
| src-tauri/src/agent_connector.rs | That path is not valid. Use a path returned by search. |  |
| src-tauri/src/agent_connector.rs | The agent connector is available on desktop in this version. |  |
| src-tauri/src/agent_connector.rs | The relay stopped reading |  |
| src-tauri/src/agent_connector.rs | The request took too long. Search fewer archives (archiveIds) and try again. |  |
| src-tauri/src/agent_connector.rs | Too many requests to Kiwix; wait a few seconds and try again. |  |
| src-tauri/src/agent_connector.rs | Unknown tool |  |
| src-tauri/src/agent_connector.rs | archiveIds must be a list of at most 64 archive IDs |  |
| src-tauri/src/agent_connector.rs | archiveIds must be strings |  |
| src-tauri/src/agent_connector.rs | error |  |
| src-tauri/src/agent_connector.rs | id |  |
| src-tauri/src/agent_connector.rs | mcp-{suffix}.sock |  |
| src-tauri/src/agent_connector.rs | not attempted |  |
| src-tauri/src/agent_connector.rs | results |  |
| src-tauri/src/agent_connector.rs | {base}-{suffix} |  |
| src-tauri/src/agent_connector.rs | {key} must be 1 to {max_chars} characters |  |
| src-tauri/src/agent_connector.rs | {key} must be an integer from {min} to {max} |  |
| src-tauri/src/android_evaluation.rs | Article assertion exceeds bound |  |
| src-tauri/src/android_evaluation.rs | Case exceeds bounds or lacks validity assertions |  |
| src-tauri/src/android_evaluation.rs | Composed ZIM MD5 mismatch |  |
| src-tauri/src/android_evaluation.rs | Invalid fixture relative path |  |
| src-tauri/src/android_evaluation.rs | Map composition cancelled |  |
| src-tauri/src/android_evaluation.rs | Missing name |  |
| src-tauri/src/android_evaluation.rs | Missing parent |  |
| src-tauri/src/android_evaluation.rs | Native evaluation panicked |  |
| src-tauri/src/android_evaluation.rs | Negative case did not exercise real writing |  |
| src-tauri/src/android_evaluation.rs | Negative case failed for an unrelated reason |  |
| src-tauri/src/android_evaluation.rs | Negative case left output or partial |  |
| src-tauri/src/android_evaluation.rs | Negative case lost journal |  |
| src-tauri/src/android_evaluation.rs | Negative case unexpectedly published output |  |
| src-tauri/src/android_evaluation.rs | Retained-byte ZIM checksum failed |  |
| src-tauri/src/android_evaluation.rs | Run size overflow |  |
| src-tauri/src/android_evaluation.rs | Search assertion exceeds bound |  |
| src-tauri/src/android_evaluation.rs | Unknown evaluation case |  |
| src-tauri/src/android_evaluation.rs | assertions |  |
| src-tauri/src/android_evaluation.rs | cancel |  |
| src-tauri/src/android_evaluation.rs | noPartial |  |
| src-tauri/src/android_evaluation.rs | noPublishedOutput |  |
| src-tauri/src/android_evaluation.rs | observedError |  |
| src-tauri/src/android_evaluation.rs | ok |  |
| src-tauri/src/android_evaluation.rs | output |  |
| src-tauri/src/android_evaluation.rs | schema |  |
| src-tauri/src/android_evaluation.rs | wroteBytes |  |
| src-tauri/src/android_notifications_service.rs | Android completion notification: {error} |  |
| src-tauri/src/android_notifications_service.rs | Android notification profile unavailable |  |
| src-tauri/src/android_notifications_service.rs | Completion notification permission requires the trusted root window |  |
| src-tauri/src/android_notifications_service.rs | Download notifications are unavailable. Restart the app |  |
| src-tauri/src/android_notifications_service.rs | Invalid Android notification profile identity |  |
| src-tauri/src/android_notifications_service.rs | Launch queue unavailable |  |
| src-tauri/src/android_notifications_service.rs | Missing Android notification permission result |  |
| src-tauri/src/android_notifications_service.rs | Mobile lifecycle unavailable |  |
| src-tauri/src/android_notifications_service.rs | Notification open expired after a lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs | Notification open expired after a privacy or lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs | Notification opens belong to the trusted root window |  |
| src-tauri/src/android_notifications_service.rs | Notification permission expired after a lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs | Notification permission expired after a privacy or lifecycle transition |  |
| src-tauri/src/android_notifications_service.rs | The notification open is no longer pending |  |
| src-tauri/src/android_notifications_service.rs | The notified archive installation is no longer available |  |
| src-tauri/src/android_notifications_service.rs | Too many pending launch locations |  |
| src-tauri/src/android_notifications_service.rs | __notification |  |
| src-tauri/src/android_notifications_service.rs | __notificationGeneration |  |
| src-tauri/src/android_notifications_service.rs | acknowledgeCompletionNotificationTap |  |
| src-tauri/src/android_notifications_service.rs | available |  |
| src-tauri/src/android_notifications_service.rs | granted |  |
| src-tauri/src/android_notifications_service.rs | launchId |  |
| src-tauri/src/android_notifications_service.rs | main |  |
| src-tauri/src/android_notifications_service.rs | reason |  |
| src-tauri/src/android_notifications_service.rs | takeCompletionNotificationTap |  |
| src-tauri/src/android_notifications_service.rs | tap |  |
| src-tauri/src/android_tls.rs | Android application context is invalid: {error} |  |
| src-tauri/src/android_tls.rs | the Android application context is unavailable |  |
| src-tauri/src/apple_requests.rs | Apple PDF request IDs exhausted |  |
| src-tauri/src/apple_window.rs | The AppKit window is only available on the main thread |  |
| src-tauri/src/apple_window.rs | The AppKit window is unavailable |  |
| src-tauri/src/archive.rs | Archive directory must be valid Unicode |  |
| src-tauri/src/archive.rs | Archive item offset overflow |  |
| src-tauri/src/archive.rs | Archive path lease is not the expected regular entry |  |
| src-tauri/src/archive.rs | Illustration_ |  |
| src-tauri/src/archive.rs | Integrity check cancelled |  |
| src-tauri/src/archive.rs | Invalid archive argument: Archive read offset exceeds item size |  |
| src-tauri/src/archive.rs | Invalid archive argument: invalid or oversized archive path |  |
| src-tauri/src/archive.rs | Invalid archive filesystem root |  |
| src-tauri/src/archive.rs | Invalid retained archive filename |  |
| src-tauri/src/archive.rs | Retained archive directory is not normalized |  |
| src-tauri/src/archive.rs | Retained archive part identity changed |  |
| src-tauri/src/archive.rs | Retained archive part size changed |  |
| src-tauri/src/archive.rs | Retained archive path must be absolute |  |
| src-tauri/src/archive.rs | Search cancelled |  |
| src-tauri/src/archive.rs | Unsupported retained archive filesystem root |  |
| src-tauri/src/archive.rs | {MALFORMED_ARCHIVE}: Cyclic or excessive archive redirect chain (limit 256) |  |
| src-tauri/src/archive.rs | {MALFORMED_ARCHIVE}: Expected an archive item |  |
| src-tauri/src/archive.rs | {MALFORMED_ARCHIVE}: Truncated archive item range |  |
| src-tauri/src/archive.rs | {MALFORMED_ARCHIVE}: Unknown archive MIME type |  |
| src-tauri/src/archive_metadata.rs | Archive metadata exceeds the 1 MiB limit |  |
| src-tauri/src/archive_metadata.rs | Archive offset overflow |  |
| src-tauri/src/archive_metadata.rs | Spelling title exceeds 16 KiB limit |  |
| src-tauri/src/archive_metadata.rs | Title batch limit must be 1..256 and offset must fit signed 32 bits |  |
| src-tauri/src/archive_metadata.rs | Title batch offset exceeds title count |  |
| src-tauri/src/archive_metadata.rs | Truncated archive metadata |  |
| src-tauri/src/archive_metadata.rs | {ENTRY_NOT_FOUND}: No article |  |
| src-tauri/src/archive_metadata.rs | {ENTRY_NOT_FOUND}: No main page |  |
| src-tauri/src/archive_search.rs | /fulltextIndex/xapian |  |
| src-tauri/src/archive_search.rs | Archive contents changed. Reopen the archive. |  |
| src-tauri/src/archive_search.rs | Archive has no full-text search index |  |
| src-tauri/src/archive_search.rs | Archive language tag exceeds 1024 bytes |  |
| src-tauri/src/archive_search.rs | Archive part lengths overflow |  |
| src-tauri/src/archive_search.rs | Embedded search index exceeds the archive |  |
| src-tauri/src/archive_search.rs | Search exceeds query or pagination limits |  |
| src-tauri/src/archive_search.rs | Search response exceeds 32 MiB limit |  |
| src-tauri/src/archive_search.rs | Search result field exceeds 1 MiB |  |
| src-tauri/src/archive_search.rs | Truncated search index cluster |  |
| src-tauri/src/archive_search.rs | Unexpected embedded search index kind |  |
| src-tauri/src/archive_search.rs | language |  |
| src-tauri/src/archive_source.rs | External document-provider references require their original mobile platform; select the archive again on this device |  |
| src-tauri/src/catalog.rs | Acquisition entry exceeds {MAX_ENTRY_TAGS} distinct tags |  |
| src-tauri/src/catalog.rs | Acquisition entry requires an id and title |  |
| src-tauri/src/catalog.rs | Acquisition link has no href |  |
| src-tauri/src/catalog.rs | Catalog root must be an Atom feed |  |
| src-tauri/src/catalog.rs | Facet feed exceeds {MAX_FACETS} distinct values |  |
| src-tauri/src/catalog.rs | Facet root must be an Atom feed |  |
| src-tauri/src/catalog.rs | HTTP {} for {} |  |
| src-tauri/src/catalog.rs | HTTPS is required (downloads additionally permit literal loopback HTTP fixture URLs) |  |
| src-tauri/src/catalog.rs | Image link has no href |  |
| src-tauri/src/catalog.rs | Invalid URL: {e} |  |
| src-tauri/src/catalog.rs | Invalid magnet URL: {e} |  |
| src-tauri/src/catalog.rs | Invalid magnet acquisition link |  |
| src-tauri/src/catalog.rs | Next link has no href |  |
| src-tauri/src/catalog.rs | Response exceeds {maximum} bytes |  |
| src-tauri/src/catalog.rs | The BitTorrent download link is too large |  |
| src-tauri/src/catalog.rs | The publisher returned an invalid BitTorrent info hash |  |
| src-tauri/src/catalog.rs | The publisher returned an invalid BitTorrent link |  |
| src-tauri/src/catalog.rs | The publisher returned no unambiguous BitTorrent v1 download |  |
| src-tauri/src/catalog.rs | This is not an official BitTorrent download endpoint |  |
| src-tauri/src/catalog.rs | Torrent acquisition has no href |  |
| src-tauri/src/catalog.rs | URL exceeds 8192 bytes |  |
| src-tauri/src/catalog.rs | URL must be a public internet destination |  |
| src-tauri/src/catalog.rs | URL must have a host and no credentials or fragment |  |
| src-tauri/src/catalog.rs | href |  |
| src-tauri/src/catalog.rs | link |  |
| src-tauri/src/catalog.rs | magnet |  |
| src-tauri/src/catalog.rs | magnet: |  |
| src-tauri/src/catalog.rs | title |  |
| src-tauri/src/catalog.rs | totalResults |  |
| src-tauri/src/catalog_cache.rs | A saved catalog page is damaged. Refresh the catalog |  |
| src-tauri/src/catalog_cache.rs | Catalog cache changed while reading |  |
| src-tauri/src/catalog_cache.rs | Catalog cache directory attachment changed |  |
| src-tauri/src/catalog_cache.rs | Catalog cache directory unavailable |  |
| src-tauri/src/catalog_cache.rs | Catalog cache exceeds disk limit |  |
| src-tauri/src/catalog_cache.rs | Catalog cache is not a bounded, unlinked regular file |  |
| src-tauri/src/catalog_cache.rs | Catalog cache permission descriptor changed identity |  |
| src-tauri/src/catalog_cache.rs | Catalog cache publication changed |  |
| src-tauri/src/catalog_cache.rs | Catalog cache scratch file changed |  |
| src-tauri/src/catalog_cache.rs | Catalog cache scratch file is not app-owned |  |
| src-tauri/src/catalog_cache.rs | Catalog cache scratch file is unsafe |  |
| src-tauri/src/catalog_cache.rs | Catalog cache unavailable: {e} |  |
| src-tauri/src/catalog_cache.rs | Catalog index exceeds disk limit |  |
| src-tauri/src/catalog_cache.rs | Catalog index is not a bounded, unlinked regular file |  |
| src-tauri/src/catalog_cache.rs | Creating catalog cache: {error} |  |
| src-tauri/src/catalog_cache.rs | Invalid OPDS redirect: {e} |  |
| src-tauri/src/catalog_cache.rs | Loading catalog cache: {e} |  |
| src-tauri/src/catalog_cache.rs | OPDS redirect has no valid Location |  |
| src-tauri/src/catalog_cache.rs | Opening catalog cache: {error} |  |
| src-tauri/src/catalog_cache.rs | Reading catalog cache: {error} |  |
| src-tauri/src/catalog_cache.rs | Reading catalog index: {error} |  |
| src-tauri/src/catalog_cache.rs | Reading {label}: {error} |  |
| src-tauri/src/catalog_cache.rs | The catalog did not refresh. Try again |  |
| src-tauri/src/catalog_cache.rs | Unsupported or oversized catalog cache |  |
| src-tauri/src/catalog_cache.rs | Your privacy settings changed, so the catalog did not update. Open the catalog again |  |
| src-tauri/src/catalog_cache.rs | cache |  |
| src-tauri/src/catalog_cache.rs | http |  |
| src-tauri/src/catalog_cache.rs | https |  |
| src-tauri/src/catalog_cache.rs | offline |  |
| src-tauri/src/catalog_cache.rs | {"version":1,"records":[]} |  |
| src-tauri/src/catalog_cache.rs | {error}; {warning} |  |
| src-tauri/src/catalog_cache.rs | {label} is not a bounded, unlinked regular file |  |
| src-tauri/src/catalog_index.rs | Catalog edition selection is unavailable in private mode |  |
| src-tauri/src/catalog_index.rs | Catalog index lock failed |  |
| src-tauri/src/catalog_index.rs | Catalog snapshot exceeds durable index capacity | snapshot |
| src-tauri/src/catalog_index.rs | Saved catalog entry has invalid source membership |  |
| src-tauri/src/catalog_index.rs | Saved catalog index has invalid source identity or bounds |  |
| src-tauri/src/catalog_index.rs | Saved catalog page membership is invalid |  |
| src-tauri/src/catalog_index.rs | Saved catalog traversal has invalid completeness |  |
| src-tauri/src/catalog_index.rs | The catalog selection has invalid source identity |  |
| src-tauri/src/catalog_index.rs | The catalog selection is stale. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs | Unknown catalog selection. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs | Unknown catalog source. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs | Unknown or conflicting catalog entry. Refresh the catalog |  |
| src-tauri/src/catalog_index.rs | Unsupported catalog index |  |
| src-tauri/src/catalog_index.rs | traversed |  |
| src-tauri/src/crash.rs | Kiwix could not delete the saved crash report. Try again |  |
| src-tauri/src/crash.rs | Kiwix could not read the saved crash report. Close this prompt and try again |  |
| src-tauri/src/crash.rs | The saved crash report is damaged. Delete it from Troubleshooting |  |
| src-tauri/src/crash.rs | The saved crash report is too large to open. Delete it from Troubleshooting |  |
| src-tauri/src/crash.rs | The saved crash report was written by a different version of Kiwix. Delete it from Troubleshooting |  |
| src-tauri/src/crash.rs | unknown |  |
| src-tauri/src/desktop_workflow.rs | Desktop dialog parent is not a native Unicode path |  |
| src-tauri/src/desktop_workflow.rs | Desktop dialog path is not Unicode |  |
| src-tauri/src/desktop_workflow.rs | Desktop dialog paths must be native absolute paths |  |
| src-tauri/src/desktop_workflow.rs | Desktop dialog selection must name a file |  |
| src-tauri/src/desktop_workflow.rs | Invalid desktop dialog path |  |
| src-tauri/src/desktop_workflow.rs | Invalid suggested export filename |  |
| src-tauri/src/desktop_workflow.rs | Unknown desktop dialog kind |  |
| src-tauri/src/device_scan.rs | Device scan archive has too many parts |  |
| src-tauri/src/device_scan.rs | Device scan archive name does not match its parts |  |
| src-tauri/src/device_scan.rs | Device scan entry has no source |  |
| src-tauri/src/device_scan.rs | Device scan returned too many archives |  |
| src-tauri/src/device_scan.rs | Device scan returned too many limitations |  |
| src-tauri/src/device_scan.rs | Invalid archive name in device scan |  |
| src-tauri/src/device_scan.rs | Invalid archive part in device scan |  |
| src-tauri/src/device_scan.rs | Invalid {what} in device scan |  |
| src-tauri/src/device_scan.rs | The file list is out of date. Scan your device again |  |
| src-tauri/src/device_scan.rs | Unknown device scan source |  |
| src-tauri/src/device_scan.rs | limitation |  |
| src-tauri/src/device_scan.rs | volume |  |
| src-tauri/src/diagnostics.rs | Cannot write opt-in diagnostics: {error} |  |
| src-tauri/src/disk_warning.rs | Download destination ancestor is not a directory |  |
| src-tauri/src/disk_warning.rs | Download destination changed while checking its free space |  |
| src-tauri/src/disk_warning.rs | Download destination has no accessible filesystem ancestor |  |
| src-tauri/src/disk_warning.rs | Download destination must be an absolute directory |  |
| src-tauri/src/disk_warning.rs | Encoding download warning: {e} |  |
| src-tauri/src/disk_warning.rs | Inspecting download volume: {e} |  |
| src-tauri/src/disk_warning.rs | Invalid download destination |  |
| src-tauri/src/disk_warning.rs | Not enough free space for this download |  |
| src-tauri/src/disk_warning.rs | known remaining size |  |
| src-tauri/src/disk_warning.rs | {WARNING_PREFIX}{} |  |
| src-tauri/src/edition_service.rs | Configured catalogs changed; check archive editions again |  |
| src-tauri/src/edition_service.rs | Edition destination has no parent |  |
| src-tauri/src/edition_service.rs | Edition errors unavailable |  |
| src-tauri/src/edition_service.rs | Edition installation changed; review it again |  |
| src-tauri/src/edition_service.rs | Invalid edition output name |  |
| src-tauri/src/edition_service.rs | Select the catalog edition again |  |
| src-tauri/src/edition_service.rs | Stop sharing this archive before deleting its original edition |  |
| src-tauri/src/edition_service.rs | Stop sharing this archive before replacing its edition |  |
| src-tauri/src/edition_service.rs | Stop sharing this archive before rolling back its edition |  |
| src-tauri/src/edition_service.rs | The catalog artifact changed. The retained verified output was kept; select an edition again |  |
| src-tauri/src/edition_service.rs | The catalog edition changed; review the verified output again |  |
| src-tauri/src/edition_service.rs | The catalog edition changed; select it again |  |
| src-tauri/src/edition_service.rs | The edition decision is no longer available |  |
| src-tauri/src/edition_service.rs | The edition transfer is no longer available; retained files have not been changed |  |
| src-tauri/src/edition_service.rs | The installed archive changed; select its edition again |  |
| src-tauri/src/edition_service.rs | The retained edition decision changed; check for editions again |  |
| src-tauri/src/edition_service.rs | This catalog is no longer configured |  |
| src-tauri/src/edition_service.rs | Unknown edition intent |  |
| src-tauri/src/edition_service.rs | Unknown edition operation |  |
| src-tauri/src/edition_service.rs | Unknown edition transaction |  |
| src-tauri/src/edition_service.rs | Use explicit rollback for a committed edition |  |
| src-tauri/src/edition_service.rs | Wait for archive verification to finish before replacing |  |
| src-tauri/src/edition_service.rs | associationConfirmed |  |
| src-tauri/src/edition_service.rs | catalogGeneration |  |
| src-tauri/src/edition_service.rs | committed |  |
| src-tauri/src/edition_service.rs | decisionToken |  |
| src-tauri/src/edition_service.rs | diskSpaceAcknowledgement |  |
| src-tauri/src/edition_service.rs | errors |  |
| src-tauri/src/edition_service.rs | installationId |  |
| src-tauri/src/edition_service.rs | intentId |  |
| src-tauri/src/edition_service.rs | sources |  |
| src-tauri/src/edition_service.rs | updates |  |
| src-tauri/src/edition_service.rs | {source}: {error} |  |
| src-tauri/src/edition_updates.rs | A committed edition requires explicit rollback |  |
| src-tauri/src/edition_updates.rs | Archive installation changed |  |
| src-tauri/src/edition_updates.rs | Archive relocation changed its installation or content metadata |  |
| src-tauri/src/edition_updates.rs | Bind the edition download before verification |  |
| src-tauri/src/edition_updates.rs | Catalog promises an invalid archive UUID | uuid |
| src-tauri/src/edition_updates.rs | Catalog source changed; select this edition again |  |
| src-tauri/src/edition_updates.rs | Confirm this archive's catalog association before preparing its edition |  |
| src-tauri/src/edition_updates.rs | Download does not belong to this edition intent |  |
| src-tauri/src/edition_updates.rs | Download location changed; select this edition again |  |
| src-tauri/src/edition_updates.rs | Downloaded archive UUID does not match the catalog's explicit UUID promise | uuid |
| src-tauri/src/edition_updates.rs | Edition acquisition was already consumed |  |
| src-tauri/src/edition_updates.rs | Edition authorization belongs to another download |  |
| src-tauri/src/edition_updates.rs | Edition authorization expired; select this edition again |  |
| src-tauri/src/edition_updates.rs | Edition authorization unavailable |  |
| src-tauri/src/edition_updates.rs | Edition changes require normal writable mode |  |
| src-tauri/src/edition_updates.rs | Edition cutover requires its exact durable verified reservation |  |
| src-tauri/src/edition_updates.rs | Edition download directory changed. Select this edition again |  |
| src-tauri/src/edition_updates.rs | Edition import has no verified download publication |  |
| src-tauri/src/edition_updates.rs | Edition intent changed before commit |  |
| src-tauri/src/edition_updates.rs | Edition intent changed during verification |  |
| src-tauri/src/edition_updates.rs | Edition intent changed. Select this edition again |  |
| src-tauri/src/edition_updates.rs | Edition intent is no longer available |  |
| src-tauri/src/edition_updates.rs | Edition receipt belongs to another acquisition |  |
| src-tauri/src/edition_updates.rs | Edition relocation no longer matches its transaction |  |
| src-tauri/src/edition_updates.rs | Edition reservation disappeared |  |
| src-tauri/src/edition_updates.rs | Edition rollback authorization expired; choose rollback again |  |
| src-tauri/src/edition_updates.rs | Edition series changed. Select this edition again |  |
| src-tauri/src/edition_updates.rs | Edition transaction changed; its original was not released |  |
| src-tauri/src/edition_updates.rs | Edition transaction changed; rollback was not authorized |  |
| src-tauri/src/edition_updates.rs | Edition transaction is no longer available |  |
| src-tauri/src/edition_updates.rs | Invalid reference revision |  |
| src-tauri/src/edition_updates.rs | Opening edition download directory: {error} |  |
| src-tauri/src/edition_updates.rs | Original archive changed after rollback verification |  |
| src-tauri/src/edition_updates.rs | Original archive is unavailable; reconnect its storage or provider: {error} |  |
| src-tauri/src/edition_updates.rs | Reference revision exhausted |  |
| src-tauri/src/edition_updates.rs | Rollback transaction disappeared |  |
| src-tauri/src/edition_updates.rs | Saved catalog association is invalid |  |
| src-tauri/src/edition_updates.rs | Saved edition intent is invalid; preserve the profile and restore its backup |  |
| src-tauri/src/edition_updates.rs | Saved edition reference-transition history is invalid |  |
| src-tauri/src/edition_updates.rs | Saved edition transaction is invalid |  |
| src-tauri/src/edition_updates.rs | Saved reference revision has missing transitions |  |
| src-tauri/src/edition_updates.rs | Select a currently configured catalog source |  |
| src-tauri/src/edition_updates.rs | Select an unambiguous native catalog entry first |  |
| src-tauri/src/edition_updates.rs | Select this edition again before continuing |  |
| src-tauri/src/edition_updates.rs | Select this edition again before downloading |  |
| src-tauri/src/edition_updates.rs | Select this edition again before replacing |  |
| src-tauri/src/edition_updates.rs | Session reference revision is newer than this profile |  |
| src-tauri/src/edition_updates.rs | Session-only provider references cannot be retained by an edition transaction |  |
| src-tauri/src/edition_updates.rs | The installed archive changed; select this edition again |  |
| src-tauri/src/edition_updates.rs | The installed archive's change time is unavailable; select this edition again |  |
| src-tauri/src/edition_updates.rs | The installed edition changed; rollback cannot overwrite it |  |
| src-tauri/src/edition_updates.rs | The original archive could not be checked, so it was not deleted: {error} |  |
| src-tauri/src/edition_updates.rs | The original archive failed verification; neither edition was changed |  |
| src-tauri/src/edition_updates.rs | The original edition is already installed separately |  |
| src-tauri/src/edition_updates.rs | The original of this edition is in the library. Remove it from the library instead |  |
| src-tauri/src/edition_updates.rs | The original of this edition was already deleted |  |
| src-tauri/src/edition_updates.rs | The retained original archive changed since this update, so Kiwix did not delete it. Delete the file yourself if you no longer need it, then choose this again |  |
| src-tauri/src/edition_updates.rs | The retained original archive changed; rollback was not authorized |  |
| src-tauri/src/edition_updates.rs | The verified edition is already installed separately; no archive was overwritten |  |
| src-tauri/src/edition_updates.rs | This catalog entry is not a comparable newer edition of the installed archive |  |
| src-tauri/src/edition_updates.rs | This edition was already rolled back |  |
| src-tauri/src/edition_updates.rs | This edition was rolled back, so its original is the installed archive. Remove it from the library instead |  |
| src-tauri/src/edition_updates.rs | Verified archive does not match the selected catalog edition |  |
| src-tauri/src/edition_updates.rs | Verified import does not belong to this edition intent |  |
| src-tauri/src/edition_updates.rs | Verify this edition before replacing the installed archive |  |
| src-tauri/src/edition_updates.rs | device |  |
| src-tauri/src/edition_updates.rs | downloading |  |
| src-tauri/src/edition_updates.rs | ready |  |
| src-tauri/src/edition_updates.rs | reserved |  |
| src-tauri/src/edition_updates.rs | urn:uuid: |  |
| src-tauri/src/exports.rs | Archive path contains dot segments that cannot be represented as an MHTML URL |  |
| src-tauri/src/exports.rs | Archive resource has an invalid MIME header |  |
| src-tauri/src/exports.rs | Archive returned bytes beyond the resource size |  |
| src-tauri/src/exports.rs | Article exceeds the 32768 static-reference limit |  |
| src-tauri/src/exports.rs | Article exceeds the 4096 packaged-asset limit |  |
| src-tauri/src/exports.rs | Asset URL exceeds the interoperable MHTML header limit: {location} |  |
| src-tauri/src/exports.rs | CSS static-reference limit exceeded |  |
| src-tauri/src/exports.rs | Cannot package referenced asset {location}: {error} |  |
| src-tauri/src/exports.rs | Checking export destination: {error} |  |
| src-tauri/src/exports.rs | Choose a regular file for this export |  |
| src-tauri/src/exports.rs | Choose a writable regular file without other links for this export |  |
| src-tauri/src/exports.rs | Conflicting redirected asset locations: {url} |  |
| src-tauri/src/exports.rs | Export destination changed. Choose the file again |  |
| src-tauri/src/exports.rs | Export destination has no filename |  |
| src-tauri/src/exports.rs | Invalid or oversized archive resource path |  |
| src-tauri/src/exports.rs | Locking export destination: {e} |  |
| src-tauri/src/exports.rs | MHTML export requires an HTML article |  |
| src-tauri/src/exports.rs | MHTML package exceeds the 2 GiB source-data limit |  |
| src-tauri/src/exports.rs | Missing archive resource path |  |
| src-tauri/src/exports.rs | Opening export destination: {error} |  |
| src-tauri/src/exports.rs | Opening export destination: {e} |  |
| src-tauri/src/exports.rs | Saving export: {error}. Restoring the original file also failed: {restore} |  |
| src-tauri/src/exports.rs | Saving export: {error}. The original file was restored. |  |
| src-tauri/src/exports.rs | Static asset URL exceeds 16 KiB |  |
| src-tauri/src/exports.rs | Text asset {} exceeds the 16 MiB parsing limit |  |
| src-tauri/src/exports.rs | The chosen file changed before export. Choose a destination again |  |
| src-tauri/src/exports.rs | The export completed and the file was kept, but Kiwix could not confirm that it will survive a sudden power loss. |  |
| src-tauri/src/exports.rs | The saved file changed during export. Check the chosen folder before trying again |  |
| src-tauri/src/exports.rs | The saved file moved during export. Check the chosen folder before trying again |  |
| src-tauri/src/exports.rs | The temporary export changed. Please save again |  |
| src-tauri/src/exports.rs | This archive is damaged. Remove it from your library and add it again |  |
| src-tauri/src/exports.rs | Too many static references in one text asset |  |
| src-tauri/src/exports.rs | Unescaped newline in CSS string |  |
| src-tauri/src/exports.rs | Unexpected end of archive resource {} at byte {offset} |  |
| src-tauri/src/exports.rs | Unterminated CSS string in article assets |  |
| src-tauri/src/exports.rs | import |  |
| src-tauri/src/exports.rs | src |  |
| src-tauri/src/exports.rs | url |  |
| src-tauri/src/fs_ext.rs | path contains a NUL byte |  |
| src-tauri/src/fs_ext.rs | volume path has no existing ancestor |  |
| src-tauri/src/fs_ext.rs | volume path is too long |  |
| src-tauri/src/hotspot.rs | Android has not published the hotspot interface yet; wait for the hotspot to finish starting and try again |  |
| src-tauri/src/hotspot.rs | Android returned an invalid hotspot name |  |
| src-tauri/src/hotspot.rs | Android returned an invalid hotspot password |  |
| src-tauri/src/hotspot.rs | Android returned hotspot credentials with control characters |  |
| src-tauri/src/hotspot.rs | The Wi-Fi hotspot is already running |  |
| src-tauri/src/hotspot.rs | The Wi-Fi hotspot is already starting |  |
| src-tauri/src/hotspot.rs | The Wi-Fi hotspot is still stopping |  |
| src-tauri/src/imports.rs | Acquisition receipt already belongs to different archive paths |  |
| src-tauri/src/imports.rs | Archive changed after verification; retry the import |  |
| src-tauri/src/imports.rs | Archive verification is shut down |  |
| src-tauri/src/imports.rs | Archive verification omitted its retained content identities |  |
| src-tauri/src/imports.rs | Archive verification queue is full |  |
| src-tauri/src/imports.rs | Archive verification requires a receipt and archive paths |  |
| src-tauri/src/imports.rs | Archive verification was cancelled |  |
| src-tauri/src/imports.rs | Archive verification worker did not retire |  |
| src-tauri/src/imports.rs | Archive verification worker ownership missing |  |
| src-tauri/src/imports.rs | Archive verification worker panicked |  |
| src-tauri/src/imports.rs | Only failed archive verification can be retried |  |
| src-tauri/src/imports.rs | Unknown acquisition receipt |  |
| src-tauri/src/imports.rs | Verified import is no longer available |  |
| src-tauri/src/instance.rs | Empty request |  |
| src-tauri/src/instance.rs | Invalid request: {error} |  |
| src-tauri/src/instance.rs | Kiwix instance: accept failed: {error} |  |
| src-tauri/src/instance.rs | Launch payload has no absolute working directory |  |
| src-tauri/src/instance.rs | No profile rendezvous |  |
| src-tauri/src/instance.rs | Profile path is too long for a local socket ({}); second instances will explain the conflict instead |  |
| src-tauri/src/instance.rs | Request exceeds the instance protocol limit |  |
| src-tauri/src/instance.rs | The activation endpoint belongs to another account |  |
| src-tauri/src/instance.rs | The owning instance rejected the activation request |  |
| src-tauri/src/instance.rs | Too many launch arguments |  |
| src-tauri/src/instance.rs | Unauthorized peer |  |
| src-tauri/src/instance.rs | Unsupported instance action |  |
| src-tauri/src/instance.rs | Unsupported instance protocol version |  |
| src-tauri/src/instance.rs | Unsupported launch payload |  |
| src-tauri/src/instance.rs | activate |  |
| src-tauri/src/instance.rs | {"status":"error"} |  |
| src-tauri/src/instance/socket.rs | .retired-{} |  |
| src-tauri/src/instance/socket.rs | .starting-{} |  |
| src-tauri/src/instance/socket.rs | Another entry occupies the local socket path |  |
| src-tauri/src/instance/socket.rs | Another listener uses the local socket |  |
| src-tauri/src/instance/socket.rs | Local socket changed before removal |  |
| src-tauri/src/instance/socket.rs | Local socket changed before setting permissions |  |
| src-tauri/src/instance/socket.rs | Local socket changed during publication |  |
| src-tauri/src/instance/socket.rs | Local socket changed during removal |  |
| src-tauri/src/instance/socket.rs | Local socket directory belongs to another user |  |
| src-tauri/src/instance/socket.rs | Local socket directory moved during startup |  |
| src-tauri/src/instance/socket.rs | Missing profile directory |  |
| src-tauri/src/instance/socket.rs | Missing socket directory |  |
| src-tauri/src/instance/socket.rs | Missing socket directory name |  |
| src-tauri/src/instance/socket.rs | Missing socket name |  |
| src-tauri/src/integrity.rs | An archive integrity check is already running |  |
| src-tauri/src/integrity.rs | Archive changed during integrity check |  |
| src-tauri/src/integrity.rs | Archive no longer in library |  |
| src-tauri/src/integrity.rs | Archive not in library |  |
| src-tauri/src/integrity.rs | Integrity worker unavailable |  |
| src-tauri/src/integrity.rs | checksum |  |
| src-tauri/src/integrity.rs | structure |  |
| src-tauri/src/ios_background.rs | Native application delegate class is unavailable before application run |  |
| src-tauri/src/ios_background.rs | Native background request IDs exhausted |  |
| src-tauri/src/ios_background.rs | Native background transport returned no response |  |
| src-tauri/src/ios_background.rs | Reading native background response: {error} |  |
| src-tauri/src/ios_background.rs | failure |  |
| src-tauri/src/kokoro.rs | Creating the voice download scratch file failed: {e} |  |
| src-tauri/src/kokoro.rs | Download the natural voice model in Settings first |  |
| src-tauri/src/kokoro.rs | Downloading the voice model failed (HTTP {}) |  |
| src-tauri/src/kokoro.rs | Installing the natural voice model failed: {e} |  |
| src-tauri/src/kokoro.rs | Invalid voice cache file name |  |
| src-tauri/src/kokoro.rs | Natural voices are disabled on this device because synthesis cannot keep up in real time; choose a system voice |  |
| src-tauri/src/kokoro.rs | Reading cached natural voice audio failed: {e} |  |
| src-tauri/src/kokoro.rs | Reading the verified voice model failed: {error} |  |
| src-tauri/src/kokoro.rs | Reopening the installed natural voice model failed: {e} |  |
| src-tauri/src/kokoro.rs | Saving cached natural voice audio failed: {e} |  |
| src-tauri/src/kokoro.rs | Starting read-aloud failed: {error} |  |
| src-tauri/src/kokoro.rs | The article has no readable text |  |
| src-tauri/src/kokoro.rs | The natural voice engine could not start |  |
| src-tauri/src/kokoro.rs | The natural voice installation is no longer active |  |
| src-tauri/src/kokoro.rs | The natural voice model changed before it could be loaded |  |
| src-tauri/src/kokoro.rs | The natural voice model changed while loading: {e} |  |
| src-tauri/src/kokoro.rs | The natural voice performance check failed unexpectedly |  |
| src-tauri/src/kokoro.rs | The natural voice performance check found no pronounceable text |  |
| src-tauri/src/kokoro.rs | The natural voice performance check produced no audio |  |
| src-tauri/src/kokoro.rs | The natural voice performance check produced no measurable audio |  |
| src-tauri/src/kokoro.rs | The natural voice performance check was cancelled |  |
| src-tauri/src/kokoro.rs | The natural voice produced no audio |  |
| src-tauri/src/kokoro.rs | The natural voice session ended |  |
| src-tauri/src/kokoro.rs | The natural voice synthesis was cancelled |  |
| src-tauri/src/kokoro.rs | The network policy changed during the voice model download |  |
| src-tauri/src/kokoro.rs | The selected natural voice model changed. Check Settings and retry. |  |
| src-tauri/src/kokoro.rs | The voice model download exceeded its pinned size |  |
| src-tauri/src/kokoro.rs | The voice model download failed verification and was discarded |  |
| src-tauri/src/kokoro.rs | The voice model download was cancelled |  |
| src-tauri/src/kokoro.rs | The voice model is already downloading |  |
| src-tauri/src/kokoro.rs | The voice model is already installed |  |
| src-tauri/src/kokoro.rs | Unknown natural voice |  |
| src-tauri/src/kokoro.rs | Wait for the voice model download to finish or cancel it first |  |
| src-tauri/src/kokoro.rs | Writing the voice model download failed: {error} |  |
| src-tauri/src/kokoro.rs | kokoro-performance |  |
| src-tauri/src/kokoro.rs | kokoro-read-aloud |  |
| src-tauri/src/kokoro.rs | offset |  |
| src-tauri/src/kokoro.rs | speaking |  |
| src-tauri/src/kokoro/audio_cache.rs | .wav |  |
| src-tauri/src/kokoro/audio_cache.rs | voice-audio-preserved-v1 |  |
| src-tauri/src/kokoro/install.rs | . |  |
| src-tauri/src/kokoro/install.rs | A natural voice installation is already present. |  |
| src-tauri/src/kokoro/install.rs | A previous voice removal is unfinished. Use Remove in Settings to retry cleanup. |  |
| src-tauri/src/kokoro/install.rs | An older voice installation was preserved. Choose Install or Repair in Settings to continue. |  |
| src-tauri/src/kokoro/install.rs | Free at least {} MiB on the app storage volume and retry the natural voice installation. |  |
| src-tauri/src/kokoro/install.rs | The bundled natural voice model was previously removed. |  |
| src-tauri/src/kokoro/install.rs | The previous natural voice installation was detached. |  |
| src-tauri/src/kokoro/install.rs | The previous voice removal did not finish. |  |
| src-tauri/src/kokoro/install.rs | The speech engine requires a Unicode model path. |  |
| src-tauri/src/kokoro/install.rs | apple |  |
| src-tauri/src/kokoro/install.rs | espeak-ng-data |  |
| src-tauri/src/kokoro/install.rs | model.fp16.onnx |  |
| src-tauri/src/kokoro/install.rs | tokens.txt |  |
| src-tauri/src/kokoro/install.rs | voices.bin |  |
| src-tauri/src/launch_search.rs | The kiwix:// link has no search text |  |
| src-tauri/src/launch_search.rs | The kiwix:// link is too long |  |
| src-tauri/src/launch_search.rs | kiwix |  |
| src-tauri/src/launch_search.rs | q |  |
| src-tauri/src/lib.rs | --article requires an archive article path |  |
| src-tauri/src/lib.rs | --article requires exactly one archive path |  |
| src-tauri/src/lib.rs | --data-dir requires a path |  |
| src-tauri/src/lib.rs | --headless |  |
| src-tauri/src/lib.rs | --help |  |
| src-tauri/src/lib.rs | --help and --version are only valid for the first launch |  |
| src-tauri/src/lib.rs | --open-url |  |
| src-tauri/src/lib.rs | --open-url must be the only argument |  |
| src-tauri/src/lib.rs | --open-url takes exactly one kiwix:// link and no other arguments |  |
| src-tauri/src/lib.rs | --port |  |
| src-tauri/src/lib.rs | --port requires a number |  |
| src-tauri/src/lib.rs | --search |  |
| src-tauri/src/lib.rs | --search cannot be combined with archive paths or --headless |  |
| src-tauri/src/lib.rs | --search has no searchable text |  |
| src-tauri/src/lib.rs | --search requires text to look up |  |
| src-tauri/src/lib.rs | -h |  |
| src-tauri/src/lib.rs | Cannot identify profile {}: {error} |  |
| src-tauri/src/lib.rs | Cannot locate application data directory; provide --data-dir |  |
| src-tauri/src/lib.rs | Cannot open archives while application Quit is pending |  |
| src-tauri/src/lib.rs | Close this window to continue |  |
| src-tauri/src/lib.rs | Command-line arguments: {error} |  |
| src-tauri/src/lib.rs | Could not open startup archive: {failure} |  |
| src-tauri/src/lib.rs | Imported archives payload is missing its book list |  |
| src-tauri/src/lib.rs | Invalid port |  |
| src-tauri/src/lib.rs | No native application is available to open the archive |  |
| src-tauri/src/lib.rs | No native window is available to open the archive |  |
| src-tauri/src/lib.rs | Not available |  |
| src-tauri/src/lib.rs | Portable mode requires adjacent data/; conflicting --data-dir is not permitted (the matching data path is allowed) |  |
| src-tauri/src/lib.rs | Quit Without Saving |  |
| src-tauri/src/lib.rs | Read-only mode can open registered archives only |  |
| src-tauri/src/lib.rs | Retry |  |
| src-tauri/src/lib.rs | This action could not be started. Try again |  |
| src-tauri/src/lib.rs | Unknown option: {arg} |  |
| src-tauri/src/lib.rs | Window groups could not be restored: {error} |  |
| src-tauri/src/lib.rs | __windowLabel |  |
| src-tauri/src/lib.rs | a version 3 profile must not open |  |
| src-tauri/src/lib.rs | android |  |
| src-tauri/src/lib.rs | library_maintenance |  |
| src-tauri/src/lib.rs | maps |  |
| src-tauri/src/lib.rs | path |  |
| src-tauri/src/lib.rs | recover_migrations |  |
| src-tauri/src/lib.rs | unexpectedly busy: {error} |  |
| src-tauri/src/lib.rs | {reason}. No archives were opened or imported. Quit the running Kiwix instance, then repeat this command to use the requested profile and mode. |  |
| src-tauri/src/lib.rs | {}\n\n{detail} |  |
| src-tauri/src/library_files.rs | {error}. |  |
| src-tauri/src/library_files.rs | .zim |  |
| src-tauri/src/library_files.rs | .zimaa |  |
| src-tauri/src/library_files.rs | ; |  |
| src-tauri/src/library_files.rs | A legacy archive has no declared checksum boundary |  |
| src-tauri/src/library_files.rs | A saved backup could not be read. Restart the app |  |
| src-tauri/src/library_files.rs | A staged copy changed and was preserved at {} |  |
| src-tauri/src/library_files.rs | Acquisition backup evidence exceeds its state limit |  |
| src-tauri/src/library_files.rs | Archive changed during source admission |  |
| src-tauri/src/library_files.rs | Archive changed during source verification |  |
| src-tauri/src/library_files.rs | Archive changed during verification |  |
| src-tauri/src/library_files.rs | Archive changed size while hashing |  |
| src-tauri/src/library_files.rs | Archive changed while retaining its exact part set |  |
| src-tauri/src/library_files.rs | Archive file changed while opening |  |
| src-tauri/src/library_files.rs | Archive filename must be valid Unicode |  |
| src-tauri/src/library_files.rs | Archive has no parent |  |
| src-tauri/src/library_files.rs | Archive has no parent directory |  |
| src-tauri/src/library_files.rs | Archive has no relative parent |  |
| src-tauri/src/library_files.rs | Archive header part changed before reading |  |
| src-tauri/src/library_files.rs | Archive header part changed while reading |  |
| src-tauri/src/library_files.rs | Archive is missing a declared split part |  |
| src-tauri/src/library_files.rs | Archive is missing parts, exceeds the 676-part bound, or has trailing data |  |
| src-tauri/src/library_files.rs | Archive part changed before deletion |  |
| src-tauri/src/library_files.rs | Archive part checksum changed: {} |  |
| src-tauri/src/library_files.rs | Archive part too large |  |
| src-tauri/src/library_files.rs | Archive parts exceed their declared checksum boundary |  |
| src-tauri/src/library_files.rs | Archive parts must be regular files |  |
| src-tauri/src/library_files.rs | Archive path changed during migration |  |
| src-tauri/src/library_files.rs | Archive size changed while copying; source retained |  |
| src-tauri/src/library_files.rs | Archive size overflow |  |
| src-tauri/src/library_files.rs | Archive streams are not regular archive filenames |  |
| src-tauri/src/library_files.rs | Attachment contents do not match the permitted document/media type. Use Save attachment instead. |  |
| src-tauri/src/library_files.rs | Attachment ended before its declared size |  |
| src-tauri/src/library_files.rs | Both migration names exist; source retained for manual recovery |  |
| src-tauri/src/library_files.rs | Cannot create migration journals |  |
| src-tauri/src/library_files.rs | Captured archive no longer belongs to the selected download |  |
| src-tauri/src/library_files.rs | Choose a folder inside Kiwix's own storage |  |
| src-tauri/src/library_files.rs | Copied archive checksum differs; source retained |  |
| src-tauri/src/library_files.rs | Deletion staging capability unavailable |  |
| src-tauri/src/library_files.rs | Deletion staging directory contains unexpected files |  |
| src-tauri/src/library_files.rs | Destination already exists: {} |  |
| src-tauri/src/library_files.rs | Destination path must be Unicode |  |
| src-tauri/src/library_files.rs | Directory identity changed: {} |  |
| src-tauri/src/library_files.rs | Download directory has no accessible filesystem root |  |
| src-tauri/src/library_files.rs | Download directory must be an absolute normalized path |  |
| src-tauri/src/library_files.rs | Download migration directories cannot contain one another |  |
| src-tauri/src/library_files.rs | Downloaded archive changed before deletion |  |
| src-tauri/src/library_files.rs | Duplicate archive has no parent directory |  |
| src-tauri/src/library_files.rs | Empty split archive part |  |
| src-tauri/src/library_files.rs | Export has no directory |  |
| src-tauri/src/library_files.rs | External attachments must be between 1 byte and 512 MiB. Use Save attachment for larger files. |  |
| src-tauri/src/library_files.rs | File changed while checking: {name} |  |
| src-tauri/src/library_files.rs | File identity changed: {name} |  |
| src-tauri/src/library_files.rs | File is not a ZIM archive |  |
| src-tauri/src/library_files.rs | Imported document has no directory |  |
| src-tauri/src/library_files.rs | Invalid ZIM checksum position |  |
| src-tauri/src/library_files.rs | Invalid canonical attachment path |  |
| src-tauri/src/library_files.rs | Invalid exact migration part manifest |  |
| src-tauri/src/library_files.rs | Invalid journal filename |  |
| src-tauri/src/library_files.rs | Invalid migration archive set |  |
| src-tauri/src/library_files.rs | Invalid single-component archive filename |  |
| src-tauri/src/library_files.rs | Journal does not describe the exact verified archive set; no source cleanup performed |  |
| src-tauri/src/library_files.rs | Journal is not bound to a managed library archive |  |
| src-tauri/src/library_files.rs | Kiwix could not verify this archive, so no files were deleted. Remove it from your library instead |  |
| src-tauri/src/library_files.rs | Kiwix could not verify this archive. Remove it from your library |  |
| src-tauri/src/library_files.rs | Kiwix will not clean up files outside its own export folder |  |
| src-tauri/src/library_files.rs | Library path no longer belongs to this migration; nothing was deleted |  |
| src-tauri/src/library_files.rs | Managed archive removed during migration |  |
| src-tauri/src/library_files.rs | Migration directory has no ownership marker; no recovery performed |  |
| src-tauri/src/library_files.rs | Migration directory identity changed; all remaining sources were preserved |  |
| src-tauri/src/library_files.rs | Migration filesystem identity changed |  |
| src-tauri/src/library_files.rs | Migration journal exceeds size bound |  |
| src-tauri/src/library_files.rs | Migration journal filename mismatch |  |
| src-tauri/src/library_files.rs | Migration source and destination are identical |  |
| src-tauri/src/library_files.rs | Migration staging directory was replaced |  |
| src-tauri/src/library_files.rs | Missing copy identity |  |
| src-tauri/src/library_files.rs | Missing destination identity |  |
| src-tauri/src/library_files.rs | Missing published copy identity |  |
| src-tauri/src/library_files.rs | Native import returned a path outside its owned staging directory |  |
| src-tauri/src/library_files.rs | Native import returned duplicate document paths |  |
| src-tauri/src/library_files.rs | Not a regular archive file: {name} |  |
| src-tauri/src/library_files.rs | Not enough free space in {}: {needed} bytes are needed but {free} bytes are free |  |
| src-tauri/src/library_files.rs | Oversized migration journal |  |
| src-tauri/src/library_files.rs | Owned library operation directory must be private to this user (0700) |  |
| src-tauri/src/library_files.rs | Owned operation directory capability unavailable |  |
| src-tauri/src/library_files.rs | Owned operation directory was replaced |  |
| src-tauri/src/library_files.rs | Owned operation directory was replaced; nothing was taken from {} |  |
| src-tauri/src/library_files.rs | Prepared export changed identity; retained at {} |  |
| src-tauri/src/library_files.rs | Read-only mode cannot move archives |  |
| src-tauri/src/library_files.rs | Reading ZIM header: {e} |  |
| src-tauri/src/library_files.rs | Removing this staged copy failed: {error} |  |
| src-tauri/src/library_files.rs | Retained archive directory was replaced |  |
| src-tauri/src/library_files.rs | Retained archive source changed |  |
| src-tauri/src/library_files.rs | Save the new download directory before moving previous downloads |  |
| src-tauri/src/library_files.rs | Select a .zim or first .zimaa archive part |  |
| src-tauri/src/library_files.rs | Selected document is not a ZIM archive or exact split part |  |
| src-tauri/src/library_files.rs | Selected document is not an exact split archive part |  |
| src-tauri/src/library_files.rs | Source cleanup staging capability unavailable |  |
| src-tauri/src/library_files.rs | Source cleanup staging directory disappeared; remaining files were preserved |  |
| src-tauri/src/library_files.rs | Source cleanup staging directory was replaced |  |
| src-tauri/src/library_files.rs | Source name reappeared during cleanup; both files were preserved |  |
| src-tauri/src/library_files.rs | Split archive has ambiguous case-colliding part names |  |
| src-tauri/src/library_files.rs | Split archive selection is incomplete: {} requires its .zimaa primary and full cohort |  |
| src-tauri/src/library_files.rs | Staged archive changed identity |  |
| src-tauri/src/library_files.rs | Staged archive part changed; deletion stopped |  |
| src-tauri/src/library_files.rs | Staged copy changed; source retained |  |
| src-tauri/src/library_files.rs | The exact selected archive primary is missing |  |
| src-tauri/src/library_files.rs | The file at this archive's location is a different archive. Use Locate to find the right file |  |
| src-tauri/src/library_files.rs | The staging folder was replaced and was preserved at {} |  |
| src-tauri/src/library_files.rs | This action could not be verified. Try again |  |
| src-tauri/src/library_files.rs | This archive is no longer in the library, so its unfinished move was dropped. Any of its files already in either folder were left in place.{leftover} |  |
| src-tauri/src/library_files.rs | This archive is not owned by the selected download |  |
| src-tauri/src/library_files.rs | This archive's file name {reason}. Kiwix can read it, but will not move, delete or share a file with such a name. Rename the file and use Locate to find it again; to remove it instead, remove the archive from the library and delete the file yourself |  |
| src-tauri/src/library_files.rs | This attachment type cannot be opened safely in an external application. Use Save attachment instead. |  |
| src-tauri/src/library_files.rs | This mobile platform cannot reveal archive folders in an external file manager. Use Export to save a copy. |  |
| src-tauri/src/library_files.rs | This mobile platform has no supported OS Trash. Disable Move deleted archives to Trash and explicitly confirm permanent deletion. |  |
| src-tauri/src/library_files.rs | This staged copy changed while it was being removed |  |
| src-tauri/src/library_files.rs | This staged copy is not the archive that is already in the library; nothing was removed |  |
| src-tauri/src/library_files.rs | Too many migration journals; automatic recovery stopped without sweeping files |  |
| src-tauri/src/library_files.rs | Unowned migration destination collision; source retained |  |
| src-tauri/src/library_files.rs | Unowned or invalid migration journal; sources were not touched |  |
| src-tauri/src/library_files.rs | Unowned source cleanup entries preserved at {} |  |
| src-tauri/src/library_files.rs | Unsafe managed archive relative directory |  |
| src-tauri/src/library_files.rs | Verified migration archive changed before backup policy |  |
| src-tauri/src/library_files.rs | ZIM\x04 |  |
| src-tauri/src/library_files.rs | attachment-exports-v1 |  |
| src-tauri/src/library_files.rs | from |  |
| src-tauri/src/library_files.rs | moved |  |
| src-tauri/src/library_files.rs | skipped |  |
| src-tauri/src/library_files.rs | to |  |
| src-tauri/src/library_files.rs | {HARD_LINKED}. If an earlier move or delete of this archive was interrupted, a hidden second name for the file (.kiwix-renamed-…) may be left in the archive's folder, or in the folder it was last moved from; delete that hidden entry and try again |  |
| src-tauri/src/library_files.rs | {error}. Any retained archive parts are recoverable at {}. {} |  |
| src-tauri/src/library_files.rs | {error}. Retained cleanup entries are recoverable at {}. {} |  |
| src-tauri/src/library_files.rs | {error}. The unfinished move is kept and will be tried again at the next start: {cleanup} |  |
| src-tauri/src/library_files.rs | {error}. This archive was not moved and is still in {} |  |
| src-tauri/src/library_files.rs | {}.{}.tmp |  |
| src-tauri/src/library_files.rs | {}aa |  |
| src-tauri/src/library_root.rs | .kiwix-library-marker-{} |  |
| src-tauri/src/library_root.rs | .kiwix-library-root-{} |  |
| src-tauri/src/library_root.rs | .{name}.{}.tmp |  |
| src-tauri/src/library_root.rs | A copied archive did not pass verification; the previous library was kept |  |
| src-tauri/src/library_root.rs | A library archive changed during the storage move |  |
| src-tauri/src/library_root.rs | A library archive was removed during the storage move |  |
| src-tauri/src/library_root.rs | A previous storage move is still pending. Restart Kiwix to finish it before changing the location again. |  |
| src-tauri/src/library_root.rs | A selected archive changed during copying |  |
| src-tauri/src/library_root.rs | A selected archive failed verification; no files were moved |  |
| src-tauri/src/library_root.rs | A selected archive is already at that location |  |
| src-tauri/src/library_root.rs | A storage operation is pending. Restart Kiwix to finish recovery first |  |
| src-tauri/src/library_root.rs | An archive was selected more than once |  |
| src-tauri/src/library_root.rs | An original archive changed; copies and recovery journal were preserved |  |
| src-tauri/src/library_root.rs | An unrecognized file was preserved at {} |  |
| src-tauri/src/library_root.rs | Archive directory contains an unsafe component |  |
| src-tauri/src/library_root.rs | Archive disappeared |  |
| src-tauri/src/library_root.rs | Archive ownership requires a regular file |  |
| src-tauri/src/library_root.rs | Archive ownership requires a regular file without hard links |  |
| src-tauri/src/library_root.rs | Archive part changed before it could be copied |  |
| src-tauri/src/library_root.rs | Archive part changed size while copying |  |
| src-tauri/src/library_root.rs | Archive part filename must be valid Unicode |  |
| src-tauri/src/library_root.rs | Archive part length overflow |  |
| src-tauri/src/library_root.rs | Archive parts are not in the same directory |  |
| src-tauri/src/library_root.rs | Archive parts must not be empty |  |
| src-tauri/src/library_root.rs | Archive source was replaced while copying |  |
| src-tauri/src/library_root.rs | Changed file preserved at {}: {e} |  |
| src-tauri/src/library_root.rs | Checking for a pending storage move: {err} |  |
| src-tauri/src/library_root.rs | Checking library location: {error} |  |
| src-tauri/src/library_root.rs | Choose one of the storage locations this app exposes |  |
| src-tauri/src/library_root.rs | Copied provider digest is missing; original and journal were preserved |  |
| src-tauri/src/library_root.rs | Creating library location: {error} |  |
| src-tauri/src/library_root.rs | Creating library staging directory: {e} |  |
| src-tauri/src/library_root.rs | Invalid library filename |  |
| src-tauri/src/library_root.rs | Kiwix could not confirm permission to delete the original, so it was kept |  |
| src-tauri/src/library_root.rs | Kiwix could not confirm which original files belong to this move. The files were kept |  |
| src-tauri/src/library_root.rs | Library destination changed |  |
| src-tauri/src/library_root.rs | Library destination changed before adoption |  |
| src-tauri/src/library_root.rs | Library destination path is not UTF-8 |  |
| src-tauri/src/library_root.rs | Library location marker is invalid |  |
| src-tauri/src/library_root.rs | Library location marker is oversized |  |
| src-tauri/src/library_root.rs | Library location marker must be a regular file |  |
| src-tauri/src/library_root.rs | Library location must be an absolute normalized path |  |
| src-tauri/src/library_root.rs | Library locations cannot contain one another |  |
| src-tauri/src/library_root.rs | Library marker has no filename |  |
| src-tauri/src/library_root.rs | Library migration journal is missing a part |  |
| src-tauri/src/library_root.rs | Library size overflow |  |
| src-tauri/src/library_root.rs | Library staging directory changed |  |
| src-tauri/src/library_root.rs | Managed archive has no directory below the library root |  |
| src-tauri/src/library_root.rs | Missing archive parent |  |
| src-tauri/src/library_root.rs | Missing staged directory identity |  |
| src-tauri/src/library_root.rs | Native external storage location is not absolute |  |
| src-tauri/src/library_root.rs | Native storage enumeration returned no locations |  |
| src-tauri/src/library_root.rs | Native storage enumeration returned too many locations |  |
| src-tauri/src/library_root.rs | Native storage location has an unknown kind |  |
| src-tauri/src/library_root.rs | Native storage location is not an absolute normalized path |  |
| src-tauri/src/library_root.rs | Not enough space at the new location: {needed} bytes are needed but {free} bytes are free |  |
| src-tauri/src/library_root.rs | Only app-managed archives or native provider selections can be copied or moved |  |
| src-tauri/src/library_root.rs | Opening library location marker: {error} |  |
| src-tauri/src/library_root.rs | Private mode cannot copy or move the library |  |
| src-tauri/src/library_root.rs | Private or read-only mode cannot copy or move archives |  |
| src-tauri/src/library_root.rs | Provider deletion is unavailable on this platform; original and journal were preserved |  |
| src-tauri/src/library_root.rs | Provider move receipt is invalid |  |
| src-tauri/src/library_root.rs | Provider move receipt is missing |  |
| src-tauri/src/library_root.rs | Provider source disappeared |  |
| src-tauri/src/library_root.rs | Provider source missing |  |
| src-tauri/src/library_root.rs | Read-only mode cannot change the library location |  |
| src-tauri/src/library_root.rs | Reading archive part size: {error} |  |
| src-tauri/src/library_root.rs | Reading storage migration journal: {error} |  |
| src-tauri/src/library_root.rs | Reconnect the selected archive before copying it |  |
| src-tauri/src/library_root.rs | Replacement archive is missing or changed; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs | Replacement archive is no longer admitted; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs | Replacement archive part set changed; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs | Replacement installation changed; originals and recovery journal were preserved |  |
| src-tauri/src/library_root.rs | Reserved staged archive changed before copying |  |
| src-tauri/src/library_root.rs | Select at least one archive to copy or move |  |
| src-tauri/src/library_root.rs | Select fewer archives: the storage recovery journal would be too large |  |
| src-tauri/src/library_root.rs | Selected archive has no parent directory |  |
| src-tauri/src/library_root.rs | Selected archive has no parts |  |
| src-tauri/src/library_root.rs | Selected archive is no longer in the library |  |
| src-tauri/src/library_root.rs | Selected archive source is missing |  |
| src-tauri/src/library_root.rs | Selected archives changed during recovery; all copies and the journal were preserved |  |
| src-tauri/src/library_root.rs | Selected source part disappeared |  |
| src-tauri/src/library_root.rs | Staged archive changed before verification |  |
| src-tauri/src/library_root.rs | Storage migration journal belongs to a different profile |  |
| src-tauri/src/library_root.rs | Storage migration journal has an empty part |  |
| src-tauri/src/library_root.rs | Storage migration journal has an invalid ID |  |
| src-tauri/src/library_root.rs | Storage migration journal has an invalid archive ID |  |
| src-tauri/src/library_root.rs | Storage migration journal has an invalid part count |  |
| src-tauri/src/library_root.rs | Storage migration journal has an invalid staging name |  |
| src-tauri/src/library_root.rs | Storage migration journal has an unsupported version |  |
| src-tauri/src/library_root.rs | Storage migration journal has invalid roots |  |
| src-tauri/src/library_root.rs | Storage migration journal has overlapping roots |  |
| src-tauri/src/library_root.rs | Storage migration journal has too many entries |  |
| src-tauri/src/library_root.rs | Storage migration journal is oversized |  |
| src-tauri/src/library_root.rs | Storage selection entry is missing |  |
| src-tauri/src/library_root.rs | Storage selection has an invalid installation |  |
| src-tauri/src/library_root.rs | Storage selection has an invalid source count |  |
| src-tauri/src/library_root.rs | Storage selection has duplicate or missing sources |  |
| src-tauri/src/library_root.rs | Storage selection is missing |  |
| src-tauri/src/library_root.rs | That folder belongs to a different Kiwix profile |  |
| src-tauri/src/library_root.rs | The copied archive changed before adoption |  |
| src-tauri/src/library_root.rs | The current library location is not an absolute normalized path |  |
| src-tauri/src/library_root.rs | The library contains too many archives for one storage move |  |
| src-tauri/src/library_root.rs | The library location is not a directory |  |
| src-tauri/src/library_root.rs | The library location must not be a symbolic link |  |
| src-tauri/src/library_root.rs | The managed copy is ready, but its original is retained for edition rollback; keep the original archive and its provider permission |  |
| src-tauri/src/library_root.rs | The original archive disappeared |  |
| src-tauri/src/library_root.rs | The selected storage folder was replaced |  |
| src-tauri/src/library_root.rs | The staged archive was replaced |  |
| src-tauri/src/library_root.rs | The storage migration journal disappeared |  |
| src-tauri/src/library_root.rs | This platform cannot delete files from that document provider. Choose Copy instead |  |
| src-tauri/src/library_root.rs | archives |  |
| src-tauri/src/library_root.rs | external |  |
| src-tauri/src/library_root.rs | internal |  |
| src-tauri/src/library_root.rs | profile |  |
| src-tauri/src/library_root.rs | removedOld |  |
| src-tauri/src/library_root.rs | sha256 |  |
| src-tauri/src/library_root.rs | size |  |
| src-tauri/src/library_root.rs | version |  |
| src-tauri/src/library_root.rs | {error}. Copies and the recovery journal were preserved; restart Kiwix to finish recovery |  |
| src-tauri/src/library_root.rs | {error}. Copies and the recovery journal were preserved; restart Kiwix to finish the storage move. |  |
| src-tauri/src/library_root.rs | {error}. Partial copies were preserved for recovery: {rollback} |  |
| src-tauri/src/live_activity.rs | liveActivity |  |
| src-tauri/src/live_activity.rs | localizer was just cached |  |
| src-tauri/src/localswarm/apple_transport.rs | ../../native/localswarm.p12 |  |
| src-tauri/src/localswarm/apple_transport.rs | Apple QUIC listener callback lost |  |
| src-tauri/src/localswarm/apple_transport.rs | Apple QUIC receive exceeded its request |  |
| src-tauri/src/localswarm/apple_transport.rs | Apple QUIC send completion length mismatch |  |
| src-tauri/src/localswarm/apple_transport.rs | Cannot create Apple QUIC connection |  |
| src-tauri/src/localswarm/apple_transport.rs | Cannot create Apple QUIC listener |  |
| src-tauri/src/localswarm/apple_transport.rs | Cannot import the pinned LocalSwarm Apple identity |  |
| src-tauri/src/localswarm/apple_transport.rs | Invalid Apple QUIC listener port |  |
| src-tauri/src/localswarm/apple_transport.rs | Invalid Bonjour endpoint |  |
| src-tauri/src/localswarm/apple_transport.rs | Invalid Bonjour service instance |  |
| src-tauri/src/localswarm/apple_transport.rs | LocalSwarm Apple QUIC connection failed or left the local link |  |
| src-tauri/src/localswarm/apple_transport.rs | LocalSwarm operation cancelled |  |
| src-tauri/src/localswarm/apple_transport.rs | Missing LocalSwarm Apple certificate |  |
| src-tauri/src/localswarm/apple_transport.rs | Native QUIC has no live effective local address |  |
| src-tauri/src/localswarm/apple_transport.rs | Native QUIC has no live required interface |  |
| src-tauri/src/localswarm/apple_transport.rs | Not a Bonjour endpoint |  |
| src-tauri/src/localswarm/apple_transport.rs | The selected native QUIC interface changed |  |
| src-tauri/src/localswarm/apple_transport.rs | infrastructure |  |
| src-tauri/src/localswarm/apple_transport.rs | peer-to-peer |  |
| src-tauri/src/localswarm/apple_transport/read_request.rs | Apple QUIC receive IDs exhausted |  |
| src-tauri/src/localswarm/cleanup.rs | .kiwix-localswarm-{suffix}.cleanup |  |
| src-tauri/src/localswarm/cleanup.rs | A LocalSwarm worker is still using this download; stop it before deleting files |  |
| src-tauri/src/localswarm/cleanup.rs | An unowned file appeared in LocalSwarm cleanup; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | Both LocalSwarm cleanup and source names exist; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | Both LocalSwarm cleanup and source names exist; cleanup ownership must be resolved first |  |
| src-tauri/src/localswarm/cleanup.rs | Cannot commit LocalSwarm cleanup receipt: {}; history was retained |  |
| src-tauri/src/localswarm/cleanup.rs | Files appeared after this LocalSwarm cleanup completed; they cannot authorize library removal |  |
| src-tauri/src/localswarm/cleanup.rs | Invalid LocalSwarm cleanup ID |  |
| src-tauri/src/localswarm/cleanup.rs | Invalid LocalSwarm cleanup ownership receipt |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm cleanup directory ownership changed; all entries were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm cleanup file ownership changed; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm cleanup file was replaced; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm cleanup has no ownership receipt for {}; all entries were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm cleanup parent was replaced; retained files were not touched |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm cleanup receipt does not identify the retained files |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm file changed after cleanup confirmation; all remaining files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm file disappeared before cleanup capture; ownership was retained |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm has no publication ownership proof for the existing destination |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm job not found |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm partial files disappeared before cleanup; ownership was retained |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm publication changed before library-removal proof was retained |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm publication changed while library-removal proof was retained |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm retained bytes changed during cleanup admission |  |
| src-tauri/src/localswarm/cleanup.rs | LocalSwarm transfer is no longer terminal; files were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | Missing LocalSwarm cleanup file receipt |  |
| src-tauri/src/localswarm/cleanup.rs | Missing LocalSwarm original publication digests |  |
| src-tauri/src/localswarm/cleanup.rs | Missing LocalSwarm publication ownership stamp |  |
| src-tauri/src/localswarm/cleanup.rs | Published LocalSwarm archive changed; remove it explicitly from the library instead |  |
| src-tauri/src/localswarm/cleanup.rs | Stop this LocalSwarm transfer before deleting its files |  |
| src-tauri/src/localswarm/cleanup.rs | Stop this LocalSwarm transfer before forgetting its history |  |
| src-tauri/src/localswarm/cleanup.rs | Stop this LocalSwarm transfer before removing its history |  |
| src-tauri/src/localswarm/cleanup.rs | Stop this LocalSwarm worker before preparing file cleanup |  |
| src-tauri/src/localswarm/cleanup.rs | Unexpected entries appeared during LocalSwarm cleanup; directory was preserved |  |
| src-tauri/src/localswarm/cleanup.rs | Unexpected entries in LocalSwarm cleanup directory; all entries were preserved |  |
| src-tauri/src/localswarm/cleanup.rs | localswarm: |  |
| src-tauri/src/localswarm/discovery.rs | Cannot advertise an unreachable LocalSwarm listener address |  |
| src-tauri/src/localswarm/discovery.rs | Cannot advertise an unreachable library listener address |  |
| src-tauri/src/localswarm/discovery.rs | Cannot browse local network: {e} |  |
| src-tauri/src/localswarm/discovery.rs | Cannot stop LocalSwarm discovery: {e} |  |
| src-tauri/src/localswarm/discovery.rs | Find nearby before browsing a shared library |  |
| src-tauri/src/localswarm/discovery.rs | Invalid LocalSwarm peer ID |  |
| src-tauri/src/localswarm/discovery.rs | Invalid library listener address |  |
| src-tauri/src/localswarm/discovery.rs | Invalid library session, listener port, or advertisement limit |  |
| src-tauri/src/localswarm/discovery.rs | Invalid listener address |  |
| src-tauri/src/localswarm/discovery.rs | Invalid owned LocalSwarm announcement |  |
| src-tauri/src/localswarm/discovery.rs | Invalid publisher name |  |
| src-tauri/src/localswarm/discovery.rs | Kiwix-{} |  |
| src-tauri/src/localswarm/discovery.rs | Library sharing requires actual listener addresses |  |
| src-tauri/src/localswarm/discovery.rs | Local network discovery stopped unexpectedly |  |
| src-tauri/src/localswarm/discovery.rs | LocalSwarm advertisement limit or invalid port |  |
| src-tauri/src/localswarm/discovery.rs | LocalSwarm discovery daemon stopped |  |
| src-tauri/src/localswarm/discovery.rs | LocalSwarm discovery did not stop |  |
| src-tauri/src/localswarm/discovery.rs | LocalSwarm discovery is stopped |  |
| src-tauri/src/localswarm/discovery.rs | LocalSwarm discovery shutdown failed |  |
| src-tauri/src/localswarm/discovery.rs | LocalSwarm discovery shutdown timed out |  |
| src-tauri/src/localswarm/discovery.rs | LocalSwarm unadvertise timed out |  |
| src-tauri/src/localswarm/discovery.rs | No bounded reachable IPv4 library listener addresses |  |
| src-tauri/src/localswarm/discovery.rs | No bounded reachable library listener addresses |  |
| src-tauri/src/localswarm/discovery.rs | No reachable IPv4 library listener address for this advertisement |  |
| src-tauri/src/localswarm/discovery.rs | No reachable local network address; use a direct connection instead |  |
| src-tauri/src/localswarm/discovery.rs | Stop the failed discovery session before restarting |  |
| src-tauri/src/localswarm/discovery.rs | The previous library announcement is still stopping; stop sharing before starting it again |  |
| src-tauri/src/localswarm/discovery.rs | The shared library is no longer available; find nearby and browse again |  |
| src-tauri/src/localswarm/discovery.rs | The shared library is no longer nearby; find nearby and browse again |  |
| src-tauri/src/localswarm/discovery.rs | kiwix-{}.local. |  |
| src-tauri/src/localswarm/discovery.rs | {fullname}#{} |  |
| src-tauri/src/localswarm/import.rs | Archive publication verification was cancelled |  |
| src-tauri/src/localswarm/import.rs | LocalSwarm is disabled in private/read-only mode |  |
| src-tauri/src/localswarm/import.rs | LocalSwarm state exceeds its size limit |  |
| src-tauri/src/localswarm/import.rs | Nearby download bytes changed after publication; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs | Nearby download contains unexpected files; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs | Nearby download ended before its verified size; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs | Nearby download file was replaced or changed; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs | Nearby download history changed during import; retry the import |  |
| src-tauri/src/localswarm/import.rs | Nearby download no longer exists |  |
| src-tauri/src/localswarm/import.rs | Nearby download ownership or split parts changed; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs | Nearby download parent or publication changed; files were left unchanged |  |
| src-tauri/src/localswarm/import.rs | Nearby download size does not match its manifest |  |
| src-tauri/src/localswarm/import.rs | Nearby publication changed during verification; retry the import |  |
| src-tauri/src/localswarm/import.rs | Nearby publication has no archive |  |
| src-tauri/src/localswarm/import.rs | This nearby download is no longer available for import |  |
| src-tauri/src/localswarm/import.rs | localswarm |  |
| src-tauri/src/localswarm/ios_discovery.rs | Bonjour operation limit reached |  |
| src-tauri/src/localswarm/ios_discovery.rs | Bonjour polling failed: {} |  |
| src-tauri/src/localswarm/ios_discovery.rs | Bonjour returned no operation |  |
| src-tauri/src/localswarm/ios_discovery.rs | Invalid library hostname |  |
| src-tauri/src/localswarm/ios_discovery.rs | Invalid library session, count or listener port |  |
| src-tauri/src/localswarm/ios_discovery.rs | LocalSwarm TXT exceeds limit |  |
| src-tauri/src/localswarm/ios_discovery.rs | The previous announcement is still stopping; stop sharing before starting it again |  |
| src-tauri/src/localswarm/ios_discovery.rs | {name}.{SERVICE} |  |
| src-tauri/src/localswarm/library.rs | Add an available archive before sharing your library |  |
| src-tauri/src/localswarm/library.rs | Archive installation is no longer shared |  |
| src-tauri/src/localswarm/library.rs | Invalid installed archive identifier |  |
| src-tauri/src/localswarm/library.rs | Invalid library completion acknowledgement |  |
| src-tauri/src/localswarm/library.rs | Invalid library data frame length |  |
| src-tauri/src/localswarm/library.rs | Invalid library inventory book |  |
| src-tauri/src/localswarm/library.rs | Invalid library inventory identity |  |
| src-tauri/src/localswarm/library.rs | Invalid library range response |  |
| src-tauri/src/localswarm/library.rs | Invalid library response metadata |  |
| src-tauri/src/localswarm/library.rs | Library completion acknowledgement timed out |  |
| src-tauri/src/localswarm/library.rs | Library inventory is missing |  |
| src-tauri/src/localswarm/library.rs | Library range response does not match the requested validator and bounds |  |
| src-tauri/src/localswarm/library.rs | Library response was interrupted |  |
| src-tauri/src/localswarm/library.rs | Library session changed |  |
| src-tauri/src/localswarm/library.rs | Missing library session |  |
| src-tauri/src/localswarm/library.rs | Native Apple service endpoints are unavailable on this platform |  |
| src-tauri/src/localswarm/library.rs | Nearby library is unavailable |  |
| src-tauri/src/localswarm/library.rs | Nearby sharing stopped |  |
| src-tauri/src/localswarm/library.rs | Shared archive changed or is no longer available | shared archive |
| src-tauri/src/localswarm/library.rs | Shared archive is unavailable or changed | shared archive |
| src-tauri/src/localswarm/library.rs | The library exceeds the Nearby inventory limit |  |
| src-tauri/src/localswarm/library.rs | The library inventory exceeds its transport bound |  |
| src-tauri/src/localswarm/library.rs | The selected Nearby library is no longer available |  |
| src-tauri/src/localswarm/library.rs | The selected Nearby library session changed |  |
| src-tauri/src/localswarm/library.rs | The selected Nearby share retired | retired |
| src-tauri/src/localswarm/library.rs | The shared library session retired | retired |
| src-tauri/src/localswarm/library.rs | Trailing library range payload |  |
| src-tauri/src/localswarm/library.rs | Truncated library range |  |
| src-tauri/src/localswarm/library.rs | Truncated library range payload |  |
| src-tauri/src/localswarm/library.rs | Unsolicited library range response |  |
| src-tauri/src/localswarm/library.rs | content-length |  |
| src-tauri/src/localswarm/library.rs | macos |  |
| src-tauri/src/localswarm/library_route.rs | Invalid native QUIC interface receipt |  |
| src-tauri/src/localswarm/library_route.rs | Invalid selected interface |  |
| src-tauri/src/localswarm/library_route.rs | The bound QUIC local address has no live interface |  |
| src-tauri/src/localswarm/library_route.rs | The selected Nearby interface retired | retired |
| src-tauri/src/localswarm/library_route.rs | The selected Nearby interface was replaced |  |
| src-tauri/src/localswarm/library_route.rs | The selected Nearby local address retired; browse again | retired |
| src-tauri/src/localswarm/library_route.rs | The selected Nearby route changed; browse again |  |
| src-tauri/src/localswarm/library_route.rs | The selected Nearby route is no longer local |  |
| src-tauri/src/localswarm/library_route.rs | The selected local address belongs to multiple interfaces |  |
| src-tauri/src/localswarm/library_route.rs | The selected native interface has no index |  |
| src-tauri/src/localswarm/library_transport.rs | Library completion failed: {reason} |  |
| src-tauri/src/localswarm/library_transport.rs | Native QUIC supplied no exact route receipt |  |
| src-tauri/src/localswarm/library_transport.rs | Nearby joined route retired | retired |
| src-tauri/src/localswarm/library_transport.rs | Nearby local policy changed |  |
| src-tauri/src/localswarm/library_transport.rs | Nearby route retired | retired |
| src-tauri/src/localswarm/library_transport.rs | Nearby share retired | retired |
| src-tauri/src/localswarm/library_transport.rs | No nearby QUIC endpoint |  |
| src-tauri/src/localswarm/library_transport.rs | The selected native QUIC interface is unavailable |  |
| src-tauri/src/localswarm/library_transport.rs | The supplied library socket does not own its advertised address |  |
| src-tauri/src/localswarm/library_transport.rs | Trailing library response payload |  |
| src-tauri/src/localswarm/library_transport.rs | made no progress |  |
| src-tauri/src/localswarm/library_transport.rs | not a local peer |  |
| src-tauri/src/localswarm/library_wire.rs | Invalid library control length |  |
| src-tauri/src/localswarm/library_wire.rs | Invalid library control: {e} |  |
| src-tauri/src/localswarm/library_wire.rs | Invalid library range |  |
| src-tauri/src/localswarm/library_wire.rs | Invalid library request |  |
| src-tauri/src/localswarm/library_wire.rs | Library control exceeds its bound |  |
| src-tauri/src/localswarm/library_wire.rs | Unsatisfiable library range |  |
| src-tauri/src/localswarm/library_wire.rs | bytes= |  |
| src-tauri/src/localswarm/mod.rs | } else { |  |
| src-tauri/src/localswarm/mod.rs | }.into());                     }                 }             }             let initial_route = session.route().map(str::to_owned);             let manifest = session.into_manifest().await;             check_cancel(&cancel)?;             (cancel, host, initial_route, manifest)         };         let _operation = self.inner.operation.lock().await;         self.allowed()?;         check_cancel(&cancel)?;         if self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()).jobs.len() >= MAX_JOBS {             return Err( |  |
| src-tauri/src/localswarm/mod.rs | && (cancelled \|\| !matches!(saved.job.status.as_str(), |  |
| src-tauri/src/localswarm/mod.rs | {                         let cancelled = token.is_cancelled();                         saved.job.status = if cancelled { | token |
| src-tauri/src/localswarm/mod.rs | { return Ok(()); }         if saved.job.requires_pin && pin.as_deref().is_none_or(str::is_empty) { return Err( |  |
| src-tauri/src/localswarm/mod.rs | \| |  |
| src-tauri/src/localswarm/mod.rs | } else { |  |
| src-tauri/src/localswarm/mod.rs | }.into())     }     fn unchanged(&self) -> Result<(), String> {         self.principal()?;         let retained = self.retained.read().map_err(\|_\| |  |
| src-tauri/src/localswarm/mod.rs | }.into(); saved.job.speed = 0; saved.job.eta_seconds = None; saved.job.error = None; saved.job.updated_at = now(); }         self.persist(&state).map_err(Into::into)     }     pub async fn resume(&self, id: &str, pin: Option<String>, acknowledgement: Option<String>) -> Result<(), String> {         self.allowed()?; let _operation = self.inner.operation.lock().await;         if self.inner.state.lock().map_err(\|_\| |  |
| src-tauri/src/localswarm/mod.rs | ) { Err(e) if e.kind() == std::io::ErrorKind::NotFound => directory.file( |  |
| src-tauri/src/localswarm/mod.rs | ) { Err(e) if e.kind() == std::io::ErrorKind::NotFound => profile.child( |  |
| src-tauri/src/localswarm/mod.rs | ) { Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()), Err(e) => return Err(err(e)), Ok(_) => () }         let directory = profile.child( |  |
| src-tauri/src/localswarm/mod.rs | ) { saved.job.status = |  |
| src-tauri/src/localswarm/mod.rs | ))             .collect::<Result<Vec<_>, _>>()?;         saved.owned.parent = PathBuf::from(owned.get( |  |
| src-tauri/src/localswarm/mod.rs | )) { saved.job.status = if cancelled { |  |
| src-tauri/src/localswarm/mod.rs | )) }     }     pub async fn revoke_invalid_sources(&self) -> Result<(), String> {         let _guard = self.inner.operation.lock().await;         let invalid_library = self.inner.library.lock().await.as_ref().is_some_and(\|offer\| !offer.server.valid_sources());         if invalid_library {             let stopped = self.stop_library_locked().await;             self.note_library_stopped();             stopped?;         }         let offers = {             let mut offers = self.inner.offers.lock().await;             let ids: Vec<_> = offers.iter().filter(\|(_, offer)\| offer.source.principal().is_err()).map(\|(id, _)\| id.clone()).collect();             ids.into_iter().filter_map(\|id\| offers.remove(&id)).collect::<Vec<_>>()         };         for offer in &offers { offer.cancel.cancel(); }         let mut errors = Vec::new();         for offer in offers { if let Err(error) = Self::finish_offer(offer).await { errors.push(error); } }         if errors.is_empty() { Ok(()) } else { Err(errors.join( |  |
| src-tauri/src/localswarm/mod.rs | )) }     }     pub async fn stop(&self) -> Result<Value, String> {         self.inner.lifetime.lock().unwrap_or_else(\|e\| e.into_inner()).cancel();         let _seed_fence = self.inner.seed_setup_gate.write().await;         let _receive_fence = self.inner.receive_setup_gate.write().await;         let _operation = self.inner.operation.lock().await;         let mut errors: Vec<_> = self.stop_library_locked().await.err().into_iter().collect();         *self.inner.library_stopped.lock().unwrap_or_else(\|e\| e.into_inner()) = None;         let ids: Vec<_> = self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()).running.keys().cloned().collect();         for id in ids { if let Err(e) = self.pause_locked(&id, false).await { errors.push(e); } }         let offers = std::mem::take(&mut *self.inner.offers.lock().await);         for (_, offer) in offers { if let Err(error) = Self::finish_offer(offer).await { errors.push(error); } }         if let Err(e) = self.inner.discovery.lock().await.shutdown().await { errors.push(e); }         if errors.is_empty() { self.status().await } else { Err(errors.join( |  |
| src-tauri/src/localswarm/mod.rs | )) }     }     pub async fn stop_seed(&self, id: &str) -> Result<(), String> {         let _operation = self.inner.operation.lock().await; let offer = self.inner.offers.lock().await.remove(id).ok_or( |  |
| src-tauri/src/localswarm/mod.rs | )) }     } } fn verified_bytes(manifest: &Manifest, bits: &[bool]) -> Result<u64, String> { bits.iter().enumerate().filter(\|(_, b)\| **b).try_fold(0u64, \|n, (i, _)\| n.checked_add(manifest.chunk_len(i as u32)? as u64).ok_or_else(\|\| |  |
| src-tauri/src/localswarm/mod.rs | ), &directory)).map_err(err)?;         let nonce = self.inner.last_start_manifest.lock().unwrap_or_else(\|e\| e.into_inner()).as_mut()             .filter(\|(key, _, _)\| key == &request).map(\|(_, nonce, _)\| {                 nonce.get_or_insert_with(\|\| Uuid::new_v4().to_string()).clone()             }).unwrap_or_else(\|\| Uuid::new_v4().to_string());         let identity = serde_json::to_string(&(request.as_str(), nonce.as_str())).map_err(err)?;         let directory = blocking(move \|\| {             crate::fs_ext::create_private_dir_all(&directory).map_err(err)?;             let directory = std::fs::canonicalize(&directory).map_err(err)?;             Directory::open(&directory)         }).await?;         *self.inner.last_start_manifest.lock().unwrap_or_else(\|e\| e.into_inner()) = Some((request, Some(nonce), manifest.clone()));         let space_identity = identity.clone(); let space_acknowledgement = acknowledgement.clone();         let suffix = Uuid::new_v4().to_string(); let id = format!( | uuid |
| src-tauri/src/localswarm/mod.rs | ).and_then(Value::as_array)             .ok_or( |  |
| src-tauri/src/localswarm/mod.rs | ).and_then(Value::as_str)             .ok_or( |  |
| src-tauri/src/localswarm/mod.rs | ).ok_or( |  |
| src-tauri/src/localswarm/mod.rs | ); }             }         });         self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()).running.insert(id.to_string(), Running { cancel, task, progress: None });         let _ = start.send(());         Ok(())     }     pub async fn pause(&self, id: &str, cancelled: bool) -> Result<(), String> {         self.allowed()?; let _operation = self.inner.operation.lock().await; self.pause_locked(id, cancelled).await     }     async fn pause_locked(&self, id: &str, cancelled: bool) -> Result<(), String> {         let running = self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()).running.remove(id);         if let Some(running) = running { running.cancel.cancel(); running.task.await.map_err(\|e\| format!( |  |
| src-tauri/src/localswarm/mod.rs | )?             .iter().map(\|path\| path.as_str().map(str::to_owned).ok_or( |  |
| src-tauri/src/localswarm/mod.rs | )?);     }     Ok(()) }  #[cfg(target_os = |  |
| src-tauri/src/localswarm/mod.rs | )?.publishing = true; self.persist(&state)?; }         let publish_open = open.clone(); let publish_manifest = manifest.clone(); let publish_owned = owned.clone(); let store = self.inner.store.clone(); let token = cancel.clone();         blocking(move \|\| {             check_cancel(&token)?; let mut open = publish_open.lock().map_err(\|_\| | token |
| src-tauri/src/localswarm/mod.rs | )?.to_owned();         saved.job.completed_paths = job.get( |  |
| src-tauri/src/localswarm/mod.rs | )?;             OwnedSet::check_space(&open.parent, &open_manifest, remaining, &space_identity, acknowledgement.as_deref())?;             if !open.published && verified.iter().any(\|bit\| !bit) {                 let donors = crate::torrent::TorrentManager::saved_partial_files(&donor_store).unwrap_or_else(\|error\| {                     eprintln!( |  |
| src-tauri/src/localswarm/mod.rs | )?;             saved.verified.copy_from_slice(verified); saved.job.downloaded = verified_bytes(&saved.manifest, verified)?; saved.job.status = status.into(); saved.job.speed = speed; saved.job.eta_seconds = if speed > 0 { Some((saved.manifest.total_bytes - saved.job.downloaded).div_ceil(speed)) } else { None }; saved.job.updated_at = now();             self.stage(&state)         };         self.write_staged(sequence, &jobs).map_err(Into::into)     }     pub async fn seed(&self, book_id: &str, pin: Option<String>) -> Result<Seed, String> {         self.allowed()?;         let pin = pin.filter(\|pin\| !pin.is_empty());         if self.inner.offers.lock().await.len() >= MAX_SEEDS { return Err( |  |
| src-tauri/src/localswarm/mod.rs | )?;             stamps.push(stamp);         }         let header = zim_header(retained.files())?;         if &header[..4] != b |  |
| src-tauri/src/localswarm/mod.rs | )?;           if pin.is_some() { saved.job.requires_pin = true; } saved.job.status = |  |
| src-tauri/src/localswarm/mod.rs | )?;         Self::finish_offer(offer).await     }     /// Background Android receives may continue under a foreground service,     /// but offers and discovery must not retain a hidden network presence.     #[cfg(target_os = |  |
| src-tauri/src/localswarm/mod.rs | )?;         buf = &mut buf[count..];     }     Ok(()) } |  |
| src-tauri/src/localswarm/mod.rs | )?;         if offer.server.descriptor.session_id != session_id \|\| !offer.server.status().running {             *self.inner.library.lock().await = Some(offer);             return Err( |  |
| src-tauri/src/localswarm/mod.rs | )?;         if saved.job.status != |  |
| src-tauri/src/localswarm/mod.rs | )?;         if saved.job.status == |  |
| src-tauri/src/localswarm/mod.rs | )?;         let StoredJob { manifest, owned, host, port, publishing, direct, mut verified, .. } = saved;         let owned = Arc::new(owned); let open_manifest = manifest.clone(); let open_owned = owned.clone(); let recheck_cancel = cancel.clone();         let donor_store = self.inner.store.clone();         let (open, checked) = blocking(move \|\| {             let mut open = open_owned.open(&open_manifest, publishing)?;             rehash_verified(&mut open, &open_manifest, &open_owned, &mut verified, &recheck_cancel)?;             // Adopted chunks consume the same space as downloaded ones, so the             // space check and its acknowledgement precede any adoption write.             let remaining = open_manifest.total_bytes.checked_sub(verified_bytes(&open_manifest, &verified)?)                 .ok_or( |  |
| src-tauri/src/localswarm/mod.rs | )?;         let job = row.get( |  |
| src-tauri/src/localswarm/mod.rs | )?;         let owned = row.get( |  |
| src-tauri/src/localswarm/mod.rs | )?;         let settings = self.inner.store.settings()?;         // Verify and hash the manifest before taking `operation` so preparing a         // multi-GB offer never blocks pause, resume, discovery, or cleanup,         // while `seed_setup_gate` + `sharing_lifetime` still let `stop()` and         // Android `stop_sharing()` cancel and drain in-flight preparation.         let (cancel, manifest, source) = {             let _setup = self.inner.seed_setup_gate.read().await;             let cancel = self.sharing_cancel_token();             check_cancel(&cancel)?;             let (manifest, source) = SourceSet::prepare(self.inner.store.clone(), book, cancel.clone()).await?;             check_cancel(&cancel)?;             (cancel, manifest, source)         };         let _operation = self.inner.operation.lock().await;         self.allowed()?;         check_cancel(&cancel)?; source.unchanged()?;         if self.inner.offers.lock().await.len() >= MAX_SEEDS {             return Err( | token |
| src-tauri/src/localswarm/mod.rs | )?;         saved.job.destination = job.get( |  |
| src-tauri/src/localswarm/mod.rs | )?;         saved.job.status = |  |
| src-tauri/src/localswarm/mod.rs | )?;         self.inner.directory.atomic( |  |
| src-tauri/src/localswarm/mod.rs | )?;         }         let mut entries = Vec::with_capacity(stamps.len());         let mut hashes = Vec::new();         let mut bytes = vec![0u8; chunk_size as usize];         for ((name, file), stamp) in retained.files_mut().iter_mut().zip(&stamps) {             let start = hashes.len() as u32;             let mut left = stamp.size;             file.seek(SeekFrom::Start(0)).map_err(err)?;             while left > 0 {                 check_cancel(&cancel)?;                 let length = left.min(u64::from(chunk_size)) as usize;                 file.read_exact(&mut bytes[..length]).map_err(err)?;                 hashes.push(hex::encode(Sha256::digest(&bytes[..length])));                 left -= length as u64;             }             entries.push(SwarmFile { path: name.clone(), size_bytes: stamp.size,                 start_chunk_index: start, end_chunk_index: hashes.len() as u32 - 1 });         }         retained.unchanged()?;         if retained.identity()? != identity { return Err( |  |
| src-tauri/src/localswarm/mod.rs | )?; open.unchanged(&publish_manifest, &publish_owned)?;             let paths: Vec<_> = publish_manifest.files.iter().map(\|f\| publish_owned.final_path(&f.path)).collect();             store.with_acquisition_destinations(&paths, \|\| {                 check_cancel(&token)?;                 if !open.published {                     let files = publish_manifest.files.iter().zip(&open.files).map(\|(entry, file)\| Ok((PathBuf::from(&entry.path), crate::publication::VerifiedFile::new(file.try_clone().map_err(err)?, entry.size_bytes).map_err(err)?))).collect::<Result<Vec<_>, String>>()?;                     open.parent.unchanged()?; open.unchanged(&publish_manifest, &publish_owned)?;                     open.publish(&publish_manifest, &publish_owned, files)?;                 }                 // Also cover a restart after the rename but before its sync.                 open.parent.sync()?;                 for (file, path) in open.files.iter().zip(&paths) { crate::mobile_backup::archive_policy(file, path)?; }                 open.unchanged(&publish_manifest, &publish_owned)             })         }).await?;         let mut state = self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()); let saved = state.jobs.get_mut(id).ok_or( | token |
| src-tauri/src/localswarm/mod.rs | )]         profile::configure();         let (path, dir) = store.acquisition_profile_directory()?;         let identity = storage::Identity::of(&dir.dir_metadata().map_err(err)?);         let profile = Directory { path, dir, identity };         let directory = match profile.dir.symlink_metadata( |  |
| src-tauri/src/localswarm/mod.rs | )]         server.stats.set_accepting(self.inner.sharing_admission.load(std::sync::atomic::Ordering::SeqCst));         let advertisement = match self.inner.discovery.lock().await.advertise_addresses(&manifest, server.port, pin.is_some(), &server.addresses, crate::publisher::name(&settings)).await { Ok(id) => id, Err(e) => { server.shutdown().await; return Err(e); } };         let row = Seed { id: format!( |  |
| src-tauri/src/localswarm/mod.rs | , 64)? { let id = String::from_utf8(bytes).map_err(err)?; if Uuid::parse_str(&id).map_err(err)?.to_string() != id { return Err( | uuid |
| src-tauri/src/localswarm/mod.rs | , Uuid::new_v4()), book_id: book_id.into(), name: manifest.name.clone(), swarm_id: manifest.swarm_id.clone(), total_bytes: manifest.total_bytes, chunk_count: manifest.chunk_hashes.len(), port: server.port, locked: pin.is_some(), uploaded: 0, upload_speed: 0, peer_count: 0, addresses: server.addresses.clone() };         let stats = server.stats.clone(); let offer_id = row.id.clone();         let weak = Arc::downgrade(&self.inner); let supervisor_source = source.clone();         let (start, started) = tokio::sync::oneshot::channel();         let task = tokio::spawn(async move {             let _ = started.await;             Self::supervise_offer(weak, offer_id, supervisor_source, reason, monitor, server, advertisement).await         });         *self.inner.error.lock().unwrap_or_else(\|e\| e.into_inner()) = None;         let mut offers = self.inner.offers.lock().await;         #[cfg(target_os = | uuid |
| src-tauri/src/localswarm/mod.rs | , donor.path.display()); continue; }             };             let (mut matched, mut mismatched) = (0usize, 0usize);             for index in start..=end {                 if cancel.is_cancelled() { return Ok(adopted); }                 if bits[index as usize] { continue; }                 let offset = u64::from(index - start) * chunk;                 let length = manifest.chunk_len(index)?;                 if !crate::partial_donor::covers(donor.ranges.as_deref(), offset, length as u64) { continue; }                 let mut bytes = vec![0u8; length];                 if let Err(error) = crate::partial_donor::read_at(&source, offset, &mut bytes) {                     eprintln!( |  |
| src-tauri/src/localswarm/mod.rs | , error.message); }     }     fn checkpoint(&self, id: &str, verified: &[bool], status: &str, speed: u64) -> Result<(), String> {         // Snapshot under the lock, then encode and fsync outside it: a         // checkpoint after every durable window must not stall chunk progress,         // status or listing behind a multi-megabyte rewrite.         let (sequence, jobs) = {             let mut state = self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner());             let saved = state.jobs.get_mut(id).ok_or( | snapshot |
| src-tauri/src/localswarm/mod.rs | , error.message, staging.display()));             }         }         self.spawn(&id, pin, cancel, identity, acknowledgement)?; Ok(job)     }     fn spawn(&self, id: &str, pin: Option<String>, cancel: CancellationToken, space_identity: String, acknowledgement: Option<String>) -> Result<(), String> {         let manager = self.clone(); let task_id = id.to_string(); let token = cancel.clone();         // Register the running handle before the receiver can open any staging         // file. Otherwise a pause that wins the scheduling race finds no handle,         // returns without waiting, and leaves the receiver holding exclusive         // file locks that the pause contract promises are released.         let (start, started) = tokio::sync::oneshot::channel();         let task = tokio::spawn(async move {             let _ = started.await;             let result = manager.receive(&task_id, pin, token.clone(), space_identity, acknowledgement).await;             if let Err(error) = result {                 let mut state = manager.inner.state.lock().unwrap_or_else(\|e\| e.into_inner());                 if let Some(saved) = state.jobs.get_mut(&task_id) {                     if saved.job.status != | token |
| src-tauri/src/localswarm/mod.rs | , false)? };         let process_lock = match directory.dir.symlink_metadata( |  |
| src-tauri/src/localswarm/mod.rs | , false)?;         Ok(load(&directory, store)?.into_values().filter(\|saved\| !saved.history_dismissed).map(\|v\| v.job).collect())     }     pub fn new(store: Arc<Store>, admission: CancellationToken) -> Result<Self, String> {         check_cancel(&admission)?;         if store.private_mode()? \|\| store.read_only()? { return Err( | token |
| src-tauri/src/localswarm/mod.rs | , id.as_bytes())?; id };         let jobs = load(&directory, &store)?; let settings = store.settings()?;         let discovery = Discovery::new(peer_id.clone())?;         let lifetime = admission.child_token();         let sharing_lifetime = lifetime.child_token();         Ok(Self { inner: Arc::new(Inner { #[cfg(target_os = | token |
| src-tauri/src/localswarm/mod.rs | , next.to_string().as_bytes())             .map_err(\|error\| error.message)?;         Ok(next)     }     async fn share_library_inner(&self, name: String,         supplied: Option<(std::net::UdpSocket, Vec<String>, CancellationToken)>) -> Result<library::LibrarySharingStatus, String> {         self.allowed()?;         let _operation = self.inner.operation.lock().await;         check_cancel(&self.cancel_token())?;         self.stop_library_locked().await?;         let server = if let Some((socket, addresses, route_cancel)) = supplied {             check_cancel(&route_cancel)?;             let server = library::LibraryServer::start_on_socket(self.inner.store.clone(), self.inner.peer_id.clone(),                 name, self.inner.upload_budget.clone(), self.cancel_token(), socket, addresses).await?;             let stopped = server.descriptor.cancel.clone();             tokio::spawn(async move {                 tokio::select! { _ = stopped.cancelled() => {}, _ = route_cancel.cancelled() => { stopped.cancel(); } }             });             server         } else {             library::LibraryServer::start(self.inner.store.clone(), self.inner.peer_id.clone(),                 name, self.inner.upload_budget.clone(), self.cancel_token()).await?         };         let descriptor = server.descriptor.clone();         crate::nearby_log!( | token |
| src-tauri/src/localswarm/mod.rs | , true)?, Err(e) => return Err(err(e)), Ok(_) => profile.child( |  |
| src-tauri/src/localswarm/mod.rs | , true, false)? };         let peer_id = if let Some(bytes) = directory.optional_bytes( |  |
| src-tauri/src/localswarm/mod.rs | , true, true)?, Err(e) => return Err(err(e)), Ok(_) => directory.file( |  |
| src-tauri/src/localswarm/mod.rs | .into()) } fn validate_zim(directory: &Path, files: &[(String, File)], manifest: &Manifest, cancel: &CancellationToken) -> Result<(), String> {     check_cancel(cancel)?;     let expected = files.iter().map(\|(_, file)\| Stamp::of(&Metadata::from_file(file).map_err(err)?)).collect::<Result<Vec<_>, _>>()?;     let header = zim_header(files)?;     if &header[..4] != b | token |
| src-tauri/src/localswarm/mod.rs | .into()) } else { Ok(()) } }     /// Clone the job map for a durable write. Callers hold the state lock for     /// this snapshot only: the encode of every job's manifest hashes and the     /// fsync must never run under it, or a chunk-level checkpoint would stall     /// progress, status and listing for a multi-megabyte rewrite.     fn stage(&self, state: &State) -> (u64, BTreeMap<String, StoredJob>) {         let mut order = self.inner.persist_order.lock().unwrap_or_else(\|error\| error.into_inner());         order.next = order.next.saturating_add(1);         (order.next, state.jobs.clone())     }     /// Serialize and durably write a staged job map. `sequence` keeps the newest     /// mutation authoritative: a snapshot that has already been superseded on     /// disk is dropped rather than rolled back.     fn write_staged(&self, sequence: u64, jobs: &BTreeMap<String, StoredJob>) -> Result<(), storage::PersistError> {         let mut order = self.inner.persist_order.lock().unwrap_or_else(\|error\| error.into_inner());         if sequence <= order.written { return Ok(()); }         let bytes = bounded_json(jobs, STATE_LIMIT as usize)             .map_err(\|message\| storage::PersistError { message, committed: false })?;         let result = self.inner.directory.atomic( | snapshot |
| src-tauri/src/localswarm/mod.rs | .into())) } /// Check durable claims only. Unclaimed staging bytes may be unwritten or /// from a lost checkpoint window; the receiver must fetch them again. A set /// already renamed by our durable publishing intent may only be revalidated /// in full, never repaired. fn rehash_verified(     open: &mut OpenSet, manifest: &Manifest, owned: &OwnedSet,     bits: &mut [bool], cancel: &CancellationToken, ) -> Result<(), String> {     open.unchanged(manifest, owned)?;     for (index, bit) in bits.iter_mut().enumerate() {         check_cancel(cancel)?;         if *bit \|\| open.published {             let bytes = open.read_chunk(manifest, index as u32)?;             *bit = hex::encode(Sha256::digest(&bytes)) == manifest.chunk_hashes[index];         }     }     if open.published && bits.iter().any(\|bit\| !bit) {         return Err( | token |
| src-tauri/src/localswarm/mod.rs | .into()),             Ok(count) => count,             Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,             Err(error) => return Err(err(error)),         };         offset = offset.checked_add(count as u64).ok_or( |  |
| src-tauri/src/localswarm/mod.rs | .into()),             permit = self.inner.slots.acquire() => permit.map_err(err)?,         };         // Meaningful admitted work includes prefix rehash and final validation,         // but a job merely waiting for the bounded worker slot cannot hold wake.         #[cfg(target_os = |  |
| src-tauri/src/localswarm/mod.rs | .into());                 }                 if retained.identity()? != identity { return Err( |  |
| src-tauri/src/localswarm/mod.rs | .into());                 }             }         }         Ok(())     }     // Read the exact offered source concurrently using positional reads so all     // pipelined chunk readers run in parallel. The shared transport hashes     // these copied bytes once before responding, including for Apple QUIC.     fn chunk(&self, index: u32) -> Result<Vec<u8>, String> {         let measured = profile::start();         self.unchanged()?;         let (file, offset) = storage::chunk_position(&self.manifest, index)?;         let mut bytes = vec![0u8; self.manifest.chunk_len(index)?];         {             let retained = self.retained.read().map_err(\|_\| |  |
| src-tauri/src/localswarm/mod.rs | .into());         }         if let Err(error) = self.inner.discovery.lock().await.unadvertise(&offer.advertisement).await {             *self.inner.library_retirement.lock().unwrap_or_else(\|e\| e.into_inner()) = Some(offer.advertisement);             offer.server.shutdown().await;             self.note_library_stopped();             return Err(error);         }         if let Err(error) = offer.server.rebind(socket, addresses).await {             offer.server.shutdown().await; self.note_library_stopped(); return Err(error);         }         let descriptor = offer.server.descriptor.clone();         let generation = self.next_library_generation()?;         let advertisement = match self.inner.discovery.lock().await.advertise_library(&descriptor.session_id, descriptor.port,             offer.server.status().library_count, &descriptor.addresses, &descriptor.name, generation, offer.server.native_listener()).await {             Ok(id) => id,             Err(error) => { offer.server.shutdown().await; self.note_library_stopped(); return Err(error); }         };         let stopped = descriptor.cancel.clone();         tokio::spawn(async move {             tokio::select! { _ = stopped.cancelled() => {}, _ = route_cancel.cancelled() => stopped.cancel() }         });         let generation = offer.server.generation.clone();         *self.inner.library.lock().await = Some(LibraryOffer { server: offer.server, advertisement });         self.supervise_library(descriptor.clone(), generation);         Ok(descriptor)     }     async fn stop_library_locked(&self) -> Result<(), String> {         let offer = self.inner.library.lock().await.take();         if let Some(offer) = offer {             offer.server.cancel();             offer.server.shutdown().await;             *self.inner.library_retirement.lock().unwrap_or_else(\|e\| e.into_inner()) = Some(offer.advertisement);         }         let pending = self.inner.library_retirement.lock().unwrap_or_else(\|e\| e.into_inner()).clone();         if let Some(id) = pending {             self.inner.discovery.lock().await.unadvertise(&id).await?;             *self.inner.library_retirement.lock().unwrap_or_else(\|e\| e.into_inner()) = None;         }         Ok(())     }     pub async fn stop_library(&self) -> Result<library::LibrarySharingStatus, String> {         let _operation = self.inner.operation.lock().await;         // The person's own Stop needs no explanation. On a phone the Bluetooth         // publisher is stopped first, which cancels the offer and lets the         // supervisor record a reason before this runs.         let stopped = self.stop_library_locked().await;         *self.inner.library_stopped.lock().unwrap_or_else(\|e\| e.into_inner()) = None;         stopped?;         Ok(self.library_status().await)     }     pub async fn library_status(&self) -> library::LibrarySharingStatus {         let stopped = \|\| library::LibrarySharingStatus {             error: self.inner.library_stopped.lock().unwrap_or_else(\|e\| e.into_inner()).clone(),             ..Default::default()         };         self.inner.library.lock().await.as_ref().map_or_else(stopped, \|offer\| offer.server.status())     }     pub async fn library_descriptor(&self) -> Option<library::LibraryPeerDescriptor> {         self.inner.library.lock().await.as_ref().filter(\|offer\| offer.server.status().running)             .map(\|offer\| offer.server.descriptor.clone())     }     /// Resolve a current browse result, never an arbitrary caller-supplied host.     pub async fn library_peer(&self, id: &str) -> Result<discovery::Peer, String> {         self.allowed()?;         check_cancel(&self.cancel_token())?;         let discovery = self.inner.discovery.lock().await;         discovery.library_peer(id)     }     #[cfg(all(test, not(any(target_os = | record, generation, token |
| src-tauri/src/localswarm/mod.rs | .into());         }         let mut chunk_size = 1024u32 * 1024;         while stamps.iter().map(\|s\| s.size.div_ceil(u64::from(chunk_size))).sum::<u64>() > 262_144 {             chunk_size = chunk_size.checked_mul(2).filter(\|n\| *n <= 16 * 1024 * 1024)                 .ok_or( |  |
| src-tauri/src/localswarm/mod.rs | .into());         }         let pin = pin.filter(\|pin\| !pin.is_empty());         self.pause_locked(id, false).await?;         let saved = self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()).jobs.get(id).cloned().ok_or( |  |
| src-tauri/src/localswarm/mod.rs | .into());         }         let request = serde_json::to_string(&(sid.as_str(), format!( |  |
| src-tauri/src/localswarm/mod.rs | .into());         }         let source_reader = source.clone();         let reason = Arc::new(Mutex::new(None));         let monitor = SourceMonitor::start(&source, cancel.clone(), reason.clone())?;         let reader_cancel = cancel.clone(); let reader_reason = reason.clone();         let reader: network::ChunkReader = Arc::new(move \|index\| source_reader.chunk(index));         let retire: network::SeedSourceFailure = Arc::new(move \|error\| {             *reader_reason.lock().unwrap_or_else(\|e\| e.into_inner()) = Some(error.to_owned());             reader_cancel.cancel();         });         let server = network::seed(manifest.clone(), self.inner.peer_id.clone(), pin.clone(), reader,             self.inner.upload_budget.clone(), cancel.clone(), Some(retire)).await?;         if let Err(error) = source.unchanged().and_then(\|_\| check_cancel(&cancel)) { server.shutdown().await; return Err(error); }         #[cfg(target_os = |  |
| src-tauri/src/localswarm/mod.rs | .into());         }     }     Ok(()) } struct SourceSet {     store: Arc<Store>,     book: Book,     retained: std::sync::RwLock<crate::archive_source::RetainedSource>,     #[cfg(mobile)]     stamps: Vec<Stamp>,     manifest: Arc<Manifest>, } impl SourceSet {     async fn prepare(store: Arc<Store>, book: Book, cancel: CancellationToken) -> Result<(Arc<Manifest>, Arc<Self>), String> {         check_cancel(&cancel)?;         let source_book = book.clone();         if book.source_reference.is_some() {             let verify_cancel = cancel.clone();             let (retained, identity) = readonly_verification(&cancel, move \|\| {                 let retained = crate::archive_source::retain(&source_book)?;                 let identity = retained.identity()?;                 let archive = retained.open()?;                 if archive.metadata_cancellable(&verify_cancel)?.id != source_book.metadata.id                     \|\| !archive.check_integrity_cancellable(&verify_cancel, \|_, _\| {})?                 {                     return Err( | token |
| src-tauri/src/localswarm/mod.rs | .into());     }     check_cancel(cancel)?;     for ((_, file), expected) in files.iter().zip(&expected) {         if Stamp::of(&Metadata::from_file(file).map_err(err)?)? != *expected {             return Err( |  |
| src-tauri/src/localswarm/mod.rs | .into());     }     let archive = Archive::open_files(directory, files)?;     let metadata = archive.metadata_cancellable(cancel)?;     if metadata.id.is_empty() \|\| !archive.check_integrity_cancellable(cancel, \|_, _\| {})? {         return Err( |  |
| src-tauri/src/localswarm/mod.rs | .into());     }     open.unchanged(manifest, owned) } /// Copies chunks that are still missing from same-sized partial files owned by /// other acquisitions (e.g. an unfinished BitTorrent download of the same /// archive). A chunk is accepted only when its SHA-256 equals the manifest /// hash, exactly as for a network chunk; donor provenance grants nothing. /// Unwritten parts of a partial download read as zeros, so an all-zero chunk is /// compared with a cached zero-chunk hash rather than hashed, and is never /// counted as a content mismatch. A donor whose first `MISMATCH_LIMIT` /// fully written (no all-zero 16 KiB block) candidate chunks all fail is /// abandoned as different content. /// Donor I/O failures skip that donor; staging write failures are returned. /// Cancellation stops early and keeps what was adopted. Returns adopted bytes. fn adopt_partial_chunks(     open: &mut OpenSet, manifest: &Manifest, bits: &mut [bool],     donors: &[crate::partial_donor::Donor], cancel: &CancellationToken, ) -> Result<u64, String> {     let mut adopted = 0u64;     let chunk = u64::from(manifest.chunk_size_bytes);     let mut zero_hashes: HashMap<usize, String> = HashMap::new();     for entry in &manifest.files {         let (start, end) = (entry.start_chunk_index, entry.end_chunk_index);         for donor in donors.iter().filter(\|donor\| donor.size == entry.size_bytes && entry.size_bytes > 0) {             if !(start..=end).any(\|index\| !bits[index as usize]) { break; }             let source = match crate::partial_donor::open(&donor.path, donor.size) {                 Ok(source) => source,                 Err(error) => { eprintln!( | token |
| src-tauri/src/localswarm/mod.rs | .into()); }                 Ok((retained, identity))             }).await?;             blocking(move \|\| Self::from_retained(store, book, cancel, retained, identity)).await         } else {             let pinned = readonly_verification(&cancel, move \|\| crate::library_files::pin_archive_files(&source_book)).await?;             blocking(move \|\| Self::from_pinned(store, book, cancel, pinned)).await         }     }     fn from_pinned(store: Arc<Store>, book: Book, cancel: CancellationToken,         pinned: crate::library_files::PinnedArchive) -> Result<(Arc<Manifest>, Arc<Self>), String> {         let retained = crate::archive_source::RetainedSource::Filesystem(pinned);         retained.unchanged()?;         let identity = retained.identity()?;         Self::from_retained(store, book, cancel, retained, identity)     }     fn from_retained(store: Arc<Store>, book: Book, cancel: CancellationToken,         mut retained: crate::archive_source::RetainedSource,         identity: crate::integrity::ContentIdentity) -> Result<(Arc<Manifest>, Arc<Self>), String> {         check_cancel(&cancel)?;         retained.unchanged()?;         if retained.identity()? != identity { return Err( | token |
| src-tauri/src/localswarm/mod.rs | .into()); }             Ok((open.directory.path.clone(), open.verification_files(&validate_manifest, &validate_owned)?))         }).await?;         let validate_manifest = manifest.clone();         let token = cancel.clone();         readonly_verification(&cancel, move \|\| validate_zim(&validate_path, &validate_files, &validate_manifest, &token)).await?;         check_cancel(&cancel)?;         { let mut state = self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()); state.jobs.get_mut(id).ok_or( | token |
| src-tauri/src/localswarm/mod.rs | .into()); }             for (entry, identity) in job.manifest.files.iter().zip(&job.owned.files) {                 candidates.push((job.owned.final_path(&entry.path), crate::library_files::BackupEvidence {                     size: entry.size_bytes, identity: Some(identity.backup_identity()), sha256: None,                 }));             }         }     }     Ok(candidates) } impl LocalSwarmManager {     pub(crate) fn claims_monitored_path(&self, path: &Path) -> Result<bool, String> {         let state = self.inner.state.lock().map_err(\|_\| |  |
| src-tauri/src/localswarm/mod.rs | .into()); }             total = total.checked_add(stamp.size).ok_or( |  |
| src-tauri/src/localswarm/mod.rs | .into()); }         #[cfg(feature = |  |
| src-tauri/src/localswarm/mod.rs | .into()); }         // receive() owns the one retained-prefix rehash under its admitted-work         // assertion. Do not rehash here while the command's transfer gate hides         // that work from the native idle-timer reconciler.         let cancel = self.cancel_token();         { let mut state = self.inner.state.lock().unwrap_or_else(\|e\| e.into_inner()); let saved = state.jobs.get_mut(id).ok_or( | token |
| src-tauri/src/localswarm/mod.rs | .into()); }         let book = self.inner.store.book(book_id)?.filter(\|b\| b.available).ok_or( |  |
| src-tauri/src/localswarm/mod.rs | .into()); }         let mut claims = Vec::new();         for (id, saved) in jobs {             validate_stored(&id, &saved)?;             if saved.cleanup.is_some() {                 let directory = saved.owned.parent.join(cleanup::cleanup_name(&saved)?);                 claims.extend(saved.manifest.files.iter().map(\|entry\| directory.join(&entry.path)));             }             claims.push(PathBuf::from(saved.job.destination));             claims.extend(saved.job.completed_paths.into_iter().map(PathBuf::from));             for entry in &saved.manifest.files {                 claims.push(saved.owned.final_path(&entry.path));             }         }         crate::acquisition::MonitoredPathClaims::new(claims)     }      /// Staged files of unfinished receives with their durably verified byte     /// ranges, for another transport to reuse after its own verification.     /// Reads saved history only; it never opens discovery, sockets or payload.     pub(crate) fn saved_partial_files(store: &Store) -> Result<Vec<crate::partial_donor::Donor>, String> {         let jobs: BTreeMap<String, StoredJob> = crate::acquisition::saved_monitored_record(             store, &[ | record |
| src-tauri/src/localswarm/mod.rs | .into()); }         let mut donors = Vec::new();         for (id, saved) in jobs {             validate_stored(&id, &saved)?;             if saved.publishing \|\| saved.cleanup.is_some() \|\| saved.job.status == |  |
| src-tauri/src/localswarm/mod.rs | .into()); }         let mut manifest = Manifest { zim: None, protocol_version: 1, swarm_id: String::new(),             name: book.metadata.title.clone(), total_bytes: total, chunk_size_bytes: chunk_size,             chunk_hash_algo: |  |
| src-tauri/src/localswarm/mod.rs | .into()); }         let mut stamps = Vec::with_capacity(retained.files().len());         let mut total = 0u64;         for (_, file) in retained.files() {             let stamp = Stamp::read_only_source(&Metadata::from_file(file).map_err(err)?)?;             if stamp.size == 0 { return Err( |  |
| src-tauri/src/localswarm/mod.rs | .into()); }         let port = target.port();         let direct = target.direct();         let (cancel, host, initial_route, manifest) = {             let _setup = self.inner.receive_setup_gate.read().await;             let cancel = self.cancel_token();             check_cancel(&cancel)?;             // Perform the network dial and optional identity preflight outside             // `operation` so a slow/unreachable peer cannot freeze unrelated             // LocalSwarm controls (pause, resume, stop, discovery, cleanup),             // while `receive_setup_gate` still lets `stop()` fence in-flight dials.             let (host, mut session) = self.session(&target, &sid, pin.as_deref(), &cancel).await?;             supported(session.manifest())?;             // No incoming UUID can collide with an empty library. Avoid reading a             // payload chunk (and waiting on its bandwidth budget) just to prove that.             if self.inner.store.has_books()? {                 if let Some(id) = incoming_archive_id(&mut session).await? {                     if let Some(existing) = self.inner.store.book(&id)? {                         session.shutdown().await;                         return Err(if existing.available { | uuid, token |
| src-tauri/src/localswarm/mod.rs | .into()); }     for (id, saved) in &mut jobs { validate_stored(id, saved)?; if matches!(saved.job.status.as_str(), |  |
| src-tauri/src/localswarm/mod.rs | .into()); } id } else { let id = Uuid::new_v4().to_string(); directory.atomic( | uuid |
| src-tauri/src/localswarm/mod.rs | .into(), files: entries, chunk_hashes: hashes };         manifest.swarm_id = manifest.content_address();         supported(&manifest)?;         let manifest = Arc::new(manifest);         let source = Arc::new(Self { store, book, retained: std::sync::RwLock::new(retained),             #[cfg(mobile)]             stamps,             manifest: manifest.clone() });         check_cancel(&cancel)?;         source.unchanged()?;         Ok((manifest, source))     }     fn principal(&self) -> Result<(), String> {         // The store compares in place under its own lock: serving a chunk must         // not clone a Book (or the whole state) per identity check.         if self.store.owner_matches(&self.book)? { return Ok(()); }         // Only the failure path pays for distinguishing the two verdicts.         let installed = self.store.book(&self.book.metadata.id)?.filter(\|b\| b.available).is_some();         Err(if installed { |  |
| src-tauri/src/localswarm/mod.rs | .into();                     retained.job.error = Some(error.message.clone());                 } else {                     state.jobs.remove(&id);                 }                 return Err(format!( |  |
| src-tauri/src/localswarm/mod.rs | .into(); saved.job.downloaded = manifest.total_bytes; saved.job.speed = 0; saved.job.eta_seconds = None; saved.job.error = None; saved.job.completed_paths = vec![saved.job.destination.clone()]; saved.job.updated_at = now(); self.persist(&state).map_err(Into::into)     }     /// In-flight progress belongs to this worker, never to the persisted job     /// map. Route changes and writes by other jobs may snapshot the map before     /// this worker's next payload flush.     fn advance(&self, id: &str, downloaded: u64, speed: u64) {         let mut state = self.inner.state.lock().unwrap_or_else(\|error\| error.into_inner());         if let Some(worker) = state.running.get_mut(id) {             worker.progress = Some(ReceiveProgress { downloaded, speed, updated_at: now() });         }     }     /// Persist the route that actually carried payload. Sessions are bounded     /// (four source slots, bounded reconnects), and this writes only when the     /// route changes, so the receipt cannot become a write amplifier.     fn record_route(&self, id: &str, route: &str) {         if route.is_empty() \|\| route.len() > 240 \|\| route.chars().any(char::is_control) { return; }         let (sequence, jobs) = {             let mut state = self.inner.state.lock().unwrap_or_else(\|error\| error.into_inner());             let Some(saved) = state.jobs.get_mut(id) else { return };             if saved.job.route.as_deref() == Some(route) { return; }             saved.job.route = Some(route.to_string());             self.stage(&state)         };         if let Err(error) = self.write_staged(sequence, &jobs) { eprintln!( | record, snapshot |
| src-tauri/src/localswarm/mod.rs | .into(); saved.job.error = None; self.persist(&state)?; }         self.spawn(id, pin, cancel, id.to_string(), acknowledgement)     }     async fn candidates(&self, host: String, port: u16, sid: &str, direct: bool) -> VecDeque<ConnectionTarget> {         let peers = self.inner.discovery.lock().await.peers();         source_candidates(host, port, sid, peers, direct)     }     async fn refresh_candidates(&self, dialer: &mut SourceDialer) -> bool {         dialer.add_discovered(self.inner.discovery.lock().await.peers())     }     async fn receive(&self, id: &str, pin: Option<String>, cancel: CancellationToken, space_identity: String, acknowledgement: Option<String>) -> Result<(), String> {         let _slot = tokio::select! {             biased;             _ = cancel.cancelled() => return Err( | token |
| src-tauri/src/localswarm/mod.rs | .into(); saved.job.speed = 0; saved.job.eta_seconds = None; } }     Ok(jobs) }  /// Apply the schema-directed profile relocation to the only filesystem paths a /// saved LocalSwarm record owns. `profile_paths::rewrite` rewrites those three /// fields per string and consults no other field, so it runs on a minimal value /// carrying exactly them; that keeps the rewrite byte-for-byte equivalent to /// rewriting the whole document while never materializing or re-parsing one. /// Keep this field list in sync with the `LocalSwarm` arm of that schema. fn rebase_saved_paths(store: &Store, jobs: &mut BTreeMap<String, StoredJob>) -> Result<(), String> {     let mut probe = serde_json::Map::with_capacity(jobs.len());     for (id, saved) in jobs.iter() {         probe.insert(id.clone(), json!({ | record, schema |
| src-tauri/src/localswarm/mod.rs | .kiwix-localswarm-{suffix}.part |  |
| src-tauri/src/localswarm/mod.rs | Incomplete LocalSwarm job contains import paths |  |
| src-tauri/src/localswarm/mod.rs | Invalid LocalSwarm completion receipt |  |
| src-tauri/src/localswarm/mod.rs | Invalid LocalSwarm job ID |  |
| src-tauri/src/localswarm/mod.rs | Invalid LocalSwarm job identity |  |
| src-tauri/src/localswarm/mod.rs | Invalid persisted LocalSwarm destination or manifest |  |
| src-tauri/src/localswarm/mod.rs | LocalSwarm history exceeds the state limit; remove finished history |  |
| src-tauri/src/localswarm/mod.rs | LocalSwarm operation stopped |  |
| src-tauri/src/localswarm/mod.rs | LocalSwarm retained ownership does not cover the manifest |  |
| src-tauri/src/localswarm/mod.rs | LocalSwarm verification stopped: {e} |  |
| src-tauri/src/localswarm/mod.rs | Nearby sharing stopped. Start it again |  |
| src-tauri/src/localswarm/mod.rs | Offered archive changed; prepare a new offer |  |
| src-tauri/src/localswarm/mod.rs | Offered archive directory changed |  |
| src-tauri/src/localswarm/mod.rs | Only one .zim or a complete .zimaa split-ZIM set is supported |  |
| src-tauri/src/localswarm/mod.rs | Select one nonempty ZIM archive or its complete contiguous split set |  |
| src-tauri/src/localswarm/mod.rs | Split-ZIM parts must be one exact ordered contiguous set beginning with .zimaa |  |
| src-tauri/src/localswarm/mod.rs | This nearby download cannot be cleaned up yet. Wait for it to finish |  |
| src-tauri/src/localswarm/mod.rs | queued |  |
| src-tauri/src/localswarm/mod.rs | {base}{}{} |  |
| src-tauri/src/localswarm/network.rs | Apple QUIC connection failed or left the local link |  |
| src-tauri/src/localswarm/network.rs | Apple QUIC send callback lost |  |
| src-tauri/src/localswarm/network.rs | Bad chunk index |  |
| src-tauri/src/localswarm/network.rs | Bad chunk length |  |
| src-tauri/src/localswarm/network.rs | Cross-swarm LocalSwarm frame |  |
| src-tauri/src/localswarm/network.rs | Inconsistent LocalSwarm bitfield count |  |
| src-tauri/src/localswarm/network.rs | Invalid LocalSwarm certificate chain |  |
| src-tauri/src/localswarm/network.rs | Invalid LocalSwarm chunk pipeline window |  |
| src-tauri/src/localswarm/network.rs | Invalid LocalSwarm connection details |  |
| src-tauri/src/localswarm/network.rs | Invalid LocalSwarm frame length |  |
| src-tauri/src/localswarm/network.rs | Invalid LocalSwarm seed credentials |  |
| src-tauri/src/localswarm/network.rs | Invalid manifest frame length |  |
| src-tauri/src/localswarm/network.rs | Local network policy interrupted LocalSwarm |  |
| src-tauri/src/localswarm/network.rs | Local network policy interrupted address lookup |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm QUIC connection failed: {e} |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm address lookup failed: {e} |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm authentication failed |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm bitfield count does not match manifest |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm chunk checksum mismatch |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm client reader stopped |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm connect timed out |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm control message flood |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm control message flood before manifest |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm duplicate-control flood |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm exact certificate pin mismatch |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm frame exceeds type limit |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm host has no usable address |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm is not accepting new receive sessions |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm is not accepting new transfers |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm listen failed: {e} |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm manifest swarm mismatch |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm next-request progress deadline exceeded |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm payload progress deadline exceeded |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm peer did not handshake for the requested swarm |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm peer did not prove the shared PIN |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm peer made no progress |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm peer stopped before the requested chunk |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm prefetch budget closed |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm read failed: {e} |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm route binding failed: {error} |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm route selection failed: {error} |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm socket failed: {error} |  |
| src-tauri/src/localswarm/network.rs | LocalSwarm source changed or returned corrupt bytes |  |
| src-tauri/src/localswarm/network.rs | Malformed chunk response |  |
| src-tauri/src/localswarm/network.rs | Manifest JSON too large |  |
| src-tauri/src/localswarm/network.rs | Missing LocalSwarm handshake |  |
| src-tauri/src/localswarm/network.rs | Mobile network policy interrupted LocalSwarm |  |
| src-tauri/src/localswarm/network.rs | Mobile network policy interrupted address lookup |  |
| src-tauri/src/localswarm/network.rs | No LocalSwarm send in flight |  |
| src-tauri/src/localswarm/network.rs | Out-of-range LocalSwarm chunk request |  |
| src-tauri/src/localswarm/network.rs | Out-of-range LocalSwarm have |  |
| src-tauri/src/localswarm/network.rs | Serving a LocalSwarm transfer |  |
| src-tauri/src/localswarm/network.rs | The selected LocalSwarm network changed while binding |  |
| src-tauri/src/localswarm/network.rs | Truncated LocalSwarm payload |  |
| src-tauri/src/localswarm/network.rs | Truncated LocalSwarm stream |  |
| src-tauri/src/localswarm/network.rs | Unexpected LocalSwarm client frame |  |
| src-tauri/src/localswarm/network.rs | Unexpected LocalSwarm client message or authentication required |  |
| src-tauri/src/localswarm/network.rs | Unexpected LocalSwarm frame for this session phase |  |
| src-tauri/src/localswarm/network.rs | Unexpected LocalSwarm message before manifest |  |
| src-tauri/src/localswarm/network.rs | Unexpected LocalSwarm message before the PIN proof |  |
| src-tauri/src/localswarm/network.rs | Unexpected LocalSwarm message while fetching chunk |  |
| src-tauri/src/localswarm/network.rs | Unsolicited chunk response |  |
| src-tauri/src/localswarm/network.rs | Unsolicited or inconsistent chunk response |  |
| src-tauri/src/localswarm/network.rs | [nearby] try {address} |  |
| src-tauri/src/localswarm/network.rs | [nearby] {address} bind failed: {error} |  |
| src-tauri/src/localswarm/network.rs | [nearby] {address} failed: {error} |  |
| src-tauri/src/localswarm/network.rs | [nearby] {address} route failed: {error} |  |
| src-tauri/src/localswarm/network.rs | bonjour-lan: |  |
| src-tauri/src/localswarm/network.rs | bonjour: |  |
| src-tauri/src/localswarm/network.rs | client |  |
| src-tauri/src/localswarm/network.rs | local policy interrupted |  |
| src-tauri/src/localswarm/network.rs | localswarm.local |  |
| src-tauri/src/localswarm/network.rs | policy interrupted |  |
| src-tauri/src/localswarm/network.rs | protocol error |  |
| src-tauri/src/localswarm/network.rs | server |  |
| src-tauri/src/localswarm/protocol.rs | .. |  |
| src-tauri/src/localswarm/protocol.rs | Bitfield too large |  |
| src-tauri/src/localswarm/protocol.rs | Chunk has no owning file |  |
| src-tauri/src/localswarm/protocol.rs | Chunk index out of range |  |
| src-tauri/src/localswarm/protocol.rs | Colliding manifest paths |  |
| src-tauri/src/localswarm/protocol.rs | Empty frame |  |
| src-tauri/src/localswarm/protocol.rs | Frame exceeds type limit |  |
| src-tauri/src/localswarm/protocol.rs | Frame field exceeds limit |  |
| src-tauri/src/localswarm/protocol.rs | Frame length overflow |  |
| src-tauri/src/localswarm/protocol.rs | Frame too large |  |
| src-tauri/src/localswarm/protocol.rs | Inconsistent bitfield length |  |
| src-tauri/src/localswarm/protocol.rs | Inconsistent manifest chunk ranges |  |
| src-tauri/src/localswarm/protocol.rs | Invalid chunk offset |  |
| src-tauri/src/localswarm/protocol.rs | Invalid manifest JSON |  |
| src-tauri/src/localswarm/protocol.rs | Manifest ZIM identity is not a canonical UUID | uuid |
| src-tauri/src/localswarm/protocol.rs | Manifest chunk count overflow |  |
| src-tauri/src/localswarm/protocol.rs | Manifest chunk range overflow |  |
| src-tauri/src/localswarm/protocol.rs | Manifest content address, totals or hashes are inconsistent |  |
| src-tauri/src/localswarm/protocol.rs | Manifest exceeds supported bounds |  |
| src-tauri/src/localswarm/protocol.rs | Manifest file/directory collision |  |
| src-tauri/src/localswarm/protocol.rs | Manifest size overflow |  |
| src-tauri/src/localswarm/protocol.rs | Nonzero bitfield padding |  |
| src-tauri/src/localswarm/protocol.rs | Outbound frame exceeds limit |  |
| src-tauri/src/localswarm/protocol.rs | Reserved manifest path |  |
| src-tauri/src/localswarm/protocol.rs | Too many manifest chunks |  |
| src-tauri/src/localswarm/protocol.rs | Trailing frame data |  |
| src-tauri/src/localswarm/protocol.rs | Truncated frame |  |
| src-tauri/src/localswarm/protocol.rs | Unknown LocalSwarm message type |  |
| src-tauri/src/localswarm/protocol.rs | Unknown frame type |  |
| src-tauri/src/localswarm/protocol.rs | Unsafe manifest path |  |
| src-tauri/src/localswarm/protocol.rs | Unsafe manifest path component |  |
| src-tauri/src/localswarm/protocol.rs | Unsafe normalized manifest path |  |
| src-tauri/src/localswarm/protocol.rs | Unsupported LocalSwarm manifest version or hash algorithm |  |
| src-tauri/src/localswarm/protocol.rs | aux |  |
| src-tauri/src/localswarm/protocol.rs | clock$ |  |
| src-tauri/src/localswarm/protocol.rs | con |  |
| src-tauri/src/localswarm/protocol.rs | conin$ |  |
| src-tauri/src/localswarm/protocol.rs | conout$ |  |
| src-tauri/src/localswarm/protocol.rs | manifest array exceeds limit |  |
| src-tauri/src/localswarm/protocol.rs | nul |  |
| src-tauri/src/localswarm/protocol.rs | prn |  |
| src-tauri/src/localswarm/sources.rs | Invalid LocalSwarm discovery selection |  |
| src-tauri/src/localswarm/sources.rs | Invalid discovered LocalSwarm address count |  |
| src-tauri/src/localswarm/sources.rs | LocalSwarm address attempt stopped |  |
| src-tauri/src/localswarm/sources.rs | LocalSwarm advertised address connection timed out |  |
| src-tauri/src/localswarm/sources.rs | LocalSwarm connection has no address |  |
| src-tauri/src/localswarm/sources.rs | LocalSwarm connection stopped |  |
| src-tauri/src/localswarm/sources.rs | LocalSwarm receive admission closed during connection |  |
| src-tauri/src/localswarm/sources.rs | The discovered LocalSwarm peer has no usable advertised address |  |
| src-tauri/src/localswarm/sources.rs | The selected LocalSwarm peer is no longer discovered; choose it again |  |
| src-tauri/src/localswarm/sources.rs | The selected LocalSwarm share changed; inspect it again |  |
| src-tauri/src/localswarm/sources.rs | Unsupported LocalSwarm discovered peer |  |
| src-tauri/src/localswarm/sources.rs | quic |  |
| src-tauri/src/localswarm/storage.rs | .state-{} |  |
| src-tauri/src/localswarm/storage.rs | COM |  |
| src-tauri/src/localswarm/storage.rs | Invalid LocalSwarm chunk write |  |
| src-tauri/src/localswarm/storage.rs | LPT |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm chunk index is outside the manifest |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm cleanup captured a replaced directory; all entries were preserved |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm cleanup source directory was replaced |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm destination parent was replaced |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm directory identity changed |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm file changed before writer retirement |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm file identity changed during publication |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm file identity changed during writer retirement |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm file ownership or size changed |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm filename is reserved |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm owned directories must be private (0700) |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm requires regular files without hard links |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm requires safe single-component filenames |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm set is already published |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm sources must be regular files |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm staged bytes changed after chunk verification |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm staged file changed |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm staged handles are incomplete |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm staged set was not sealed before publication |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm staging capability was released for publication; reopen the staged set |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm staging ownership changed; retained data was not touched |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm state must not be a symlink |  |
| src-tauri/src/localswarm/storage.rs | LocalSwarm verification handle no longer identifies the staged bytes |  |
| src-tauri/src/localswarm/storage.rs | Not enough free space for this LocalSwarm archive |  |
| src-tauri/src/localswarm/storage.rs | Seal LocalSwarm writers before native verification |  |
| src-tauri/src/localswarm/storage.rs | Seal LocalSwarm writers before publication |  |
| src-tauri/src/localswarm/storage.rs | Unexpected LocalSwarm staging entries |  |
| src-tauri/src/localswarm/storage.rs | Unexpected entries in LocalSwarm staging; retained data was not touched |  |
| src-tauri/src/localswarm/storage.rs | linux |  |
| src-tauri/src/localswarm/storage.rs | localswarm-{}-{suffix} |  |
| src-tauri/src/location_android.rs | Android did not answer the location request |  |
| src-tauri/src/location_android.rs | Android location is not ready yet |  |
| src-tauri/src/location_android.rs | Android returned an invalid position |  |
| src-tauri/src/location_apple.rs | Native location provider returned an invalid position |  |
| src-tauri/src/location_linux.rs | GeoClue stopped before reporting a location |  |
| src-tauri/src/location_windows.rs | Location provider could not start |  |
| src-tauri/src/location_windows.rs | Location state unavailable |  |
| src-tauri/src/location_windows.rs | Windows COM initialization failed: 0x{:08x} |  |
| src-tauri/src/location_windows.rs | Windows could not get a location in time |  |
| src-tauri/src/map_chunk_cache.rs | Cannot inspect map chunk cache entry: {error} |  |
| src-tauri/src/map_chunk_cache.rs | Cannot inspect map chunk cache entry: {e} |  |
| src-tauri/src/map_chunk_cache.rs | Cannot open map chunk cache entry: {error} |  |
| src-tauri/src/map_chunk_cache.rs | Cannot remove map chunk cache entry: {error} |  |
| src-tauri/src/map_chunk_cache.rs | Invalid map chunk cache name |  |
| src-tauri/src/map_chunk_cache.rs | Known oldest cache entry |  |
| src-tauri/src/map_chunk_cache.rs | Map chunk cache accounting mismatch |  |
| src-tauri/src/map_chunk_cache.rs | Map chunk cache directory changed identity |  |
| src-tauri/src/map_chunk_cache.rs | Map chunk cache name and payload size differ or exceed the entry limit |  |
| src-tauri/src/map_chunk_cache.rs | Map chunk cache name is occupied by a non-regular entry |  |
| src-tauri/src/map_chunk_cache.rs | Map chunk cache temporary changed identity |  |
| src-tauri/src/map_chunk_cache.rs | Map chunk exceeds cache capacity |  |
| src-tauri/src/map_chunk_cache.rs | {original}; cannot remove owned map cache temporary {name}: {error} |  |
| src-tauri/src/maps.rs | Catalog worker failed during retirement: {error} |  |
| src-tauri/src/maps.rs | Catalog worker failed: {error} |  |
| src-tauri/src/maps.rs | Evaluation recipe output size differs from bounded manifest |  |
| src-tauri/src/maps.rs | Evaluation source budget overflow |  |
| src-tauri/src/maps.rs | Evaluation source size overflow |  |
| src-tauri/src/maps.rs | Evaluation staged sources exceed bounded workload budget |  |
| src-tauri/src/maps.rs | Map installation changed |  |
| src-tauri/src/maps.rs | Map settings changed; select the map again before downloading or replacing it |  |
| src-tauri/src/maps.rs | Missing staged mask |  |
| src-tauri/src/maps.rs | Missing staged source {} |  |
| src-tauri/src/maps.rs | Recipe does not match the selected catalog version |  |
| src-tauri/src/maps.rs | Selected map language is not offered |  |
| src-tauri/src/maps.rs | Selected map region is not in the catalog |  |
| src-tauri/src/maps.rs | Staged source SHA-256 mismatch |  |
| src-tauri/src/maps.rs | Staged source size mismatch |  |
| src-tauri/src/maps.rs | The installed map changed; select the map again before replacing it |  |
| src-tauri/src/maps.rs | Verified map does not match the authorized recipe |  |
| src-tauri/src/maps/compose.rs | Accept |  |
| src-tauri/src/maps/compose.rs | Accept-Encoding |  |
| src-tauri/src/maps/compose.rs | Cached chunk length changed |  |
| src-tauri/src/maps/compose.rs | Cannot sync map job state directory: {e} |  |
| src-tauri/src/maps/compose.rs | Chunk exceeds source |  |
| src-tauri/src/maps/compose.rs | Chunk index exceeds source |  |
| src-tauri/src/maps/compose.rs | Chunk offset overflow |  |
| src-tauri/src/maps/compose.rs | Downloaded map chunk SHA-256 mismatch |  |
| src-tauri/src/maps/compose.rs | Insufficient disk space for map and downloaded parts |  |
| src-tauri/src/maps/compose.rs | Invalid StreetZim range |  |
| src-tauri/src/maps/compose.rs | LOW_DISK_SPACE:{warning} |  |
| src-tauri/src/maps/compose.rs | Map base candidate was replaced |  |
| src-tauri/src/maps/compose.rs | Map block count overflow |  |
| src-tauri/src/maps/compose.rs | Map cache requirement overflow |  |
| src-tauri/src/maps/compose.rs | Map chunk transfer failed |  |
| src-tauri/src/maps/compose.rs | Map disk requirement overflow |  |
| src-tauri/src/maps/compose.rs | Map job history exceeds its state limit; previous claims were retained |  |
| src-tauri/src/maps/compose.rs | Map recipe is too complex to count blocks safely |  |
| src-tauri/src/maps/compose.rs | Map state file has no parent |  |
| src-tauri/src/maps/compose.rs | Missing source URL |  |
| src-tauri/src/maps/compose.rs | Missing source length |  |
| src-tauri/src/maps/compose.rs | Missing source mask |  |
| src-tauri/src/maps/compose.rs | Missing source offset |  |
| src-tauri/src/maps/compose.rs | Missing staged map source |  |
| src-tauri/src/maps/compose.rs | Range |  |
| src-tauri/src/maps/compose.rs | Saved map block checkpoint does not match the exact recipe |  |
| src-tauri/src/maps/compose.rs | Staged map source range was truncated |  |
| src-tauri/src/maps/compose.rs | Staged map source size changed |  |
| src-tauri/src/maps/compose.rs | StreetZim JSON returned HTTP {} |  |
| src-tauri/src/maps/compose.rs | StreetZim range request failed: {e} |  |
| src-tauri/src/maps/compose.rs | StreetZim request failed: {e} |  |
| src-tauri/src/maps/compose.rs | StreetZim response exceeded size limit |  |
| src-tauri/src/maps/compose.rs | StreetZim response exceeds allowed length |  |
| src-tauri/src/maps/compose.rs | StreetZim source range was truncated |  |
| src-tauri/src/maps/compose.rs | StreetZim source returned an invalid HTTP range response |  |
| src-tauri/src/maps/compose.rs | Unexpected compressed StreetZim response |  |
| src-tauri/src/maps/compose.rs | Unsupported StreetZim JSON encoding |  |
| src-tauri/src/maps/compose.rs | Unsupported StreetZim JSON encoding: {other} |  |
| src-tauri/src/maps/compose.rs | Verified map block is not required by its recipe |  |
| src-tauri/src/maps/compose.rs | application/json |  |
| src-tauri/src/maps/compose.rs | base |  |
| src-tauri/src/maps/compose.rs | bytes={offset}-{end} |  |
| src-tauri/src/maps/compose.rs | gzip, identity |  |
| src-tauri/src/maps/compose.rs | {expected}-{len} |  |
| src-tauri/src/maps/manager.rs | A map is already being composed |  |
| src-tauri/src/maps/manager.rs | Cancel the active map before clearing downloaded parts |  |
| src-tauri/src/maps/manager.rs | Cannot publish verified map without overwriting: {e} |  |
| src-tauri/src/maps/manager.rs | Cannot save map job state: {error} |  |
| src-tauri/src/maps/manager.rs | Cannot sync composed map: {error} |  |
| src-tauri/src/maps/manager.rs | Composed file size mismatch |  |
| src-tauri/src/maps/manager.rs | Composed whole-file MD5 mismatch |  |
| src-tauri/src/maps/manager.rs | Delete the retained map files instead of only removing them from the list |  |
| src-tauri/src/maps/manager.rs | Explicit replacement is required only for the selected installed map |  |
| src-tauri/src/maps/manager.rs | Kiwix could not clean up a partial map download. Delete the download and start again |  |
| src-tauri/src/maps/manager.rs | Map checkpoint destination changed; no bytes were modified |  |
| src-tauri/src/maps/manager.rs | Map checkpoint is missing |  |
| src-tauri/src/maps/manager.rs | Map checkpoint lacks its verified base identity |  |
| src-tauri/src/maps/manager.rs | Map checkpoint ownership is missing |  |
| src-tauri/src/maps/manager.rs | Map choice expired after cancellation or a privacy transition; select the map again |  |
| src-tauri/src/maps/manager.rs | Map choice expired; select the map again |  |
| src-tauri/src/maps/manager.rs | Map destination must be an absolute directory |  |
| src-tauri/src/maps/manager.rs | Map download changed while preparing cleanup; no deletion was authorized |  |
| src-tauri/src/maps/manager.rs | Map job history exceeds its state limit; saved claims were not changed |  |
| src-tauri/src/maps/manager.rs | Map pause omitted its durable result |  |
| src-tauri/src/maps/manager.rs | Map prefix checkpoint has no owned output |  |
| src-tauri/src/maps/manager.rs | Map publication is no longer the current verified job |  |
| src-tauri/src/maps/manager.rs | Map publication ownership has an invalid filename |  |
| src-tauri/src/maps/manager.rs | Map recipe or source version changed; the retained prefix was not modified |  |
| src-tauri/src/maps/manager.rs | Map replacement authorization expired; select the map again |  |
| src-tauri/src/maps/manager.rs | Map state lock poisoned |  |
| src-tauri/src/maps/manager.rs | Map version changed; select the map again before downloading |  |
| src-tauri/src/maps/manager.rs | Map worker omitted its retirement result |  |
| src-tauri/src/maps/manager.rs | Map worker stopped unexpectedly; its last durable checkpoint was retained |  |
| src-tauri/src/maps/manager.rs | Map write failed: {error} |  |
| src-tauri/src/maps/manager.rs | Map writer stopped unexpectedly; its last durable checkpoint was retained |  |
| src-tauri/src/maps/manager.rs | Missing inline data |  |
| src-tauri/src/maps/manager.rs | Missing map job |  |
| src-tauri/src/maps/manager.rs | Missing pack URL |  |
| src-tauri/src/maps/manager.rs | Missing retained terminal map job |  |
| src-tauri/src/maps/manager.rs | Missing segment length |  |
| src-tauri/src/maps/manager.rs | No available map filename |  |
| src-tauri/src/maps/manager.rs | Only finished map downloads can be removed from history |  |
| src-tauri/src/maps/manager.rs | Pause the map before deleting its downloaded files |  |
| src-tauri/src/maps/manager.rs | Pause the map before preparing file cleanup |  |
| src-tauri/src/maps/manager.rs | Published map has no filename |  |
| src-tauri/src/maps/manager.rs | Published map path changed identity; verified inode {:?} in directory {:?} is retained for recovery; no output was removed |  |
| src-tauri/src/maps/manager.rs | Recovered map lost its publication stamp |  |
| src-tauri/src/maps/manager.rs | Resume or delete retained map data first. |  |
| src-tauri/src/maps/manager.rs | Resume, finish importing or delete the previous map download before starting another map |  |
| src-tauri/src/maps/manager.rs | The map cache is already being cleared |  |
| src-tauri/src/maps/manager.rs | The map cache is being cleared; try again |  |
| src-tauri/src/maps/manager.rs | The original downloaded map contents changed; no deletion was authorized |  |
| src-tauri/src/maps/manager.rs | The original downloaded map file changed while preparing cleanup |  |
| src-tauri/src/maps/manager.rs | The original downloaded map file changed; no files were authorized for deletion |  |
| src-tauri/src/maps/manager.rs | The original map directory changed; no files were authorized for deletion |  |
| src-tauri/src/maps/manager.rs | This map download changed unexpectedly. Delete the download and start again |  |
| src-tauri/src/maps/manager.rs | This map download does not match the selected map. Delete it and download the map again |  |
| src-tauri/src/maps/manager.rs | This map download is damaged. Delete the download to start again |  |
| src-tauri/src/maps/manager.rs | This map has no retained output checkpoint to resume; select Get map instead |  |
| src-tauri/src/maps/manager.rs | This map recovery entry is no longer in the current download. Refresh and try again |  |
| src-tauri/src/maps/manager.rs | This older map cannot be checked for damage. Remove the map from your library instead |  |
| src-tauri/src/maps/manager.rs | This older map cannot be removed from Kiwix. Remove the map from your library instead |  |
| src-tauri/src/maps/manager.rs | This older map has no owned publication stamp; remove the installed archive with its protected file action first |  |
| src-tauri/src/maps/manager.rs | This older map has no resumable recipe authorization |  |
| src-tauri/src/maps/manager.rs | Unknown map download |  |
| src-tauri/src/maps/manager.rs | Unknown map job |  |
| src-tauri/src/maps/manager.rs | Verified map output changed while authorizing its import |  |
| src-tauri/src/maps/manager.rs | composing |  |
| src-tauri/src/maps/manager.rs | map:{} |  |
| src-tauri/src/maps/manager.rs | md5 |  |
| src-tauri/src/maps/partials.rs | .kiwix-renamed-{:x}-{:x} |  |
| src-tauri/src/maps/partials.rs | Captured map contents changed; no bytes were deleted |  |
| src-tauri/src/maps/partials.rs | Captured map partial changed identity or size; entry was preserved |  |
| src-tauri/src/maps/partials.rs | Map checkpoint directory changed; no bytes were modified |  |
| src-tauri/src/maps/partials.rs | Map checkpoint file changed identity or length; no bytes were modified |  |
| src-tauri/src/maps/partials.rs | Map checkpoint has no resumable private output |  |
| src-tauri/src/maps/partials.rs | Map checkpoint moved or changed identity during verification; no bytes were modified |  |
| src-tauri/src/maps/partials.rs | Map cleanup capability was replaced; nothing was removed |  |
| src-tauri/src/maps/partials.rs | Map destination directory changed identity |  |
| src-tauri/src/maps/partials.rs | Map file length changed during ownership verification |  |
| src-tauri/src/maps/partials.rs | Map publication checkpoint has an invalid filename |  |
| src-tauri/src/maps/partials.rs | Map publication directory changed; retained entries were not modified |  |
| src-tauri/src/maps/partials.rs | Missing map cleanup capability |  |
| src-tauri/src/maps/partials.rs | Published map changed identity during recovery; no bytes were modified |  |
| src-tauri/src/maps/partials.rs | Published map directory changed; no bytes were deleted |  |
| src-tauri/src/maps/partials.rs | Published map file changed; no bytes were deleted |  |
| src-tauri/src/maps/partials.rs | Published map ownership changed; no bytes were deleted |  |
| src-tauri/src/maps/partials.rs | Recovered map output changed identity; entry was preserved |  |
| src-tauri/src/maps/partials.rs | Saved map prefix failed its checksum; delete the retained download or get the map again. No bytes were modified |  |
| src-tauri/src/maps/partials.rs | This older map cannot be checked for damage. Remove the map from your library, then download it again |  |
| src-tauri/src/maps/partials.rs | This partial map download was moved or changed, so Kiwix kept it. Delete the map download, then start it again |  |
| src-tauri/src/maps/recipe.rs | Inline data SHA-256 or length mismatch |  |
| src-tauri/src/maps/recipe.rs | Invalid StreetZim recipe or output filename |  |
| src-tauri/src/maps/recipe.rs | Invalid inline base64 |  |
| src-tauri/src/maps/recipe.rs | Invalid or duplicate StreetZim language version |  |
| src-tauri/src/maps/recipe.rs | Invalid or duplicate StreetZim region |  |
| src-tauri/src/maps/recipe.rs | Invalid source SHA-256 |  |
| src-tauri/src/maps/recipe.rs | Invalid source key |  |
| src-tauri/src/maps/recipe.rs | Invalid source mask bounds |  |
| src-tauri/src/maps/recipe.rs | Map recipe is too complex to estimate download size safely |  |
| src-tauri/src/maps/recipe.rs | Map transfer estimate overflow |  |
| src-tauri/src/maps/recipe.rs | Mask offset overflow |  |
| src-tauri/src/maps/recipe.rs | Missing pack ID |  |
| src-tauri/src/maps/recipe.rs | Missing pack key |  |
| src-tauri/src/maps/recipe.rs | Missing source masks |  |
| src-tauri/src/maps/recipe.rs | Recipe output size mismatch |  |
| src-tauri/src/maps/recipe.rs | Recipe segment size overflow |  |
| src-tauri/src/maps/recipe.rs | Segment is not a source range |  |
| src-tauri/src/maps/recipe.rs | Source segment exceeds source or copies masked bytes |  |
| src-tauri/src/maps/recipe.rs | StreetZim URLs must use HTTPS without credentials or fragments |  |
| src-tauri/src/maps/recipe.rs | This map download is damaged. Start the download again |  |
| src-tauri/src/maps/recipe.rs | Unknown segment or non-final MD5 segment |  |
| src-tauri/src/maps/recipe.rs | Unknown source pack |  |
| src-tauri/src/maps/recipe.rs | Unsupported StreetZim catalog format |  |
| src-tauri/src/maps/recipe.rs | pack |  |
| src-tauri/src/maps/recipe.rs | pack: |  |
| src-tauri/src/menus.rs | && |  |
| src-tauri/src/menus.rs | Install the application menu on the main thread |  |
| src-tauri/src/menus.rs | Invalid menu label |  |
| src-tauri/src/menus.rs | Invalid reader menu labels |  |
| src-tauri/src/menus.rs | Invalid tab selection labels |  |
| src-tauri/src/menus.rs | Kiwix |  |
| src-tauri/src/menus.rs | Library service unavailable during menu localization |  |
| src-tauri/src/menus.rs | Missing reader menu label |  |
| src-tauri/src/menus.rs | Not an application window |  |
| src-tauri/src/menus.rs | kiwix-application |  |
| src-tauri/src/migration.rs | A saved bookmark has an invalid link and was skipped |  |
| src-tauri/src/migration.rs | Bookmark URLs must be archive-local, not {scheme:?} |  |
| src-tauri/src/migration.rs | Bookmark book ID does not match its archive URL |  |
| src-tauri/src/migration.rs | Bookmark contains characters XML 1.0 cannot represent; use JSON export instead |  |
| src-tauri/src/migration.rs | Bookmark is missing its <book> descriptor |  |
| src-tauri/src/migration.rs | Bookmark is missing its <title> |  |
| src-tauri/src/migration.rs | Bookmark is missing its <url> |  |
| src-tauri/src/migration.rs | Choose a regular document to import |  |
| src-tauri/src/migration.rs | Expected a legacy <library> XML document or an OPDS <feed> |  |
| src-tauri/src/migration.rs | Expected a libkiwix <bookmarks>, <library>, or OPDS <feed> document |  |
| src-tauri/src/migration.rs | Import document exceeds 16 MiB |  |
| src-tauri/src/migration.rs | Import document is not valid UTF-8 |  |
| src-tauri/src/migration.rs | Import exceeds the 16 MiB limit; split the export into smaller files |  |
| src-tauri/src/migration.rs | Import is empty |  |
| src-tauri/src/migration.rs | Legacy archive path contains a NUL byte |  |
| src-tauri/src/migration.rs | Legacy file URL cannot be represented as a local path on this platform |  |
| src-tauri/src/migration.rs | Legacy file URL must not contain a query or fragment |  |
| src-tauri/src/migration.rs | Legacy library size overflows bytes |  |
| src-tauri/src/migration.rs | Library entry for {path:?} has no archive ID |  |
| src-tauri/src/migration.rs | Malformed or duplicate highlights extension |  |
| src-tauri/src/migration.rs | This library has no embedded bookmarks; import its separate library.bookmarks.xml file |  |
| src-tauri/src/migration.rs | Two library entries describe the same archive. Remove one before importing bookmarks |  |
| src-tauri/src/migration.rs | Unsupported bookmark export version {} (expected {BOOKMARK_EXPORT_VERSION}) |  |
| src-tauri/src/migration.rs | book |  |
| src-tauri/src/migration.rs | entry |  |
| src-tauri/src/migration.rs | tags |  |
| src-tauri/src/mobile.rs | Android Activity changed before window construction |  |
| src-tauri/src/mobile.rs | Android Activity identity missing |  |
| src-tauri/src/mobile.rs | Android window binding unavailable |  |
| src-tauri/src/mobile.rs | Android window incarnation missing |  |
| src-tauri/src/mobile.rs | Application handle unavailable |  |
| src-tauri/src/mobile.rs | Canonical Android main window configuration missing |  |
| src-tauri/src/mobile.rs | Canonical main window configuration missing |  |
| src-tauri/src/mobile.rs | Initial Android Activity identity missing |  |
| src-tauri/src/mobile.rs | Kiwix could not create its main window. {error} |  |
| src-tauri/src/mobile.rs | Kiwix could not find its application storage. {error} |  |
| src-tauri/src/mobile.rs | Kiwix has already started; the startup actions are no longer available |  |
| src-tauri/src/mobile.rs | Kiwix mobile finish-activity: {message} |  |
| src-tauri/src/mobile.rs | Native Activity foreground state missing |  |
| src-tauri/src/mobile.rs | Native Activity visibility missing |  |
| src-tauri/src/mobile.rs | Native incoming batch has no document identifiers |  |
| src-tauri/src/mobile.rs | Native incoming batch has no private ID |  |
| src-tauri/src/mobile.rs | Native incoming document queue is invalid |  |
| src-tauri/src/mobile.rs | Releasing rejected incoming document access failed: {message} |  |
| src-tauri/src/mobile.rs | The application storage location is unavailable |  |
| src-tauri/src/mobile.rs | Unknown startup action |  |
| src-tauri/src/mobile.rs | activity-window |  |
| src-tauri/src/mobile.rs | activityState |  |
| src-tauri/src/mobile.rs | app-entry |  |
| src-tauri/src/mobile.rs | background-download-recovery |  |
| src-tauri/src/mobile.rs | batches |  |
| src-tauri/src/mobile.rs | checkpoint-expired |  |
| src-tauri/src/mobile.rs | checkpoint-retiring |  |
| src-tauri/src/mobile.rs | completion-notification-tap |  |
| src-tauri/src/mobile.rs | deep-link |  |
| src-tauri/src/mobile.rs | document-progress |  |
| src-tauri/src/mobile.rs | dynamic-type |  |
| src-tauri/src/mobile.rs | finish-activity |  |
| src-tauri/src/mobile.rs | finishActivity |  |
| src-tauri/src/mobile.rs | foreground |  |
| src-tauri/src/mobile.rs | foreground-retirement |  |
| src-tauri/src/mobile.rs | hardware-shortcut |  |
| src-tauri/src/mobile.rs | hotspot |  |
| src-tauri/src/mobile.rs | hotspot-stopped |  |
| src-tauri/src/mobile.rs | incoming |  |
| src-tauri/src/mobile.rs | incoming-documents |  |
| src-tauri/src/mobile.rs | lifecycle |  |
| src-tauri/src/mobile.rs | live-activity |  |
| src-tauri/src/mobile.rs | local-transfers |  |
| src-tauri/src/mobile.rs | mobile-inactive |  |
| src-tauri/src/mobile.rs | mobile-resumed |  |
| src-tauri/src/mobile.rs | mobile-suspended |  |
| src-tauri/src/mobile.rs | native-listeners |  |
| src-tauri/src/mobile.rs | native-transfer-events |  |
| src-tauri/src/mobile.rs | nearby-bluetooth |  |
| src-tauri/src/mobile.rs | nearby-join |  |
| src-tauri/src/mobile.rs | network-policy |  |
| src-tauri/src/mobile.rs | network-policy-changed |  |
| src-tauri/src/mobile.rs | network-resume |  |
| src-tauri/src/mobile.rs | quit |  |
| src-tauri/src/mobile.rs | reader-audio |  |
| src-tauri/src/mobile.rs | releaseIncomingOriginals |  |
| src-tauri/src/mobile.rs | requestId |  |
| src-tauri/src/mobile.rs | resume-interrupted |  |
| src-tauri/src/mobile.rs | retireWindow |  |
| src-tauri/src/mobile.rs | speech-action |  |
| src-tauri/src/mobile.rs | speech-state |  |
| src-tauri/src/mobile.rs | transfers |  |
| src-tauri/src/mobile.rs | transfers-error |  |
| src-tauri/src/mobile.rs | transfers-pause |  |
| src-tauri/src/mobile.rs | transfers-timeout |  |
| src-tauri/src/mobile.rs | uris |  |
| src-tauri/src/mobile.rs | visible |  |
| src-tauri/src/mobile.rs | windowId |  |
| src-tauri/src/mobile_backup.rs | Android archive backup is excluded by static platform rules |  |
| src-tauri/src/mobile_backup.rs | iOS could not apply the owned archive backup policy |  |
| src-tauri/src/mobile_backup.rs | iOS could not conditionally exclude the owned payload |  |
| src-tauri/src/mobile_backup.rs | iOS could not inspect owned archive admission |  |
| src-tauri/src/mobile_policy.rs | MOBILE_LOCAL_NETWORK_BLOCKED |  |
| src-tauri/src/mobile_policy.rs | Map manager unavailable |  |
| src-tauri/src/mobile_policy.rs | Native app entry value is too large |  |
| src-tauri/src/mobile_policy.rs | Network changed during transfer admission; start or resume again |  |
| src-tauri/src/mobile_policy.rs | This native app entry does not accept a value |  |
| src-tauri/src/mobile_policy.rs | Torrent manager unavailable |  |
| src-tauri/src/mobile_policy.rs | Turn on Wi-Fi and connect both devices to the same network, or create a hotspot. Internet access is not required. |  |
| src-tauri/src/mobile_policy.rs | Unsupported native app entry kind |  |
| src-tauri/src/mobile_policy.rs | Waiting for network status. Try again when the connection is ready. |  |
| src-tauri/src/mobile_policy.rs | bookmarks |  |
| src-tauri/src/mobile_policy.rs | sharing |  |
| src-tauri/src/mobile_policy.rs | voice-search |  |
| src-tauri/src/mobile_rebase.rs | .json |  |
| src-tauri/src/mobile_rebase.rs | A saved file changed while Kiwix was moving it. Restart the app |  |
| src-tauri/src/mobile_rebase.rs | A saved file is too large to move. Restart the app |  |
| src-tauri/src/mobile_rebase.rs | Conflicting torrent receipt path correlations |  |
| src-tauri/src/mobile_rebase.rs | Created mobile root capability disappeared |  |
| src-tauri/src/mobile_rebase.rs | Invalid recovery directory component |  |
| src-tauri/src/mobile_rebase.rs | Invalid relocated path component |  |
| src-tauri/src/mobile_rebase.rs | Invalid torrent receipt job ID |  |
| src-tauri/src/mobile_rebase.rs | Kiwix: mobile root marker is unreadable: {error} |  |
| src-tauri/src/mobile_rebase.rs | Mobile filesystem path contains traversal: {} |  |
| src-tauri/src/mobile_rebase.rs | Mobile filesystem path is not absolute: {} |  |
| src-tauri/src/mobile_rebase.rs | Mobile filesystem path is not normalized: {} |  |
| src-tauri/src/mobile_rebase.rs | Mobile path is not UTF-8 |  |
| src-tauri/src/mobile_rebase.rs | Mobile relocation intent exceeds its size bound |  |
| src-tauri/src/mobile_rebase.rs | Mobile root capability must be owned and private (0700) |  |
| src-tauri/src/mobile_rebase.rs | Non-UTF-8 recovery directory |  |
| src-tauri/src/mobile_rebase.rs | Non-UTF-8 relocated path component |  |
| src-tauri/src/mobile_rebase.rs | Persisted filesystem path is not a string |  |
| src-tauri/src/mobile_rebase.rs | Recovery directory has no identity evidence |  |
| src-tauri/src/mobile_rebase.rs | Relocated directory disappeared |  |
| src-tauri/src/mobile_rebase.rs | Relocated path crosses a non-directory |  |
| src-tauri/src/mobile_rebase.rs | Relocated path crosses a symlink: {} |  |
| src-tauri/src/mobile_rebase.rs | Relocated recovery directory changed identity: {} |  |
| src-tauri/src/mobile_rebase.rs | Relocation capability must be owned and not writable by other users |  |
| src-tauri/src/mobile_rebase.rs | Relocation directory is not a real directory: {name} |  |
| src-tauri/src/mobile_rebase.rs | Relocation publication identity changed |  |
| src-tauri/src/mobile_rebase.rs | Relocation state must be a regular file without hard links |  |
| src-tauri/src/mobile_rebase.rs | Saved transfer history is damaged. Restore a backup |  |
| src-tauri/src/mobile_rebase.rs | The storage folder changed while Kiwix was moving files. Restart the app |  |
| src-tauri/src/mobile_rebase.rs | Torrent completed path is not UTF-8 |  |
| src-tauri/src/mobile_rebase.rs | Torrent completed path is not a string |  |
| src-tauri/src/mobile_rebase.rs | Torrent job has no ID |  |
| src-tauri/src/mobile_rebase.rs | Torrent job has no completed paths array |  |
| src-tauri/src/mobile_rebase.rs | Torrent receipt source path is not UTF-8 |  |
| src-tauri/src/mobile_rebase.rs | Torrent state has no jobs array |  |
| src-tauri/src/mobile_rebase.rs | completedPaths |  |
| src-tauri/src/mobile_rebase.rs | job |  |
| src-tauri/src/mobile_rebase.rs | jobs |  |
| src-tauri/src/mobile_rebase.rs | pending.json |  |
| src-tauri/src/mobile_rebase.rs | torrent: |  |
| src-tauri/src/mobile_rebase.rs | {id}:{} |  |
| src-tauri/src/mobile_service.rs | .xml |  |
| src-tauri/src/mobile_service.rs | A native Pause or timeout must be durably reconciled before starting transfers |  |
| src-tauri/src/mobile_service.rs | ACTIVITY_RECREATED |  |
| src-tauri/src/mobile_service.rs | Add archive folder |  |
| src-tauri/src/mobile_service.rs | Add files |  |
| src-tauri/src/mobile_service.rs | Archive cache unavailable |  |
| src-tauri/src/mobile_service.rs | Archive changed while reconnecting |  |
| src-tauri/src/mobile_service.rs | Archive to reconnect is no longer in the library |  |
| src-tauri/src/mobile_service.rs | Archive transfers |  |
| src-tauri/src/mobile_service.rs | Background execution time ended; transfer retirement remains pending until execution resumes |  |
| src-tauri/src/mobile_service.rs | Byte export exceeds 16 MiB |  |
| src-tauri/src/mobile_service.rs | Byte export exceeds 16 MiB; use streaming resource export |  |
| src-tauri/src/mobile_service.rs | Cancel |  |
| src-tauri/src/mobile_service.rs | Close this file and open it again after changing privacy settings |  |
| src-tauri/src/mobile_service.rs | Confirm or dismiss pending incoming archives before opening more documents |  |
| src-tauri/src/mobile_service.rs | Copying documents… |  |
| src-tauri/src/mobile_service.rs | Copying or moving archives requires explicit confirmation |  |
| src-tauri/src/mobile_service.rs | Creating the Wi-Fi hotspot failed: {error} |  |
| src-tauri/src/mobile_service.rs | Device scanning is unavailable in private mode. Leave private mode to look for ZIM files on this device. |  |
| src-tauri/src/mobile_service.rs | Device scanning is unavailable in private mode. Leave private mode to open a ZIM file from this device. |  |
| src-tauri/src/mobile_service.rs | Folder access cannot be saved in private mode. Leave private mode to scan this device. |  |
| src-tauri/src/mobile_service.rs | Handle pending app entry requests before opening more |  |
| src-tauri/src/mobile_service.rs | Hotspot state unavailable |  |
| src-tauri/src/mobile_service.rs | Import a folder copy |  |
| src-tauri/src/mobile_service.rs | Import a managed copy |  |
| src-tauri/src/mobile_service.rs | Import document |  |
| src-tauri/src/mobile_service.rs | Incoming batch contains no complete archive |  |
| src-tauri/src/mobile_service.rs | Incoming document scheme is not supported by this native provider |  |
| src-tauri/src/mobile_service.rs | Invalid archive replacement identifier |  |
| src-tauri/src/mobile_service.rs | Invalid export MIME type |  |
| src-tauri/src/mobile_service.rs | Invalid incoming document identifier |  |
| src-tauri/src/mobile_service.rs | Invalid mobile background request identity |  |
| src-tauri/src/mobile_service.rs | Invalid monitored mobile archive folder |  |
| src-tauri/src/mobile_service.rs | Invalid native transfer event ID |  |
| src-tauri/src/mobile_service.rs | Invalid platform locale preservation choice |  |
| src-tauri/src/mobile_service.rs | Invalid reader media state |  |
| src-tauri/src/mobile_service.rs | Invalid theme |  |
| src-tauri/src/mobile_service.rs | Kiwix could not finish a previous library storage move. {error} |  |
| src-tauri/src/mobile_service.rs | Kiwix mobile admission rejected {command}: active={} desired_active={} inactive={} background={} |  |
| src-tauri/src/mobile_service.rs | LOCAL_NETWORK_PERMISSION_DENIED |  |
| src-tauri/src/mobile_service.rs | Leave private or read-only mode before replacing a saved external archive reference |  |
| src-tauri/src/mobile_service.rs | Lifecycle retirement is still pending; foreground activation will retry |  |
| src-tauri/src/mobile_service.rs | Mobile native actions require the trusted root window |  |
| src-tauri/src/mobile_service.rs | Mobile native adapter is unavailable |  |
| src-tauri/src/mobile_service.rs | Mobile retirement revision exhausted |  |
| src-tauri/src/mobile_service.rs | Native app entry ID must be canonical |  |
| src-tauri/src/mobile_service.rs | Native app entry changed its immutable payload |  |
| src-tauri/src/mobile_service.rs | Native app entry queue is invalid |  |
| src-tauri/src/mobile_service.rs | Native background checkpoint state unavailable: {error} |  |
| src-tauri/src/mobile_service.rs | Native background checkpoint unavailable: {error} |  |
| src-tauri/src/mobile_service.rs | Native directory picker returned an unexpected owned directory |  |
| src-tauri/src/mobile_service.rs | Native export did not return a completion state |  |
| src-tauri/src/mobile_service.rs | Native export did not return a completion state; prepared export retained at {} |  |
| src-tauri/src/mobile_service.rs | Native external archive selection missing |  |
| src-tauri/src/mobile_service.rs | Native folder picker did not return cancellation state |  |
| src-tauri/src/mobile_service.rs | Native import did not return cancellation state |  |
| src-tauri/src/mobile_service.rs | Native incoming batch ID must be canonical |  |
| src-tauri/src/mobile_service.rs | Native incoming batch changed its immutable source identifiers |  |
| src-tauri/src/mobile_service.rs | Native original archive errors missing |  |
| src-tauri/src/mobile_service.rs | Native original archive selections missing |  |
| src-tauri/src/mobile_service.rs | Native original selection missing |  |
| src-tauri/src/mobile_service.rs | Native pending transfer event queue is invalid |  |
| src-tauri/src/mobile_service.rs | Native picker did not return cancellation state |  |
| src-tauri/src/mobile_service.rs | Native reader did not return cancellation state |  |
| src-tauri/src/mobile_service.rs | Native scanned archive selection missing |  |
| src-tauri/src/mobile_service.rs | Native transfer event ID is not canonical |  |
| src-tauri/src/mobile_service.rs | No complete ZIM archives were found in the selected location |  |
| src-tauri/src/mobile_service.rs | OS Trash is not supported on this mobile platform |  |
| src-tauri/src/mobile_service.rs | Open external archive |  |
| src-tauri/src/mobile_service.rs | Pause |  |
| src-tauri/src/mobile_service.rs | Pinning articles is unavailable in private or read-only mode |  |
| src-tauri/src/mobile_service.rs | Reader storage did not acknowledge the finite background checkpoint deadline |  |
| src-tauri/src/mobile_service.rs | Selected file is a different archive; open it separately |  |
| src-tauri/src/mobile_service.rs | Start or resume transfers while the application is visible |  |
| src-tauri/src/mobile_service.rs | Start the Wi-Fi hotspot before sharing over it |  |
| src-tauri/src/mobile_service.rs | Stop sharing the selected archive before copying or moving it |  |
| src-tauri/src/mobile_service.rs | Stopping the Wi-Fi hotspot failed: {error} |  |
| src-tauri/src/mobile_service.rs | Text export exceeds 16 MiB |  |
| src-tauri/src/mobile_service.rs | The Wi-Fi hotspot start finished after the app left the foreground |  |
| src-tauri/src/mobile_service.rs | The Wi-Fi hotspot start was stopped before it completed |  |
| src-tauri/src/mobile_service.rs | The mobile background checkpoint ended without a durability acknowledgement |  |
| src-tauri/src/mobile_service.rs | The mobile background checkpoint was not issued by its lifecycle transition |  |
| src-tauri/src/mobile_service.rs | The saved library location was not recognized, so Kiwix uses internal app storage. Existing files were kept. |  |
| src-tauri/src/mobile_service.rs | The selected library storage is unavailable. Reconnect it or choose another location in Settings. |  |
| src-tauri/src/mobile_service.rs | The temporary Wi-Fi permission dialog was not completed. Dismiss it and try again. |  |
| src-tauri/src/mobile_service.rs | This article location cannot be pinned |  |
| src-tauri/src/mobile_service.rs | This pinned article link is too long to open |  |
| src-tauri/src/mobile_service.rs | Too many incoming documents or an oversized document identifier |  |
| src-tauri/src/mobile_service.rs | Transfer authorization was superseded by a lifecycle or native Pause action; resume after reconciliation |  |
| src-tauri/src/mobile_service.rs | Transfer retirement is still pending; foreground admission remains closed and will retry when its owner completes |  |
| src-tauri/src/mobile_service.rs | Transfer retirement worker stopped before completion |  |
| src-tauri/src/mobile_service.rs | Transferring archives |  |
| src-tauri/src/mobile_service.rs | Transfers |  |
| src-tauri/src/mobile_service.rs | Unknown incoming document request |  |
| src-tauri/src/mobile_service.rs | Unknown mobile command: {command} |  |
| src-tauri/src/mobile_service.rs | Unknown native transfer control event |  |
| src-tauri/src/mobile_service.rs | Unknown or already handled incoming document request |  |
| src-tauri/src/mobile_service.rs | Unknown or already imported incoming document request |  |
| src-tauri/src/mobile_service.rs | Use the native mobile document import/export action |  |
| src-tauri/src/mobile_service.rs | VISIBLE_ACTIVITY_REQUIRED |  |
| src-tauri/src/mobile_service.rs | active |  |
| src-tauri/src/mobile_service.rs | application/xml |  |
| src-tauri/src/mobile_service.rs | applyLocale |  |
| src-tauri/src/mobile_service.rs | applyTheme |  |
| src-tauri/src/mobile_service.rs | audible |  |
| src-tauri/src/mobile_service.rs | body |  |
| src-tauri/src/mobile_service.rs | bookId |  |
| src-tauri/src/mobile_service.rs | bookIds |  |
| src-tauri/src/mobile_service.rs | books |  |
| src-tauri/src/mobile_service.rs | bytes |  |
| src-tauri/src/mobile_service.rs | channelName |  |
| src-tauri/src/mobile_service.rs | completed |  |
| src-tauri/src/mobile_service.rs | content |  |
| src-tauri/src/mobile_service.rs | copyIncomingArchives |  |
| src-tauri/src/mobile_service.rs | copying |  |
| src-tauri/src/mobile_service.rs | count |  |
| src-tauri/src/mobile_service.rs | crashCapturePolicy |  |
| src-tauri/src/mobile_service.rs | dark |  |
| src-tauri/src/mobile_service.rs | destinationDirectory |  |
| src-tauri/src/mobile_service.rs | discoveryState |  |
| src-tauri/src/mobile_service.rs | downloadDirectory |  |
| src-tauri/src/mobile_service.rs | edition_download |  |
| src-tauri/src/mobile_service.rs | enabled |  |
| src-tauri/src/mobile_service.rs | endCheckpoint |  |
| src-tauri/src/mobile_service.rs | entries |  |
| src-tauri/src/mobile_service.rs | events |  |
| src-tauri/src/mobile_service.rs | exportFile |  |
| src-tauri/src/mobile_service.rs | folder |  |
| src-tauri/src/mobile_service.rs | hotspotStop |  |
| src-tauri/src/mobile_service.rs | kind |  |
| src-tauri/src/mobile_service.rs | kiwix-mobile-entry |  |
| src-tauri/src/mobile_service.rs | kiwix-mobile-incoming |  |
| src-tauri/src/mobile_service.rs | labels |  |
| src-tauri/src/mobile_service.rs | library-notice |  |
| src-tauri/src/mobile_service.rs | light |  |
| src-tauri/src/mobile_service.rs | local-transfer-retirement |  |
| src-tauri/src/mobile_service.rs | locale |  |
| src-tauri/src/mobile_service.rs | localswarm_download |  |
| src-tauri/src/mobile_service.rs | localswarm_seed |  |
| src-tauri/src/mobile_service.rs | localswarm_share_all |  |
| src-tauri/src/mobile_service.rs | map-checkpoint |  |
| src-tauri/src/mobile_service.rs | map_resume |  |
| src-tauri/src/mobile_service.rs | map_start |  |
| src-tauri/src/mobile_service.rs | maxBytes |  |
| src-tauri/src/mobile_service.rs | maxEntries |  |
| src-tauri/src/mobile_service.rs | media-reader-retirement |  |
| src-tauri/src/mobile_service.rs | media-retirement |  |
| src-tauri/src/mobile_service.rs | media-spelling-retirement |  |
| src-tauri/src/mobile_service.rs | mime |  |
| src-tauri/src/mobile_service.rs | mimeType |  |
| src-tauri/src/mobile_service.rs | mobile_background_ack |  |
| src-tauri/src/mobile_service.rs | mobile_save_bytes |  |
| src-tauri/src/mobile_service.rs | mobile_save_text |  |
| src-tauri/src/mobile_service.rs | native-incoming:{id} |  |
| src-tauri/src/mobile_service.rs | open |  |
| src-tauri/src/mobile_service.rs | paths |  |
| src-tauri/src/mobile_service.rs | pauseLabel |  |
| src-tauri/src/mobile_service.rs | persistent |  |
| src-tauri/src/mobile_service.rs | pickArchiveCopies |  |
| src-tauri/src/mobile_service.rs | pickExternalArchive |  |
| src-tauri/src/mobile_service.rs | pickLibraryDirectoryCopy |  |
| src-tauri/src/mobile_service.rs | pickScanFolder |  |
| src-tauri/src/mobile_service.rs | pictureInPicture |  |
| src-tauri/src/mobile_service.rs | pinArticleShortcut |  |
| src-tauri/src/mobile_service.rs | progress |  |
| src-tauri/src/mobile_service.rs | readTextDocument |  |
| src-tauri/src/mobile_service.rs | readerAudioState |  |
| src-tauri/src/mobile_service.rs | release |  |
| src-tauri/src/mobile_service.rs | resume_download |  |
| src-tauri/src/mobile_service.rs | root |  |
| src-tauri/src/mobile_service.rs | scanDeviceArchives |  |
| src-tauri/src/mobile_service.rs | scan_folder |  |
| src-tauri/src/mobile_service.rs | selection |  |
| src-tauri/src/mobile_service.rs | share |  |
| src-tauri/src/mobile_service.rs | share_torrent |  |
| src-tauri/src/mobile_service.rs | shared_library_download |  |
| src-tauri/src/mobile_service.rs | sharing_start |  |
| src-tauri/src/mobile_service.rs | start_download |  |
| src-tauri/src/mobile_service.rs | suggestedName |  |
| src-tauri/src/mobile_service.rs | system |  |
| src-tauri/src/mobile_service.rs | target |  |
| src-tauri/src/mobile_service.rs | text |  |
| src-tauri/src/mobile_service.rs | text/plain |  |
| src-tauri/src/mobile_service.rs | theme |  |
| src-tauri/src/mobile_service.rs | token |  |
| src-tauri/src/mobile_service.rs | torrent_seeding |  |
| src-tauri/src/mobile_service.rs | total |  |
| src-tauri/src/mobile_service.rs | transferState |  |
| src-tauri/src/mobile_service.rs | uri |  |
| src-tauri/src/mobile_service.rs | {error}. {stop} |  |
| src-tauri/src/mobile_service.rs | {error}; releasing external selection failed: {cleanup} |  |
| src-tauri/src/mobile_service.rs | {error}; releasing scanned archive selection failed: {cleanup} |  |
| src-tauri/src/mobile_speech.rs | Browsing session expired after a privacy transition |  |
| src-tauri/src/mobile_speech.rs | Invalid speech arguments |  |
| src-tauri/src/mobile_speech.rs | Kiwix read-aloud activity: {error} |  |
| src-tauri/src/mobile_speech.rs | Missing speech argument: rate |  |
| src-tauri/src/mobile_speech.rs | Missing speech argument: {key} |  |
| src-tauri/src/mobile_speech.rs | Mobile native speech adapter is unavailable |  |
| src-tauri/src/mobile_speech.rs | Native speech preparation returned no capability |  |
| src-tauri/src/mobile_speech.rs | Native speech requires the trusted root window |  |
| src-tauri/src/mobile_speech.rs | Natural voices are off in private and read-only mode; choose a system voice |  |
| src-tauri/src/mobile_speech.rs | Natural voices are only available for English; choose an installed system voice |  |
| src-tauri/src/mobile_speech.rs | Paused |  |
| src-tauri/src/mobile_speech.rs | Reading aloud |  |
| src-tauri/src/mobile_speech.rs | SPEECH_ARCHIVE_INSTALLATION_RETIRED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_BUSY |  |
| src-tauri/src/mobile_speech.rs | SPEECH_FOREGROUND_REQUIRED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_INVALID_REQUEST |  |
| src-tauri/src/mobile_speech.rs | SPEECH_READER_LISTENER_RETIRED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_READER_SESSION_RETIRED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_SEQUENCE_EXHAUSTED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_SESSION_CONSUMED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_SESSION_REGISTERED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_SESSION_RETIRED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_SESSION_SUPERSEDED |  |
| src-tauri/src/mobile_speech.rs | SPEECH_SOURCE_RETIRING |  |
| src-tauri/src/mobile_speech.rs | Unknown native speech command |  |
| src-tauri/src/mobile_speech.rs | accepted |  |
| src-tauri/src/mobile_speech.rs | archiveId |  |
| src-tauri/src/mobile_speech.rs | en |  |
| src-tauri/src/mobile_speech.rs | kiwix-mobile-error |  |
| src-tauri/src/mobile_speech.rs | kokoro: |  |
| src-tauri/src/mobile_speech.rs | lang |  |
| src-tauri/src/mobile_speech.rs | main:{} |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_forward |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_pause |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_prepare |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_restore_reader |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_retire |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_rewind |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_seek |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_start |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_stop |  |
| src-tauri/src/mobile_speech.rs | mobile_speech_voices |  |
| src-tauri/src/mobile_speech.rs | operation |  |
| src-tauri/src/mobile_speech.rs | pause |  |
| src-tauri/src/mobile_speech.rs | rate |  |
| src-tauri/src/mobile_speech.rs | readAloudActivity |  |
| src-tauri/src/mobile_speech.rs | seconds |  |
| src-tauri/src/mobile_speech.rs | sessionId |  |
| src-tauri/src/mobile_speech.rs | speech-reader-retirement |  |
| src-tauri/src/mobile_speech.rs | speech-retire |  |
| src-tauri/src/mobile_speech.rs | speech-retirement |  |
| src-tauri/src/mobile_speech.rs | speech-spelling-retirement |  |
| src-tauri/src/mobile_speech.rs | speechEngineStop |  |
| src-tauri/src/mobile_speech.rs | speechVoices |  |
| src-tauri/src/mobile_speech.rs | start |  |
| src-tauri/src/mobile_speech.rs | state |  |
| src-tauri/src/mobile_speech.rs | voice |  |
| src-tauri/src/mobile_speech.rs | {}:{} |  |
| src-tauri/src/native_pdf.rs | %PDF- |  |
| src-tauri/src/native_pdf.rs | <article image> |  |
| src-tauri/src/native_pdf.rs | <article origin> |  |
| src-tauri/src/native_pdf.rs | Another article PDF export is still running or finishing |  |
| src-tauri/src/native_pdf.rs | Article PDF export timed out |  |
| src-tauri/src/native_pdf.rs | Article PDF export was cancelled |  |
| src-tauri/src/native_pdf.rs | Native PDF is missing, invalid, or exceeds the 64 MiB output limit |  |
| src-tauri/src/native_pdf.rs | Not enough temporary storage for article PDF export |  |
| src-tauri/src/native_pdf.rs | System sharing is unavailable on this platform |  |
| src-tauri/src/native_pdf.rs | The article PDF changed before sharing. Export it again |  |
| src-tauri/src/native_pdf.rs | The article PDF has an unexpected file link |  |
| src-tauri/src/native_pdf.rs | The article PDF was not validated |  |
| src-tauri/src/native_pdf.rs | The article image could not be read. Try the export again |  |
| src-tauri/src/native_pdf.rs | The article image stopped loading. Try the export again |  |
| src-tauri/src/native_pdf.rs | The native provider did not produce a PDF document |  |
| src-tauri/src/native_pdf.rs | The native provider did not produce a usable PDF document |  |
| src-tauri/src/native_pdf.rs | content-type |  |
| src-tauri/src/native_pdf.rs | iOS could not exclude PDF temporary storage from backup |  |
| src-tauri/src/native_pdf.rs | invalid archive resource path |  |
| src-tauri/src/native_pdf.rs | invalid resource media type |  |
| src-tauri/src/native_pdf.rs | missing archive resource |  |
| src-tauri/src/native_pdf.rs | native origin stopped |  |
| src-tauri/src/native_pdf.rs | pdf-{} |  |
| src-tauri/src/native_pdf.rs | the builder is only configured with a Unix mode |  |
| src-tauri/src/native_pdf/android.rs | Android PDF sharing returned no completion state |  |
| src-tauri/src/native_pdf/android.rs | Android article PDF is invalid or exceeds the 64 MiB limit |  |
| src-tauri/src/native_pdf/android.rs | Android did not open the native PDF print UI |  |
| src-tauri/src/native_pdf/android.rs | Android did not produce a PDF document |  |
| src-tauri/src/native_pdf/android.rs | Android did not render the live article PDF |  |
| src-tauri/src/native_pdf/android.rs | Article PDF sharing was cancelled |  |
| src-tauri/src/native_pdf/android.rs | Reading article PDF: {error} |  |
| src-tauri/src/native_pdf/apple.rs | Activate the original article window and try printing again |  |
| src-tauri/src/native_pdf/apple.rs | Article PDF print authority expired | authority |
| src-tauri/src/native_pdf/apple.rs | Article PDF printing was cancelled |  |
| src-tauri/src/native_pdf/apple.rs | Cannot create the native print operation |  |
| src-tauri/src/native_pdf/apple.rs | Cannot open the rendered article PDF for printing |  |
| src-tauri/src/native_pdf/apple.rs | Close the current system print dialog before printing another article |  |
| src-tauri/src/native_pdf/apple.rs | Close the open dialog in the article window before printing |  |
| src-tauri/src/native_pdf/apple.rs | Invalid PDF output path |  |
| src-tauri/src/native_pdf/apple.rs | Invalid PDF print path |  |
| src-tauri/src/native_pdf/apple.rs | Invalid PDF print title |  |
| src-tauri/src/native_pdf/apple.rs | PDF output path is not UTF-8 |  |
| src-tauri/src/native_pdf/apple.rs | PDF print path is not UTF-8 |  |
| src-tauri/src/native_pdf/apple.rs | The native print operation failed ({system_error}) |  |
| src-tauri/src/native_pdf/apple.rs | The native print panel could not be opened |  |
| src-tauri/src/native_pdf/apple.rs | The system print dialog timed out |  |
| src-tauri/src/native_pdf/cleanup.rs | Could not remove private article PDF temporary files: {error} |  |
| src-tauri/src/native_pdf/linux.rs | GTK could not open the system print dialog |  |
| src-tauri/src/native_pdf/linux.rs | GTK printer discovery is already in progress |  |
| src-tauri/src/native_pdf/linux.rs | GTK printer discovery timed out |  |
| src-tauri/src/native_pdf/linux.rs | GTK printer enumeration did not finish in time; refusing to use a partial list |  |
| src-tauri/src/native_pdf/linux.rs | GTK reports an ambiguous file-printer name; refusing to choose a print backend |  |
| src-tauri/src/native_pdf/linux.rs | GTK's native PDF file-print backend is unavailable; no printer was selected |  |
| src-tauri/src/native_pdf/linux.rs | Invalid native PDF scratch path |  |
| src-tauri/src/native_pdf/linux.rs | Invalid print title |  |
| src-tauri/src/native_pdf/linux.rs | Invalid printer page dimensions |  |
| src-tauri/src/native_pdf/linux.rs | Native GTK PDF provider stopped |  |
| src-tauri/src/native_pdf/linux.rs | Native PDF resource load failed: {error} |  |
| src-tauri/src/native_pdf/linux.rs | No system printer was selected |  |
| src-tauri/src/native_pdf/linux.rs | The system print settings are unavailable |  |
| src-tauri/src/native_pdf/linux.rs | output-file-format |  |
| src-tauri/src/native_pdf/linux.rs | output-uri |  |
| src-tauri/src/native_pdf/linux.rs | pdf |  |
| src-tauri/src/native_pdf/live.rs | </style> |  |
| src-tauri/src/native_pdf/live.rs | =\" |  |
| src-tauri/src/native_pdf/live.rs | Invalid live article identity |  |
| src-tauri/src/native_pdf/live.rs | The article capture is empty or exceeds the 16 MiB limit |  |
| src-tauri/src/native_pdf/live.rs | The live article contains a style element with child elements; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs | The live article contains an uncaptured frame, media surface, or canvas; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs | The live article contains an uncaptured visual resource; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs | The live article contains an unfrozen SVG animation; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs | The live article contains an unresolved responsive image; no PDF was saved |  |
| src-tauri/src/native_pdf/live.rs | The live article exceeds the 512-page PDF limit |  |
| src-tauri/src/native_pdf/live.rs | The live article has unsupported capture dimensions |  |
| src-tauri/src/native_pdf/live.rs | This article has too many nested elements to export as a PDF |  |
| src-tauri/src/native_pdf/live.rs | animate |  |
| src-tauri/src/native_pdf/live.rs | animateMotion |  |
| src-tauri/src/native_pdf/live.rs | animateTransform |  |
| src-tauri/src/native_pdf/live.rs | audio |  |
| src-tauri/src/native_pdf/live.rs | browser-icon |  |
| src-tauri/src/native_pdf/live.rs | canvas |  |
| src-tauri/src/native_pdf/live.rs | clip-path |  |
| src-tauri/src/native_pdf/live.rs | cursor |  |
| src-tauri/src/native_pdf/live.rs | discard |  |
| src-tauri/src/native_pdf/live.rs | embed |  |
| src-tauri/src/native_pdf/live.rs | fill |  |
| src-tauri/src/native_pdf/live.rs | filter |  |
| src-tauri/src/native_pdf/live.rs | frame |  |
| src-tauri/src/native_pdf/live.rs | frameset |  |
| src-tauri/src/native_pdf/live.rs | iframe |  |
| src-tauri/src/native_pdf/live.rs | marker-end |  |
| src-tauri/src/native_pdf/live.rs | marker-mid |  |
| src-tauri/src/native_pdf/live.rs | marker-start |  |
| src-tauri/src/native_pdf/live.rs | mask |  |
| src-tauri/src/native_pdf/live.rs | object |  |
| src-tauri/src/native_pdf/live.rs | set |  |
| src-tauri/src/native_pdf/live.rs | stroke |  |
| src-tauri/src/native_pdf/live.rs | style |  |
| src-tauri/src/native_pdf/live.rs | video |  |
| src-tauri/src/native_pdf/windows.rs | A current InPrivate-capable WebView2 runtime is required for PDF export: {error} |  |
| src-tauri/src/native_pdf/windows.rs | Accessing the native PDF view: {error} |  |
| src-tauri/src/native_pdf/windows.rs | Missing PDF scratch directory |  |
| src-tauri/src/native_pdf/windows.rs | Native PDF export failed: {error} |  |
| src-tauri/src/native_pdf/windows.rs | Native PDF export failed: {e} |  |
| src-tauri/src/native_pdf/windows.rs | Native WebView2 PDF provider stopped |  |
| src-tauri/src/native_pdf/windows.rs | Queueing the Windows print dialog: {error} |  |
| src-tauri/src/native_pdf/windows.rs | Sending the article to the Windows printer: {error} |  |
| src-tauri/src/native_pdf/windows.rs | The WebView2 runtime cannot create a private article PDF view |  |
| src-tauri/src/native_pdf/windows.rs | The Windows print spool rejected the job ({}) |  |
| src-tauri/src/native_pdf/windows.rs | The native PDF view was closed |  |
| src-tauri/src/native_pdf/windows.rs | WebView2 could not load the article for PDF export |  |
| src-tauri/src/native_pdf/windows.rs | WebView2 did not save the article PDF |  |
| src-tauri/src/native_pdf/windows.rs | about:blank |  |
| src-tauri/src/native_pdf/windows.rs | webview |  |
| src-tauri/src/native_pdf/windows_print.rs | Article printing was cancelled |  |
| src-tauri/src/native_pdf/windows_print.rs | Invalid print page range |  |
| src-tauri/src/native_pdf/windows_print.rs | Invalid printer identity |  |
| src-tauri/src/native_pdf/windows_print.rs | Invalid printer paper dimensions |  |
| src-tauri/src/native_pdf/windows_print.rs | Invalid printer resolution |  |
| src-tauri/src/native_pdf/windows_print.rs | Missing printer configuration |  |
| src-tauri/src/native_pdf/windows_print.rs | Missing printer identity |  |
| src-tauri/src/native_pdf/windows_print.rs | No printer was selected |  |
| src-tauri/src/native_pdf/windows_print.rs | The Windows print dialog could not start its cancellation timer |  |
| src-tauri/src/native_pdf/windows_print.rs | The Windows print dialog failed ({error}) |  |
| src-tauri/src/native_pdf/windows_print.rs | The live article is too wide for WebView2 PDF capture |  |
| src-tauri/src/native_pdf/windows_print.rs | The live article is too wide for the selected printer scale |  |
| src-tauri/src/native_pdf/windows_print.rs | Unsupported printer duplex setting |  |
| src-tauri/src/native_pdf/windows_print.rs | {first}-{last} |  |
| src-tauri/src/nearby_hotspot.rs | Android did not retain the exact hotspot interface binding |  |
| src-tauri/src/nearby_hotspot.rs | Invalid native hotspot IPv4 address |  |
| src-tauri/src/nearby_hotspot.rs | Invalid native hotspot IPv4 prefix |  |
| src-tauri/src/nearby_hotspot.rs | Invalid native hotspot interface index |  |
| src-tauri/src/nearby_hotspot.rs | Invalid native hotspot interface name |  |
| src-tauri/src/nearby_hotspot.rs | Invalid native hotspot interface receipt |  |
| src-tauri/src/nearby_hotspot.rs | Invalid native hotspot lease |  |
| src-tauri/src/nearby_hotspot.rs | The owned hotspot requires a UDP socket |  |
| src-tauri/src/nearby_hotspot.rs | The selected peer is not on the owned hotspot interface |  |
| src-tauri/src/nearby_hotspot.rs | address |  |
| src-tauri/src/nearby_hotspot.rs | lease |  |
| src-tauri/src/nearby_hotspot.rs | prefixLength |  |
| src-tauri/src/nearby_public_link.rs | Nearby connection information is too large |  |
| src-tauri/src/nearby_public_link.rs | The sender supplied an unsupported temporary Wi-Fi network |  |
| src-tauri/src/nearby_public_link.rs | The sender supplied invalid public Nearby connection information |  |
| src-tauri/src/nearby_route.rs | Automatic Nearby joining requires an on-link IPv4 endpoint |  |
| src-tauri/src/nearby_route.rs | Automatic hotspot connections require a supported mobile device |  |
| src-tauri/src/nearby_route.rs | Invalid native Nearby UDP route receipt |  |
| src-tauri/src/nearby_route.rs | Invalid native Nearby route receipt |  |
| src-tauri/src/nearby_route.rs | Native Nearby route adapter is unavailable |  |
| src-tauri/src/nearby_route.rs | Native mobile state and joined-route loss callbacks revoke the route |  |
| src-tauri/src/nearby_route.rs | Nearby requires an on-link private network endpoint |  |
| src-tauri/src/nearby_route.rs | Only Android can own a temporary hotspot route |  |
| src-tauri/src/nearby_route.rs | The Nearby UDP socket does not own its confirmed local address |  |
| src-tauri/src/nearby_route.rs | The joined Wi-Fi route has no confirmed local IPv4 address |  |
| src-tauri/src/nearby_route.rs | hotspotQuicState |  |
| src-tauri/src/nearby_route.rs | nearbyJoinState |  |
| src-tauri/src/nearby_service.rs | A Nearby connection is already in progress |  |
| src-tauri/src/nearby_service.rs | Android did not finish creating the temporary Wi-Fi network |  |
| src-tauri/src/nearby_service.rs | Android has no app-owned temporary hotspot |  |
| src-tauri/src/nearby_service.rs | Bluetooth connection ended |  |
| src-tauri/src/nearby_service.rs | Bluetooth discovery is unavailable |  |
| src-tauri/src/nearby_service.rs | Bluetooth discovery requires the iPhone, iPad or Android application. Use the same local network on this device. |  |
| src-tauri/src/nearby_service.rs | Bluetooth discovery stopped |  |
| src-tauri/src/nearby_service.rs | Bluetooth discovery stopped; the accepted library connection remains available. |  |
| src-tauri/src/nearby_service.rs | Bluetooth is unavailable. Check its permissions. |  |
| src-tauri/src/nearby_service.rs | Bluetooth packet is missing |  |
| src-tauri/src/nearby_service.rs | Choose and connect to the nearby sender first |  |
| src-tauri/src/nearby_service.rs | Choose and connect to this nearby sender first |  |
| src-tauri/src/nearby_service.rs | EXACT_INTERFACE_UNAVAILABLE |  |
| src-tauri/src/nearby_service.rs | Invalid Bluetooth packet budget |  |
| src-tauri/src/nearby_service.rs | Invalid Bluetooth packet encoding |  |
| src-tauri/src/nearby_service.rs | Invalid Bluetooth packet length or encoding |  |
| src-tauri/src/nearby_service.rs | Invalid hotspot interface lease |  |
| src-tauri/src/nearby_service.rs | Invalid public Nearby share identity |  |
| src-tauri/src/nearby_service.rs | Invalid sender temporary-network response |  |
| src-tauri/src/nearby_service.rs | Invalid temporary-network endpoint |  |
| src-tauri/src/nearby_service.rs | Library sharing stopped |  |
| src-tauri/src/nearby_service.rs | NEARBY_VISIBLE_ACTIVITY_REQUIRED |  |
| src-tauri/src/nearby_service.rs | Native Nearby listeners are not ready yet. Try again. |  |
| src-tauri/src/nearby_service.rs | Native adapter unavailable |  |
| src-tauri/src/nearby_service.rs | Nearby connection ended |  |
| src-tauri/src/nearby_service.rs | Nearby connection expired after a privacy transition |  |
| src-tauri/src/nearby_service.rs | Nearby connection stopped |  |
| src-tauri/src/nearby_service.rs | Nearby discovery stopped |  |
| src-tauri/src/nearby_service.rs | Nearby radio owner is already active |  |
| src-tauri/src/nearby_service.rs | Nearby sender is no longer available |  |
| src-tauri/src/nearby_service.rs | Nearby sender stopped |  |
| src-tauri/src/nearby_service.rs | Nearby sharing was retired | retired |
| src-tauri/src/nearby_service.rs | Nearby state unavailable |  |
| src-tauri/src/nearby_service.rs | Open Nearby to find the sender again |  |
| src-tauri/src/nearby_service.rs | The Nearby connection did not stop in time |  |
| src-tauri/src/nearby_service.rs | The Nearby connection ended. Connect again. |  |
| src-tauri/src/nearby_service.rs | The Nearby sender is not connected |  |
| src-tauri/src/nearby_service.rs | The nearby sender did not respond. Try again. |  |
| src-tauri/src/nearby_service.rs | The selected Nearby network connection ended. Connect again. |  |
| src-tauri/src/nearby_service.rs | The selected nearby sender is no longer available |  |
| src-tauri/src/nearby_service.rs | The selected sender disconnected before sending connection information |  |
| src-tauri/src/nearby_service.rs | The sender could not join the temporary Wi-Fi network: {error} |  |
| src-tauri/src/nearby_service.rs | The sender has no common local-network endpoint |  |
| src-tauri/src/nearby_service.rs | The sender is not reachable on the current Wi-Fi |  |
| src-tauri/src/nearby_service.rs | The sender's public library changed while connecting |  |
| src-tauri/src/nearby_service.rs | The temporary Wi-Fi connection ended while the sender was joining |  |
| src-tauri/src/nearby_service.rs | The temporary hotspot is not ready |  |
| src-tauri/src/nearby_service.rs | The temporary hotspot stopped |  |
| src-tauri/src/nearby_service.rs | The temporary-network response is too large |  |
| src-tauri/src/nearby_service.rs | This sender cannot join a receiver-hosted temporary network |  |
| src-tauri/src/nearby_service.rs | This sender uses an unsupported Nearby version |  |
| src-tauri/src/nearby_service.rs | Unknown Nearby command |  |
| src-tauri/src/nearby_service.rs | [nearby] connect exchange failed: {error} |  |
| src-tauri/src/nearby_service.rs | [nearby] direct route failed: {error} |  |
| src-tauri/src/nearby_service.rs | [nearby] hosting request failed: {error} |  |
| src-tauri/src/nearby_service.rs | [nearby] hosting request sent |  |
| src-tauri/src/nearby_service.rs | [nearby] publish failed: {error} |  |
| src-tauri/src/nearby_service.rs | [nearby] publish library gate acquired |  |
| src-tauri/src/nearby_service.rs | [nearby] reverse hotspot ready; sending join request |  |
| src-tauri/src/nearby_service.rs | [nearby] transient final route proof failed: {error} |  |
| src-tauri/src/nearby_service.rs | [nearby] transient outer route proof failed: {error} |  |
| src-tauri/src/nearby_service.rs | advertising |  |
| src-tauri/src/nearby_service.rs | connected |  |
| src-tauri/src/nearby_service.rs | connection |  |
| src-tauri/src/nearby_service.rs | done |  |
| src-tauri/src/nearby_service.rs | joining |  |
| src-tauri/src/nearby_service.rs | librarySessionId |  |
| src-tauri/src/nearby_service.rs | lost |  |
| src-tauri/src/nearby_service.rs | nearbyBluetoothDisconnect |  |
| src-tauri/src/nearby_service.rs | nearbyBluetoothStop |  |
| src-tauri/src/nearby_service.rs | nearbyJoin |  |
| src-tauri/src/nearby_service.rs | nearbyJoinCancel |  |
| src-tauri/src/nearby_service.rs | nearbyRetire |  |
| src-tauri/src/nearby_service.rs | nearby_discover |  |
| src-tauri/src/nearby_service.rs | owner |  |
| src-tauri/src/nearby_service.rs | packetBudget |  |
| src-tauri/src/nearby_service.rs | peer |  |
| src-tauri/src/nearby_service.rs | peerId |  |
| src-tauri/src/nearby_service.rs | publisherName |  |
| src-tauri/src/nearby_service.rs | reservationId |  |
| src-tauri/src/notifications.rs | Allocating native notification reply |  |
| src-tauri/src/notifications.rs | Invalid acquisition notification receipt |  |
| src-tauri/src/notifications.rs | Invalid native notification profile identity |  |
| src-tauri/src/notifications.rs | Invalid native notification reply: {error} |  |
| src-tauri/src/notifications.rs | Leave private or read-only mode before enabling notifications |  |
| src-tauri/src/notifications.rs | Missing native notification admission result |  |
| src-tauri/src/notifications.rs | Missing native notification reply |  |
| src-tauri/src/notifications.rs | Native notification authorization failed |  |
| src-tauri/src/notifications.rs | Native notification profile was not admitted |  |
| src-tauri/src/notifications.rs | Native notification request IDs exhausted |  |
| src-tauri/src/notifications.rs | Notification permission expired after a policy transition |  |
| src-tauri/src/notifications.rs | Notification policy revision exhausted |  |
| src-tauri/src/notifications.rs | Notification profile is unavailable |  |
| src-tauri/src/notifications.rs | Notifications are off in private or read-only mode |  |
| src-tauri/src/notifications.rs | ack |  |
| src-tauri/src/notifications.rs | command |  |
| src-tauri/src/notifications.rs | notice |  |
| src-tauri/src/notifications.rs | post |  |
| src-tauri/src/notifications.rs | receipt |  |
| src-tauri/src/notifications.rs | value |  |
| src-tauri/src/notifications_service.rs | A verified archive was added to your library. |  |
| src-tauri/src/notifications_service.rs | Archive ready |  |
| src-tauri/src/notifications_service.rs | Completion notifications are unavailable |  |
| src-tauri/src/notifications_service.rs | Native completion notifications are unavailable |  |
| src-tauri/src/notifications_service.rs | Notification permission expired before the app returned to the foreground |  |
| src-tauri/src/notifications_service.rs | {prefix}: {} |  |
| src-tauri/src/os_open.rs | The location is not valid UTF-8 |  |
| src-tauri/src/os_open.rs | macOS could not find an application to open it |  |
| src-tauri/src/os_open.rs | macOS rejected the link |  |
| src-tauri/src/partial_donor.rs | Partial download ended early |  |
| src-tauri/src/partial_donor.rs | Partial download has no filename |  |
| src-tauri/src/partial_donor.rs | Partial download has no parent directory |  |
| src-tauri/src/partial_donor.rs | Partial download is not a matching regular file |  |
| src-tauri/src/partial_donor.rs | Partial download offset overflow |  |
| src-tauri/src/pdf_export.rs | .pdf |  |
| src-tauri/src/pdf_export.rs | Android PDF export must use the native system print UI |  |
| src-tauri/src/pdf_export.rs | Article PDF export requires a native application window |  |
| src-tauri/src/pdf_export.rs | Article PDF export requires an HTML article |  |
| src-tauri/src/pdf_export.rs | Capture the current article before exporting a PDF |  |
| src-tauri/src/pdf_export.rs | Invalid PDF filename suggestion |  |
| src-tauri/src/pdf_export.rs | Kiwix could not capture the current article: {error} |  |
| src-tauri/src/pdf_export.rs | Kiwix could not share the article PDF. Try sharing it again |  |
| src-tauri/src/pdf_export.rs | Mobile PDF export is unavailable on this platform |  |
| src-tauri/src/pdf_export.rs | Native Android cache root is missing |  |
| src-tauri/src/pdf_export.rs | Native Android cache root must be an existing absolute directory |  |
| src-tauri/src/pdf_export.rs | Open the article in the main window to export a PDF |  |
| src-tauri/src/pdf_export.rs | PDF export stopped because the app is closing |  |
| src-tauri/src/pdf_export.rs | Resolving PDF temporary storage: {error} |  |
| src-tauri/src/pdf_export.rs | Save export |  |
| src-tauri/src/pdf_export.rs | Share |  |
| src-tauri/src/pdf_export.rs | The PDF capture identity is already in use |  |
| src-tauri/src/pdf_export.rs | The app window is unavailable. Reopen the app to continue |  |
| src-tauri/src/pdf_export.rs | The article was closed or changed. Open it again before exporting |  |
| src-tauri/src/pdf_export.rs | The live article changed during capture; export it again |  |
| src-tauri/src/pdf_export.rs | The live article reader has retired; capture it again | retired |
| src-tauri/src/pdf_export.rs | The native PDF export picker timed out |  |
| src-tauri/src/pdf_export.rs | The native PDF picker returned no completion state |  |
| src-tauri/src/pdf_export.rs | The native print provider did not complete its system print operation |  |
| src-tauri/src/pdf_export.rs | The window you started printing from has closed |  |
| src-tauri/src/pdf_export.rs | The window you started sharing from has closed |  |
| src-tauri/src/pdf_export.rs | The window you started this from has closed |  |
| src-tauri/src/pdf_export.rs | The window you started this from is out of date. Reopen the app and try again |  |
| src-tauri/src/pdf_export.rs | This window is out of date. Reopen the app to continue |  |
| src-tauri/src/pdf_export.rs | Unsafe PDF temporary storage directory |  |
| src-tauri/src/pdf_export.rs | Use the native mobile PDF export action |  |
| src-tauri/src/pdf_export.rs | Your privacy settings changed during PDF export. Start the export again |  |
| src-tauri/src/pdf_export.rs | application/pdf |  |
| src-tauri/src/pdf_export.rs | liveSnapshot |  |
| src-tauri/src/pdf_export.rs | mobile_save_pdf |  |
| src-tauri/src/pdf_export.rs | pdfAuthority |  |
| src-tauri/src/pdf_export.rs | print_pdf |  |
| src-tauri/src/pdf_export.rs | save_pdf |  |
| src-tauri/src/pdf_export.rs | share_pdf |  |
| src-tauri/src/pdf_export.rs | window- |  |
| src-tauri/src/pdf_export.rs | windows |  |
| src-tauri/src/permissions.rs | Archive sensor session is no longer active |  |
| src-tauri/src/permissions.rs | Duplicate location request |  |
| src-tauri/src/permissions.rs | Location consent was revoked |  |
| src-tauri/src/permissions.rs | Location is unavailable on this platform |  |
| src-tauri/src/permissions.rs | Location permission has not been granted for this archive |  |
| src-tauri/src/permissions.rs | Location request identifiers exhausted |  |
| src-tauri/src/permissions.rs | Native location provider stopped |  |
| src-tauri/src/permissions.rs | Reset this sensor's denied decision before asking again |  |
| src-tauri/src/permissions.rs | Retained sensor session is no longer owned by this reader |  |
| src-tauri/src/permissions.rs | Sensor state unavailable |  |
| src-tauri/src/permissions.rs | This archive cannot use device sensors in this window. Open the article again |  |
| src-tauri/src/permissions.rs | Too many pending location requests |  |
| src-tauri/src/photo_capture.rs | A file dialog is already open |  |
| src-tauri/src/photo_capture.rs | Camera unavailable |  |
| src-tauri/src/photo_capture.rs | Missing photo |  |
| src-tauri/src/photo_capture.rs | Missing photo request |  |
| src-tauri/src/photo_capture.rs | No application window is available |  |
| src-tauri/src/photo_capture.rs | That file is not a supported image |  |
| src-tauri/src/photo_capture.rs | That image is larger than 25 MB |  |
| src-tauri/src/photo_capture.rs | That is not a file |  |
| src-tauri/src/photo_capture.rs | The camera is already open |  |
| src-tauri/src/photo_capture.rs | The camera is closing; try again in a moment |  |
| src-tauri/src/photo_capture.rs | The camera is in use in another Kiwix window |  |
| src-tauri/src/photo_capture.rs | The camera window could not open: {error} |  |
| src-tauri/src/photo_capture.rs | The photo is damaged |  |
| src-tauri/src/photo_capture.rs | The photo is too large |  |
| src-tauri/src/photo_capture.rs | The reader window is gone |  |
| src-tauri/src/photo_capture.rs | This camera session has ended |  |
| src-tauri/src/photo_capture.rs | This page asks for a file type Kiwix cannot pick as a photo |  |
| src-tauri/src/photo_capture.rs | This page does not accept a camera photo |  |
| src-tauri/src/photo_capture.rs | This page does not accept that kind of image |  |
| src-tauri/src/photo_capture.rs | [year][month][day]-[hour][minute][second] |  |
| src-tauri/src/photo_capture.rs | accept |  |
| src-tauri/src/photo_capture.rs | capture |  |
| src-tauri/src/photo_capture.rs | checked above |  |
| src-tauri/src/photo_capture.rs | data |  |
| src-tauri/src/photo_capture.rs | environment |  |
| src-tauri/src/photo_capture.rs | image/* |  |
| src-tauri/src/photo_capture.rs | image/png |  |
| src-tauri/src/photo_capture.rs | jpg |  |
| src-tauri/src/photo_capture.rs | photo |  |
| src-tauri/src/photo_capture.rs | png |  |
| src-tauri/src/photo_capture.rs | status |  |
| src-tauri/src/photo_capture.rs | user |  |
| src-tauri/src/portable.rs | .portable-roots-{}.tmp |  |
| src-tauri/src/portable.rs | / |  |
| src-tauri/src/portable.rs | A saved file changed while Kiwix was reading it. Restart the app |  |
| src-tauri/src/portable.rs | A saved file in the portable folder is not valid. Replace it from a backup |  |
| src-tauri/src/portable.rs | A saved file is larger than expected. Restart the app |  |
| src-tauri/src/portable.rs | APPIMAGE has no distribution directory |  |
| src-tauri/src/portable.rs | APPIMAGE must name the absolute regular, non-symlink distributed AppImage |  |
| src-tauri/src/portable.rs | App bundle has no distribution directory |  |
| src-tauri/src/portable.rs | Archive path is not Unicode |  |
| src-tauri/src/portable.rs | Archive path is not a string |  |
| src-tauri/src/portable.rs | Cannot create portable data directory: {error} |  |
| src-tauri/src/portable.rs | Cannot inspect portable marker: {error} |  |
| src-tauri/src/portable.rs | Cannot open portable data directory: {error} |  |
| src-tauri/src/portable.rs | Conflicting portable torrent receipt correlations |  |
| src-tauri/src/portable.rs | Consumed acquisition receipt is not a string |  |
| src-tauri/src/portable.rs | Contents |  |
| src-tauri/src/portable.rs | Edition acquisition receipt is not a string |  |
| src-tauri/src/portable.rs | Executable has no distribution directory |  |
| src-tauri/src/portable.rs | Invalid portable webview label |  |
| src-tauri/src/portable.rs | Invalid torrent job ID |  |
| src-tauri/src/portable.rs | MacOS |  |
| src-tauri/src/portable.rs | Portable app-owned directory {name} must be a real directory |  |
| src-tauri/src/portable.rs | Portable app-owned file {name} must be a regular non-symlink file |  |
| src-tauri/src/portable.rs | Portable archive pathname cannot be represented on Windows |  |
| src-tauri/src/portable.rs | Portable archive pathname contains an invalid native component |  |
| src-tauri/src/portable.rs | Portable automatic directory {root}/{child} must be a real directory |  |
| src-tauri/src/portable.rs | Portable data has no parent |  |
| src-tauri/src/portable.rs | Portable data must be a real directory, not a symbolic link |  |
| src-tauri/src/portable.rs | Portable data must be a real non-symlink directory |  |
| src-tauri/src/portable.rs | Portable directory must be valid Unicode |  |
| src-tauri/src/portable.rs | Portable directory must not be a symbolic link: {} |  |
| src-tauri/src/portable.rs | Portable filesystem path contains NUL |  |
| src-tauri/src/portable.rs | Portable filesystem path contains traversal |  |
| src-tauri/src/portable.rs | Portable marker .portable must be a regular file, not a directory or symbolic link |  |
| src-tauri/src/portable.rs | Portable marker changed while opening |  |
| src-tauri/src/portable.rs | Portable path is not Unicode |  |
| src-tauri/src/portable.rs | Portable path is not a normalized relative pathname |  |
| src-tauri/src/portable.rs | Portable root history contains an invalid absolute path |  |
| src-tauri/src/portable.rs | Portable root history exceeds its safety size limit |  |
| src-tauri/src/portable.rs | Portable root history has reached its safety limit |  |
| src-tauri/src/portable.rs | Portable torrent completed path is outside portable data |  |
| src-tauri/src/portable.rs | Portable torrent root must be a real directory |  |
| src-tauri/src/portable.rs | Portable webview profile must be a real directory |  |
| src-tauri/src/portable.rs | Portable webview root must be a real directory |  |
| src-tauri/src/portable.rs | Saved automatic job points outside portable data: {original} |  |
| src-tauri/src/portable.rs | Torrent job has no completed paths |  |
| src-tauri/src/portable.rs | Unsupported or invalid portable root history |  |
| src-tauri/src/portable.rs | jobs.json |  |
| src-tauri/src/portable.rs | portableComponents |  |
| src-tauri/src/power_activity.rs | held |  |
| src-tauri/src/power_activity.rs | wifiPerformance |  |
| src-tauri/src/private_sync.rs | Disconnect an unused private device before connecting another |  |
| src-tauri/src/private_sync.rs | Invalid connected private device |  |
| src-tauri/src/private_sync.rs | Invalid private device identity |  |
| src-tauri/src/private_sync.rs | Invalid private encrypted frame |  |
| src-tauri/src/private_sync.rs | Invalid private encrypted frame length |  |
| src-tauri/src/private_sync.rs | Invalid private message framing |  |
| src-tauri/src/private_sync.rs | Invalid private reading-data message |  |
| src-tauri/src/private_sync.rs | Private device did not authenticate an identity |  |
| src-tauri/src/private_sync.rs | Private handshake failed |  |
| src-tauri/src/private_sync.rs | Private message exceeds the supported transfer bound |  |
| src-tauri/src/private_sync.rs | Private reading data exceeds the supported transfer bound. Remove unused connected devices or reduce the reading list before syncing |  |
| src-tauri/src/private_sync.rs | The nearby device identity does not match the scanned invitation |  |
| src-tauri/src/private_sync.rs | Too many connected private devices |  |
| src-tauri/src/private_sync.rs | Unexpected private device identity |  |
| src-tauri/src/private_sync_identity.rs | Noise generated an invalid secret key length |  |
| src-tauri/src/private_sync_identity.rs | X25519 is unavailable |  |
| src-tauri/src/private_sync_identity.rs | concurrent identity creation disappeared |  |
| src-tauri/src/private_sync_identity.rs | identity changed while opening |  |
| src-tauri/src/private_sync_identity.rs | identity changed while reading |  |
| src-tauri/src/private_sync_identity.rs | identity directory must be owned by this user with permissions 0700 |  |
| src-tauri/src/private_sync_identity.rs | identity file must be owned by this user with permissions 0600 |  |
| src-tauri/src/private_sync_identity.rs | identity grew while reading |  |
| src-tauri/src/private_sync_identity.rs | identity must be a regular, single-link, exactly 64-byte file |  |
| src-tauri/src/private_sync_identity.rs | identity public key does not match its secret |  |
| src-tauri/src/private_sync_identity.rs | key file was substituted or modified |  |
| src-tauri/src/private_sync_identity.rs | profile directory was substituted |  |
| src-tauri/src/private_sync_identity.rs | profile must be owned by this user and not writable by other users |  |
| src-tauri/src/private_sync_identity.rs | this platform has no supported private-file permission implementation |  |
| src-tauri/src/private_sync_service.rs | Add this archive to your local library before resuming this reading position |  |
| src-tauri/src/private_sync_service.rs | Choose reading data to sync |  |
| src-tauri/src/private_sync_service.rs | Choose whether to enable private sync |  |
| src-tauri/src/private_sync_service.rs | Confirm that all connected devices must reconnect before resetting sync history |  |
| src-tauri/src/private_sync_service.rs | Connect both devices to the same Wi-Fi or Ethernet network before using private Nearby sync |  |
| src-tauri/src/private_sync_service.rs | Connected device no longer exists |  |
| src-tauri/src/private_sync_service.rs | Connection invitation is too large |  |
| src-tauri/src/private_sync_service.rs | Device name is too long or empty |  |
| src-tauri/src/private_sync_service.rs | Enable private reading sync first |  |
| src-tauri/src/private_sync_service.rs | Invalid conflict alternatives |  |
| src-tauri/src/private_sync_service.rs | Invalid invitation expiry |  |
| src-tauri/src/private_sync_service.rs | Invalid or already used private invitation |  |
| src-tauri/src/private_sync_service.rs | Invalid private connection invitation |  |
| src-tauri/src/private_sync_service.rs | Invalid private device confirmation |  |
| src-tauri/src/private_sync_service.rs | Invalid private device greeting |  |
| src-tauri/src/private_sync_service.rs | Invalid private sync categories |  |
| src-tauri/src/private_sync_service.rs | Invitation expired |  |
| src-tauri/src/private_sync_service.rs | No active private connection invitation |  |
| src-tauri/src/private_sync_service.rs | Private device did not acknowledge durable sync |  |
| src-tauri/src/private_sync_service.rs | Private device did not finish confirmation |  |
| src-tauri/src/private_sync_service.rs | Private device did not send reading data |  |
| src-tauri/src/private_sync_service.rs | Private device sent an unexpected response |  |
| src-tauri/src/private_sync_service.rs | Private device was revoked |  |
| src-tauri/src/private_sync_service.rs | Private invitation expired |  |
| src-tauri/src/private_sync_service.rs | Private invitation expired or was cancelled |  |
| src-tauri/src/private_sync_service.rs | Private invitation was cancelled |  |
| src-tauri/src/private_sync_service.rs | Private pairing unavailable |  |
| src-tauri/src/private_sync_service.rs | Private pairing was cancelled |  |
| src-tauri/src/private_sync_service.rs | Private reading sync is disabled |  |
| src-tauri/src/private_sync_service.rs | Private sync admission was retired. Return to the app and try again | retired |
| src-tauri/src/private_sync_service.rs | Private sync is paused |  |
| src-tauri/src/private_sync_service.rs | Private sync is paused in private or read-only mode |  |
| src-tauri/src/private_sync_service.rs | Private sync is paused. Return to the app and try again |  |
| src-tauri/src/private_sync_service.rs | Private sync is unavailable in private or read-only mode |  |
| src-tauri/src/private_sync_service.rs | Private sync startup was cancelled |  |
| src-tauri/src/private_sync_service.rs | Private sync status unavailable |  |
| src-tauri/src/private_sync_service.rs | Private sync timed out. Keep both apps open on the same network |  |
| src-tauri/src/private_sync_service.rs | Private sync was cancelled |  |
| src-tauri/src/private_sync_service.rs | Return to the app before syncing private reading data |  |
| src-tauri/src/private_sync_service.rs | Review the conflict alternatives before resolving |  |
| src-tauri/src/private_sync_service.rs | The connected device is unavailable. Keep both apps open on the same Wi-Fi network; create a new invitation if its network address changed |  |
| src-tauri/src/private_sync_service.rs | This conflict changed. Review every current alternative before resolving |  |
| src-tauri/src/private_sync_service.rs | This conflict changed. Review its current alternatives before resolving |  |
| src-tauri/src/private_sync_service.rs | This connection invitation has expired. Create a new invitation |  |
| src-tauri/src/private_sync_service.rs | This invitation is invalid or expired. Create a new invitation on the other device |  |
| src-tauri/src/private_sync_service.rs | This private device is disabled or no longer connected |  |
| src-tauri/src/private_sync_service.rs | This private device is not connected or was revoked |  |
| src-tauri/src/private_sync_service.rs | This reading position is no longer available |  |
| src-tauri/src/private_sync_service.rs | Unknown private reading protocol |  |
| src-tauri/src/private_sync_service.rs | Unknown private sync command: {command} |  |
| src-tauri/src/private_sync_service.rs | Verify the current comparison code on both devices before connecting |  |
| src-tauri/src/private_sync_service.rs | alternativeIds |  |
| src-tauri/src/private_sync_service.rs | categories |  |
| src-tauri/src/private_sync_service.rs | confirmed |  |
| src-tauri/src/private_sync_service.rs | pair |  |
| src-tauri/src/private_sync_service.rs | peerIdentity |  |
| src-tauri/src/private_sync_service.rs | private_sync_clear_positions |  |
| src-tauri/src/private_sync_service.rs | private_sync_configure |  |
| src-tauri/src/private_sync_service.rs | private_sync_pair_cancel |  |
| src-tauri/src/private_sync_service.rs | private_sync_pair_confirm |  |
| src-tauri/src/private_sync_service.rs | private_sync_status |  |
| src-tauri/src/private_sync_service.rs | scroll |  |
| src-tauri/src/private_sync_service.rs | sync |  |
| src-tauri/src/private_sync_service.rs | waiting |  |
| src-tauri/src/profile_paths.rs | A saved map download could not be read. Restart the app |  |
| src-tauri/src/profile_paths.rs | Download state is not an object |  |
| src-tauri/src/profile_paths.rs | LocalSwarm state is not an object |  |
| src-tauri/src/profile_paths.rs | Persisted path list is not an array |  |
| src-tauri/src/profile_paths.rs | Retained map publications are not an array |  |
| src-tauri/src/profile_paths.rs | Saved edition records are not an array | record |
| src-tauri/src/profile_paths.rs | destination |  |
| src-tauri/src/profile_paths.rs | directory |  |
| src-tauri/src/profile_paths.rs | editionIntents |  |
| src-tauri/src/profile_paths.rs | metadata |  |
| src-tauri/src/profile_paths.rs | owned |  |
| src-tauri/src/profile_paths.rs | parent |  |
| src-tauri/src/profile_paths.rs | published |  |
| src-tauri/src/profile_paths.rs | record |  |
| src-tauri/src/profile_paths.rs | staging |  |
| src-tauri/src/provider_reference.rs | External archive parts are missing, duplicated, or out of order |  |
| src-tauri/src/provider_reference.rs | External archive references cannot be managed filesystem paths |  |
| src-tauri/src/provider_reference.rs | Invalid external archive display name |  |
| src-tauri/src/provider_reference.rs | Invalid external archive part count |  |
| src-tauri/src/provider_reference.rs | Kiwix cannot reopen this file from the other app. Choose it again |  |
| src-tauri/src/provider_reference.rs | Native provider descriptor invalid |  |
| src-tauri/src/provider_reference.rs | Native provider files missing |  |
| src-tauri/src/provider_reference.rs | Native provider lease invalid |  |
| src-tauri/src/provider_reference.rs | Native provider lease missing |  |
| src-tauri/src/provider_reference.rs | Native provider move receipt invalid |  |
| src-tauri/src/provider_reference.rs | Native provider move receipt missing |  |
| src-tauri/src/provider_reference.rs | Native provider resolver is not initialized |  |
| src-tauri/src/provider_reference.rs | Native provider returned an invalid acquisition error |  |
| src-tauri/src/provider_reference.rs | PROVIDER_DOCUMENT_UNAVAILABLE_USE_COPY_IMPORT |  |
| src-tauri/src/provider_reference.rs | PROVIDER_LEASE_CONFLICT |  |
| src-tauri/src/provider_reference.rs | Provider deletion was not confirmed; the managed copy and recovery journal were preserved |  |
| src-tauri/src/provider_reference.rs | Split external archives require an explicitly selected same-folder capability |  |
| src-tauri/src/provider_reference.rs | The managed copy is ready, but the provider original was preserved: {error} |  |
| src-tauri/src/provider_reference.rs | The provider has not authorized moving this archive: {error}. Choose Copy to keep the original. |  |
| src-tauri/src/provider_reference.rs | This provider does not offer a seekable regular file. Choose Import archives to make a managed copy. |  |
| src-tauri/src/provider_reference.rs | code |  |
| src-tauri/src/provider_reference.rs | deleteExternalArchiveMove |  |
| src-tauri/src/provider_reference.rs | deleted |  |
| src-tauri/src/provider_reference.rs | files |  |
| src-tauri/src/provider_reference.rs | message |  |
| src-tauri/src/provider_reference.rs | parts |  |
| src-tauri/src/provider_reference.rs | prepareExternalArchiveMove |  |
| src-tauri/src/provider_reference.rs | reference |  |
| src-tauri/src/provider_reference.rs | retry |  |
| src-tauri/src/provider_reference.rs | sourceReference |  |
| src-tauri/src/publication.rs | An entry that replaced the moved file's old name was preserved as {private}: {error} |  |
| src-tauri/src/publication.rs | Confined publication requires a single filename |  |
| src-tauri/src/publication.rs | Duplicate directory publication child |  |
| src-tauri/src/publication.rs | Invalid confined publication filename |  |
| src-tauri/src/publication.rs | Kiwix cannot rename a file on this storage without the risk of overwriting another one: it has no exclusive rename, and a hard link could not be created ({error}). Nothing was changed. Keep archives that Kiwix should move or delete on a different drive |  |
| src-tauri/src/publication.rs | Publication path has no filename |  |
| src-tauri/src/publication.rs | Publication source must be a regular file |  |
| src-tauri/src/publication.rs | The file changed while it was being moved; both names were kept |  |
| src-tauri/src/publication.rs | The file was published under its new name but its old name could not be removed: {error} |  |
| src-tauri/src/publication.rs | Verified publication requires a regular file of the expected size without hard links |  |
| src-tauri/src/publication.rs | Verified publication requires sibling filenames |  |
| src-tauri/src/publication.rs | {private} is in use by another file; the old name was kept |  |
| src-tauri/src/quit.rs | , |  |
| src-tauri/src/quit.rs | Application quit is already pending |  |
| src-tauri/src/quit.rs | Cannot open a window while application Quit is pending |  |
| src-tauri/src/quit.rs | Cannot request session flush for {}: {error} |  |
| src-tauri/src/quit.rs | Library service unavailable during Quit |  |
| src-tauri/src/quit.rs | No application quit is pending |  |
| src-tauri/src/quit.rs | Quit cancelled: session flush timed out for {}. Retry Quit or close the affected window explicitly. |  |
| src-tauri/src/quit.rs | Quit flush is restricted to application windows |  |
| src-tauri/src/quit.rs | Quit flush is restricted to the trusted application origin |  |
| src-tauri/src/quit.rs | Quit manager unavailable |  |
| src-tauri/src/quit.rs | Quit request was cancelled |  |
| src-tauri/src/quit.rs | Quit state unavailable |  |
| src-tauri/src/quit.rs | Unknown, stale, or already acknowledged quit request |  |
| src-tauri/src/quit.rs | kiwix-quit-cancelled |  |
| src-tauri/src/quit.rs | kiwix-quit-flush |  |
| src-tauri/src/reading_data.rs | Reading sync bookmark does not match its canonical article identity |  |
| src-tauri/src/reading_data.rs | Reading sync comment exceeds its allowed bounds |  |
| src-tauri/src/reading_data.rs | Reading sync contains an empty bookmark generation set | generation |
| src-tauri/src/reading_data.rs | Reading sync contains an empty causal register |  |
| src-tauri/src/reading_data.rs | Reading sync contains an empty highlight generation set | generation |
| src-tauri/src/reading_data.rs | Reading sync contains an empty identifier |  |
| src-tauri/src/reading_data.rs | Reading sync contains an inconsistent article identity |  |
| src-tauri/src/reading_data.rs | Reading sync contains an inconsistent causal event |  |
| src-tauri/src/reading_data.rs | Reading sync contains an invalid canonical bookmark identity |  |
| src-tauri/src/reading_data.rs | Reading sync contains an invalid causal counter |  |
| src-tauri/src/reading_data.rs | Reading sync contains an invalid causal event |  |
| src-tauri/src/reading_data.rs | Reading sync contains an invalid generation | generation |
| src-tauri/src/reading_data.rs | Reading sync contains an invalid highlight identity |  |
| src-tauri/src/reading_data.rs | Reading sync contains an invalid replica identity |  |
| src-tauri/src/reading_data.rs | Reading sync contains duplicate local bookmark IDs |  |
| src-tauri/src/reading_data.rs | Reading sync contains invalid or oversized text |  |
| src-tauri/src/reading_data.rs | Reading sync generation has no causal creation event | generation |
| src-tauri/src/reading_data.rs | Reading sync has inconsistent local bookmark identities |  |
| src-tauri/src/reading_data.rs | Reading sync has no local bookmark projection identity |  |
| src-tauri/src/reading_data.rs | Reading sync has reached its reset safety limit. Use a new profile; retired identities were not discarded | retired |
| src-tauri/src/reading_data.rs | Reading sync has too many tags |  |
| src-tauri/src/reading_data.rs | Reading sync locations must be archive-relative references |  |
| src-tauri/src/reading_data.rs | Reading sync position has the wrong replica owner |  |
| src-tauri/src/reading_data.rs | Reading sync replica counter rolled back; rebootstrap this device |  |
| src-tauri/src/reading_data.rs | Reading sync requires finite nonnegative scroll positions |  |
| src-tauri/src/reading_data.rs | Resolve all reading conflicts before rebootstrap so no alternatives are discarded |  |
| src-tauri/src/reading_data.rs | The reading conflict no longer exists |  |
| src-tauri/src/reading_data.rs | The selected conflict alternative no longer exists |  |
| src-tauri/src/reading_data.rs | The selected recreation no longer exists |  |
| src-tauri/src/reading_data.rs | Unsupported reading sync snapshot version | snapshot |
| src-tauri/src/sandbox.rs | The bookmark does not name a file |  |
| src-tauri/src/sandbox.rs | The path is not valid UTF-8 |  |
| src-tauri/src/sandbox.rs | The saved bookmark is corrupt |  |
| src-tauri/src/sandbox.rs | The system refused access |  |
| src-tauri/src/search.rs | Search was cancelled |  |
| src-tauri/src/search.rs | Unknown search mode |  |
| src-tauri/src/search.rs | hasMore |  |
| src-tauri/src/search.rs | hits |  |
| src-tauri/src/server.rs | /.kiwix/{}/ |  |
| src-tauri/src/server.rs | /api |  |
| src-tauri/src/server.rs | Application asset not found |  |
| src-tauri/src/server.rs | Archive URL is not valid UTF-8 |  |
| src-tauri/src/server.rs | Archive location contains a malformed percent escape |  |
| src-tauri/src/server.rs | Archive resource plan was not readable |  |
| src-tauri/src/server.rs | Archive resources are read-only |  |
| src-tauri/src/server.rs | Archive storage quota exceeded |  |
| src-tauri/src/server.rs | Archive worker failed: {error} |  |
| src-tauri/src/server.rs | Archive worker stopped |  |
| src-tauri/src/server.rs | Archive-controlled service workers are not supported |  |
| src-tauri/src/server.rs | Archives are read-only |  |
| src-tauri/src/server.rs | Cookie identity mismatch |  |
| src-tauri/src/server.rs | Could not prepare the archive response |  |
| src-tauri/src/server.rs | Empty suffix |  |
| src-tauri/src/server.rs | Foreign archive origin |  |
| src-tauri/src/server.rs | Foreign preview origin |  |
| src-tauri/src/server.rs | HTML documents must use the instrumented reader |  |
| src-tauri/src/server.rs | Invalid command JSON |  |
| src-tauri/src/server.rs | Invalid cookie |  |
| src-tauri/src/server.rs | Invalid cookie expiry |  |
| src-tauri/src/server.rs | Invalid cookie fields |  |
| src-tauri/src/server.rs | Invalid cookie name |  |
| src-tauri/src/server.rs | Invalid cookie path |  |
| src-tauri/src/server.rs | Invalid cookie value |  |
| src-tauri/src/server.rs | Invalid local storage value |  |
| src-tauri/src/server.rs | Invalid range |  |
| src-tauri/src/server.rs | Invalid resource path |  |
| src-tauri/src/server.rs | Invalid saved storage |  |
| src-tauri/src/server.rs | Invalid storage key |  |
| src-tauri/src/server.rs | Invalid storage mutation |  |
| src-tauri/src/server.rs | Invalid storage mutation batch |  |
| src-tauri/src/server.rs | Invalid storage mutation batch fields |  |
| src-tauri/src/server.rs | Invalid storage mutation fields |  |
| src-tauri/src/server.rs | Invalid storage mutation sequence |  |
| src-tauri/src/server.rs | Invalid storage mutations |  |
| src-tauri/src/server.rs | Invalid storage {field} |  |
| src-tauri/src/server.rs | Invalid suffix |  |
| src-tauri/src/server.rs | Inverted range |  |
| src-tauri/src/server.rs | Missing command |  |
| src-tauri/src/server.rs | Missing cookie |  |
| src-tauri/src/server.rs | Offline archive server panicked: {message} |  |
| src-tauri/src/server.rs | Offline archive shutdown task failed: {error} |  |
| src-tauri/src/server.rs | Only one nonempty byte range is supported |  |
| src-tauri/src/server.rs | Preview authorization required |  |
| src-tauri/src/server.rs | Preview origin required |  |
| src-tauri/src/server.rs | Range outside resource |  |
| src-tauri/src/server.rs | Read-only application assets |  |
| src-tauri/src/server.rs | Reader origin unavailable |  |
| src-tauri/src/server.rs | Reader privacy session expired |  |
| src-tauri/src/server.rs | Reader storage is read-only |  |
| src-tauri/src/server.rs | Reading stopped. Reload the article |  |
| src-tauri/src/server.rs | Shell service workers are not supported |  |
| src-tauri/src/server.rs | Storage mutation batch is too large |  |
| src-tauri/src/server.rs | Storage mutation queue unavailable |  |
| src-tauri/src/server.rs | Storage mutation rejected |  |
| src-tauri/src/server.rs | Storage mutation sequence is out of order |  |
| src-tauri/src/server.rs | This link can only be opened, not changed |  |
| src-tauri/src/server.rs | This link is out of date. Reload the article |  |
| src-tauri/src/server.rs | This link must be opened inside Kiwix |  |
| src-tauri/src/server.rs | Truncated archive document |  |
| src-tauri/src/server.rs | Truncated archive resource |  |
| src-tauri/src/server.rs | Unknown archive host |  |
| src-tauri/src/server.rs | Unknown or exhausted storage document stream |  |
| src-tauri/src/server.rs | Unknown preview host |  |
| src-tauri/src/server.rs | Unknown reader resource |  |
| src-tauri/src/server.rs | Unknown storage mutation |  |
| src-tauri/src/server.rs | Unsupported range unit |  |
| src-tauri/src/server.rs | Untrusted reader parent origin |  |
| src-tauri/src/server.rs | authorization |  |
| src-tauri/src/server.rs | cookie |  |
| src-tauri/src/server.rs | cookie-remove |  |
| src-tauri/src/server.rs | cookie-set |  |
| src-tauri/src/server.rs | cookies |  |
| src-tauri/src/server.rs | documentId |  |
| src-tauri/src/server.rs | expires |  |
| src-tauri/src/server.rs | host |  |
| src-tauri/src/server.rs | key |  |
| src-tauri/src/server.rs | local-clear |  |
| src-tauri/src/server.rs | local-remove |  |
| src-tauri/src/server.rs | local-set |  |
| src-tauri/src/server.rs | localStorage |  |
| src-tauri/src/server.rs | operations |  |
| src-tauri/src/server.rs | read_article |  |
| src-tauri/src/server.rs | read_resource |  |
| src-tauri/src/server.rs | service-worker |  |
| src-tauri/src/server.rs | {failure}; {workers} |  |
| src-tauri/src/server.rs | {path}\n{name} |  |
| src-tauri/src/server.rs | {server}; {workers} |  |
| src-tauri/src/service.rs | A download must have exactly one verified archive |  |
| src-tauri/src/service.rs | A map receipt must contain exactly one verified archive |  |
| src-tauri/src/service.rs | An acquisition must identify exactly one archive for verification |  |
| src-tauri/src/service.rs | Android window retirement failed: {error} |  |
| src-tauri/src/service.rs | Another copy of this archive is already in your library. Open that copy, or remove it before adding this file |  |
| src-tauri/src/service.rs | Archive file identity changed; import it separately or locate the original archive |  |
| src-tauri/src/service.rs | Archive installation changed while opening the reader; open it again |  |
| src-tauri/src/service.rs | Archive installation changed while relinking |  |
| src-tauri/src/service.rs | Archive is not in the library |  |
| src-tauri/src/service.rs | Archive “{}” is unavailable; locate it in the library before opening this link |  |
| src-tauri/src/service.rs | Archives are being moved; try again when the move finishes |  |
| src-tauri/src/service.rs | Download ownership is unavailable; reopen Downloads and retry |  |
| src-tauri/src/service.rs | Download verification omitted its original publication authority | authority |
| src-tauri/src/service.rs | Enable completion notifications through the explicit permission action |  |
| src-tauri/src/service.rs | External URL scheme or embedded credentials are not permitted |  |
| src-tauri/src/service.rs | External links are disabled in Settings. This address has not been opened. |  |
| src-tauri/src/service.rs | Finish recovering the pending storage move before changing the download folder |  |
| src-tauri/src/service.rs | Import manager unavailable |  |
| src-tauri/src/service.rs | Imported archive disappeared |  |
| src-tauri/src/service.rs | Invalid LocalSwarm port |  |
| src-tauri/src/service.rs | Invalid saved transfer state: {e} |  |
| src-tauri/src/service.rs | Invalid torrent publication receipt |  |
| src-tauri/src/service.rs | Leave read-only mode before changing settings |  |
| src-tauri/src/service.rs | Map choice changed while waiting for its installation gate; retry verification with the current explicit choice |  |
| src-tauri/src/service.rs | Missing search request identity |  |
| src-tauri/src/service.rs | Missing settings object |  |
| src-tauri/src/service.rs | Missing string argument: {key} |  |
| src-tauri/src/service.rs | Nearby download ownership is unavailable; retry the import |  |
| src-tauri/src/service.rs | Nearby sharing stopped. Turn it on again |  |
| src-tauri/src/service.rs | Nearby sharing was stopped. Select the share or resume the download again. |  |
| src-tauri/src/service.rs | No archive in the library has ID {}; import it before opening this link |  |
| src-tauri/src/service.rs | Open Kiwix from the app window to continue |  |
| src-tauri/src/service.rs | Open the archive before changing restricted mode |  |
| src-tauri/src/service.rs | Opening an external URL requires explicit confirmation |  |
| src-tauri/src/service.rs | Pausing downloads: {error} |  |
| src-tauri/src/service.rs | Pausing maps: {error} |  |
| src-tauri/src/service.rs | Pinned archives require the dedicated pin action |  |
| src-tauri/src/service.rs | Reader preferences expired after a privacy transition |  |
| src-tauri/src/service.rs | Releasing session archive providers: {error} |  |
| src-tauri/src/service.rs | Reopen the app to continue |  |
| src-tauri/src/service.rs | Return to the foreground before starting a LocalSwarm operation |  |
| src-tauri/src/service.rs | Saved transfer state exceeds 16 MiB |  |
| src-tauri/src/service.rs | Sensor manager unavailable |  |
| src-tauri/src/service.rs | Settings are not an object |  |
| src-tauri/src/service.rs | Sharing over a Wi-Fi hotspot is only available on Android |  |
| src-tauri/src/service.rs | Speech preferences require a language-level edit |  |
| src-tauri/src/service.rs | Stop sharing this archive before relinking it |  |
| src-tauri/src/service.rs | Stopping LocalSwarm: {error} |  |
| src-tauri/src/service.rs | Stopping archive imports: {error} |  |
| src-tauri/src/service.rs | Stopping archive readers: {error} |  |
| src-tauri/src/service.rs | Stopping archive sharing: {error} |  |
| src-tauri/src/service.rs | Stopping archive verification: {error} |  |
| src-tauri/src/service.rs | Stopping native transfer service: {error} |  |
| src-tauri/src/service.rs | Stopping spelling dictionary: {error} |  |
| src-tauri/src/service.rs | Stopping torrents: {error} |  |
| src-tauri/src/service.rs | The application is in strict read-only mode |  |
| src-tauri/src/service.rs | The application is shutting down |  |
| src-tauri/src/service.rs | The archive installation changed or is unavailable; open it again |  |
| src-tauri/src/service.rs | The previous Android window has not drained |  |
| src-tauri/src/service.rs | The selected LocalSwarm peer is no longer discovered; start discovery and choose it again |  |
| src-tauri/src/service.rs | This application window has closed |  |
| src-tauri/src/service.rs | This file belongs to an archive edition decision. Review it in Downloads instead of importing it separately |  |
| src-tauri/src/service.rs | This file is outside Kiwix's library storage. Import it into your library, then open it |  |
| src-tauri/src/service.rs | This operation is disabled in private mode |  |
| src-tauri/src/service.rs | This profile was opened with --read-only; restart without that flag to change settings |  |
| src-tauri/src/service.rs | This saved transfer item cannot be read. Restart the app, then try again |  |
| src-tauri/src/service.rs | This saved window is out of date. Close it and open a new window |  |
| src-tauri/src/service.rs | This search is no longer available. Start a new search |  |
| src-tauri/src/service.rs | This window is out of date. Close it and open a new window |  |
| src-tauri/src/service.rs | This window's session has expired. Reopen the window to continue |  |
| src-tauri/src/service.rs | Too many search consumers |  |
| src-tauri/src/service.rs | Too many search windows |  |
| src-tauri/src/service.rs | Too many zim:// links in one request |  |
| src-tauri/src/service.rs | Unknown archive |  |
| src-tauri/src/service.rs | Unknown download cleanup source |  |
| src-tauri/src/service.rs | Window state unavailable |  |
| src-tauri/src/service.rs | Writing the last reading position: {error} |  |
| src-tauri/src/service.rs | crash_report |  |
| src-tauri/src/service.rs | crash_report_acknowledge |  |
| src-tauri/src/service.rs | crash_report_delete |  |
| src-tauri/src/service.rs | debug_crash |  |
| src-tauri/src/service.rs | discoveryId |  |
| src-tauri/src/service.rs | integrity_status |  |
| src-tauri/src/service.rs | local-sharing-admission |  |
| src-tauri/src/service.rs | local-transfer-idle-release |  |
| src-tauri/src/service.rs | localTransferState |  |
| src-tauri/src/service.rs | localswarm_inspect |  |
| src-tauri/src/service.rs | localswarm_status |  |
| src-tauri/src/service.rs | ownerId |  |
| src-tauri/src/service.rs | pinnedBookIds |  |
| src-tauri/src/service.rs | privacyGeneration |  |
| src-tauri/src/service.rs | private-sync |  |
| src-tauri/src/service.rs | restricted |  |
| src-tauri/src/service.rs | retired |  |
| src-tauri/src/service.rs | search_owner |  |
| src-tauri/src/service.rs | speechPreferences |  |
| src-tauri/src/service.rs | swarmId |  |
| src-tauri/src/service.rs | transfer-background-release |  |
| src-tauri/src/service.rs | transfer-idle-release |  |
| src-tauri/src/service/android_voice_pack.rs | Invalid bundled voice APK extent |  |
| src-tauri/src/service/android_voice_pack.rs | Invalid bundled voice asset identity |  |
| src-tauri/src/service/android_voice_pack.rs | Invalid bundled voice availability |  |
| src-tauri/src/service/android_voice_pack.rs | Missing bundled voice APK location |  |
| src-tauri/src/service/android_voice_pack.rs | The installed voice APK changed. Restart the app and try again. |  |
| src-tauri/src/service/android_voice_pack.rs | bundleCheckFailed |  |
| src-tauri/src/service/android_voice_pack.rs | bundled |  |
| src-tauri/src/service/android_voice_pack.rs | bundled-voice |  |
| src-tauri/src/service/android_voice_pack.rs | bundledVoicePack |  |
| src-tauri/src/service/dispatch.rs | Acquisition cleanup requires explicit file deletion |  |
| src-tauri/src/service/dispatch.rs | Archive installation changed while confirming cleanup |  |
| src-tauri/src/service/dispatch.rs | Archive installation changed while confirming removal; select it again |  |
| src-tauri/src/service/dispatch.rs | Article preview MIME type is invalid |  |
| src-tauri/src/service/dispatch.rs | Article preview location is too long |  |
| src-tauri/src/service/dispatch.rs | At most 16 native windows can be open |  |
| src-tauri/src/service/dispatch.rs | Background download policy: {error} |  |
| src-tauri/src/service/dispatch.rs | Book zoom must be from 25% to 500% |  |
| src-tauri/src/service/dispatch.rs | Cannot monitor portable data directory {directory}: {error} |  |
| src-tauri/src/service/dispatch.rs | Debug crash archive is not in the library |  |
| src-tauri/src/service/dispatch.rs | Debug crash trigger is disabled |  |
| src-tauri/src/service/dispatch.rs | External provider content is read-only; remove its library reference instead |  |
| src-tauri/src/service/dispatch.rs | Invalid attachment export name |  |
| src-tauri/src/service/dispatch.rs | Invalid search error response |  |
| src-tauri/src/service/dispatch.rs | Invalid sharing port |  |
| src-tauri/src/service/dispatch.rs | Invalid speech preference: {error} |  |
| src-tauri/src/service/dispatch.rs | Invalid window surface |  |
| src-tauri/src/service/dispatch.rs | Library sharing serves every available archive; do not select individual archives |  |
| src-tauri/src/service/dispatch.rs | Map choice expired after a privacy policy transition |  |
| src-tauri/src/service/dispatch.rs | Map settings changed; select the map again |  |
| src-tauri/src/service/dispatch.rs | Missing archive storage batch |  |
| src-tauri/src/service/dispatch.rs | Missing book zoom |  |
| src-tauri/src/service/dispatch.rs | Missing bookmark |  |
| src-tauri/src/service/dispatch.rs | Missing discovery enabled flag |  |
| src-tauri/src/service/dispatch.rs | Missing download cleanup installations |  |
| src-tauri/src/service/dispatch.rs | Missing launch search ID |  |
| src-tauri/src/service/dispatch.rs | Missing pin state |  |
| src-tauri/src/service/dispatch.rs | Missing request ID |  |
| src-tauri/src/service/dispatch.rs | Missing restricted state |  |
| src-tauri/src/service/dispatch.rs | Missing sensor |  |
| src-tauri/src/service/dispatch.rs | Missing speech preference |  |
| src-tauri/src/service/dispatch.rs | Missing visit creation intent |  |
| src-tauri/src/service/dispatch.rs | Moving archives requires explicit confirmation |  |
| src-tauri/src/service/dispatch.rs | NEARBY_LAN_REQUIRED |  |
| src-tauri/src/service/dispatch.rs | Native windows require the desktop application |  |
| src-tauri/src/service/dispatch.rs | Nearby sharing ended before it was ready |  |
| src-tauri/src/service/dispatch.rs | New bookmarks require a title |  |
| src-tauri/src/service/dispatch.rs | No saved session is available to restore |  |
| src-tauri/src/service/dispatch.rs | Opening an attachment requires explicit confirmation |  |
| src-tauri/src/service/dispatch.rs | Opening the attachment in its default application: {e} |  |
| src-tauri/src/service/dispatch.rs | Resolved article preview location is too long |  |
| src-tauri/src/service/dispatch.rs | Resource exceeds in-memory export limit; use the archive resource URL to save large media |  |
| src-tauri/src/service/dispatch.rs | Restore previous session is unavailable in a read-only profile |  |
| src-tauri/src/service/dispatch.rs | Restore previous session is unavailable in private mode |  |
| src-tauri/src/service/dispatch.rs | Saved map download state could not be read: {error} |  |
| src-tauri/src/service/dispatch.rs | Search history book identity is invalid |  |
| src-tauri/src/service/dispatch.rs | Search history query is invalid or too large |  |
| src-tauri/src/service/dispatch.rs | Select an archive to suggest titles from |  |
| src-tauri/src/service/dispatch.rs | Select at least one archive to share |  |
| src-tauri/src/service/dispatch.rs | Selected file is a different archive; import it separately |  |
| src-tauri/src/service/dispatch.rs | Sensor installation is not an active archive listener for this window |  |
| src-tauri/src/service/dispatch.rs | Sensor origin is not an active archive listener for this window |  |
| src-tauri/src/service/dispatch.rs | Session archive changed while removing |  |
| src-tauri/src/service/dispatch.rs | Sharing is disabled in private mode |  |
| src-tauri/src/service/dispatch.rs | Spelling correction is disabled in private/read-only mode or preferences |  |
| src-tauri/src/service/dispatch.rs | Stop sharing before changing its selection or binding |  |
| src-tauri/src/service/dispatch.rs | Stop sharing before moving the library |  |
| src-tauri/src/service/dispatch.rs | Stop sharing this archive before removing it from the library |  |
| src-tauri/src/service/dispatch.rs | Stop the integrity check before deleting this archive |  |
| src-tauri/src/service/dispatch.rs | Stop the integrity check before moving the library |  |
| src-tauri/src/service/dispatch.rs | Suggestion prefix is too long |  |
| src-tauri/src/service/dispatch.rs | The archive contents changed. Search again. |  |
| src-tauri/src/service/dispatch.rs | The bookmark no longer exists |  |
| src-tauri/src/service/dispatch.rs | The installed archive list changed; close this dialog and try again |  |
| src-tauri/src/service/dispatch.rs | The map catalog URL does not match the configured catalog |  |
| src-tauri/src/service/dispatch.rs | The original library directory must be absolute |  |
| src-tauri/src/service/dispatch.rs | The saved session has no open tabs to restore |  |
| src-tauri/src/service/dispatch.rs | The saved session is already open |  |
| src-tauri/src/service/dispatch.rs | This download has no file that can be unlinked here; remove its archive from the library first, then delete the download files |  |
| src-tauri/src/service/dispatch.rs | Too many archive cleanup claims |  |
| src-tauri/src/service/dispatch.rs | Too many launch acknowledgements |  |
| src-tauri/src/service/dispatch.rs | Too many tabs |  |
| src-tauri/src/service/dispatch.rs | Unknown application command: {command} |  |
| src-tauri/src/service/dispatch.rs | [nearby] share all failed: {error} |  |
| src-tauri/src/service/dispatch.rs | activeSurface |  |
| src-tauri/src/service/dispatch.rs | activeTab |  |
| src-tauri/src/service/dispatch.rs | article.html |  |
| src-tauri/src/service/dispatch.rs | batch |  |
| src-tauri/src/service/dispatch.rs | binding |  |
| src-tauri/src/service/dispatch.rs | bookmark |  |
| src-tauri/src/service/dispatch.rs | cleanupSource |  |
| src-tauri/src/service/dispatch.rs | corrections |  |
| src-tauri/src/service/dispatch.rs | deleteFile |  |
| src-tauri/src/service/dispatch.rs | download |  |
| src-tauri/src/service/dispatch.rs | excerpt |  |
| src-tauri/src/service/dispatch.rs | fromDirectory |  |
| src-tauri/src/service/dispatch.rs | highAccuracy |  |
| src-tauri/src/service/dispatch.rs | highlightChange |  |
| src-tauri/src/service/dispatch.rs | highlightChange and highlights cannot be supplied together |  |
| src-tauri/src/service/dispatch.rs | history |  |
| src-tauri/src/service/dispatch.rs | launchLocations |  |
| src-tauri/src/service/dispatch.rs | launchSearch |  |
| src-tauri/src/service/dispatch.rs | localswarm_stop_library |  |
| src-tauri/src/service/dispatch.rs | map |  |
| src-tauri/src/service/dispatch.rs | nextToken |  |
| src-tauri/src/service/dispatch.rs | notifications |  |
| src-tauri/src/service/dispatch.rs | preference |  |
| src-tauri/src/service/dispatch.rs | privacyPolicyGeneration |  |
| src-tauri/src/service/dispatch.rs | recentSearches |  |
| src-tauri/src/service/dispatch.rs | referenceRevision |  |
| src-tauri/src/service/dispatch.rs | referenceTransitions |  |
| src-tauri/src/service/dispatch.rs | rejected |  |
| src-tauri/src/service/dispatch.rs | reveal_book |  |
| src-tauri/src/service/dispatch.rs | scope |  |
| src-tauri/src/service/dispatch.rs | sensor |  |
| src-tauri/src/service/dispatch.rs | sensor_activate |  |
| src-tauri/src/service/dispatch.rs | sensor_reset |  |
| src-tauri/src/service/dispatch.rs | sessionRestored |  |
| src-tauri/src/service/dispatch.rs | settings |  |
| src-tauri/src/service/dispatch.rs | shared_library_browse |  |
| src-tauri/src/service/dispatch.rs | spelling_build |  |
| src-tauri/src/service/dispatch.rs | startupWarnings |  |
| src-tauri/src/service/dispatch.rs | suggestions |  |
| src-tauri/src/service/dispatch.rs | tabs |  |
| src-tauri/src/service/dispatch.rs | thumbnail |  |
| src-tauri/src/service/dispatch.rs | window-{} |  |
| src-tauri/src/service/dispatch.rs | zoom |  |
| src-tauri/src/service/dispatch.rs | {error}; background download policy: {background} |  |
| src-tauri/src/service/dispatch.rs | {error}; background download policy: {policy} |  |
| src-tauri/src/service/dispatch.rs | {error}; crash capture policy: {policy} |  |
| src-tauri/src/service/dispatch.rs | {id}\u{1f}{query} |  |
| src-tauri/src/service/dispatch.rs | {label}:{book} |  |
| src-tauri/src/service/dispatch.rs | “{title}” is not available in this app’s storage. Choose Locate in the library to reconnect it. |  |
| src-tauri/src/share.rs | Activate the original window and try sharing again |  |
| src-tauri/src/share.rs | Kiwix stopped waiting for the share. A sharing service you already chose may still finish |  |
| src-tauri/src/share.rs | Kiwix stopped waiting. A sharing service you already chose may still finish |  |
| src-tauri/src/share.rs | Only a regular archive file can be shared |  |
| src-tauri/src/share.rs | Sharing authority expired. Open the item and try again | authority |
| src-tauri/src/share.rs | Sharing stopped because the app is closing |  |
| src-tauri/src/share.rs | Sharing was cancelled |  |
| src-tauri/src/share.rs | The archive at the selected path changed. Open it again before sharing |  |
| src-tauri/src/share.rs | The archive file is missing; relocate it before sharing |  |
| src-tauri/src/share.rs | The archive is no longer in the library |  |
| src-tauri/src/share.rs | The archive name is not UTF-8 |  |
| src-tauri/src/share.rs | The browsing policy is changing |  |
| src-tauri/src/share.rs | The file to share is unavailable |  |
| src-tauri/src/share.rs | The macOS share service failed ({detail}) |  |
| src-tauri/src/share.rs | The selected archive files changed. Verify them before sharing |  |
| src-tauri/src/share.rs | The selected archive installation changed. Open it again before sharing |  |
| src-tauri/src/share.rs | The share sheet did not complete |  |
| src-tauri/src/share.rs | The system share sheet could not be opened |  |
| src-tauri/src/share.rs | This file does not have a stable macOS share reference. Copy it to a local disk and try again |  |
| src-tauri/src/share.rs | Your privacy settings changed during sharing |  |
| src-tauri/src/share.rs | {error}. A sharing service you already chose may still finish |  |
| src-tauri/src/shared_library.rs | Book is not in this shared library |  |
| src-tauri/src/shared_library.rs | Invalid shared library book |  |
| src-tauri/src/shared_library.rs | Invalid shared library session |  |
| src-tauri/src/shared_library.rs | Shared library access is unavailable in private or read-only mode |  |
| src-tauri/src/shared_library.rs | Shared library did not respond |  |
| src-tauri/src/shared_library.rs | This archive is already in your library, but its file is unavailable. Locate or remove that entry in Library before downloading it again. |  |
| src-tauri/src/shared_library.rs | This archive is already in your library. Open the existing copy instead. |  |
| src-tauri/src/shared_library.rs | Too many shared libraries are open; close discovery and try again |  |
| src-tauri/src/sharing.rs | /.kiwix-access/{} |  |
| src-tauri/src/sharing.rs | <!doctype html><html lang=\"{}\" dir=\"{}\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style> |  |
| src-tauri/src/sharing.rs | <p class=\"error\">{}</p> |  |
| src-tauri/src/sharing.rs | Archive file identity changed |  |
| src-tauri/src/sharing.rs | Archive is not available |  |
| src-tauri/src/sharing.rs | Archive is not shared |  |
| src-tauri/src/sharing.rs | Archive search worker stopped |  |
| src-tauri/src/sharing.rs | Archive source changed while opening |  |
| src-tauri/src/sharing.rs | Article URL is too long |  |
| src-tauri/src/sharing.rs | Bootstrap body is too large |  |
| src-tauri/src/sharing.rs | Bootstrap body timed out |  |
| src-tauri/src/sharing.rs | Catalog count must be between 1 and 200 |  |
| src-tauri/src/sharing.rs | Catalog filter is too long |  |
| src-tauri/src/sharing.rs | Catalog query is too long |  |
| src-tauri/src/sharing.rs | Connect to a trusted local Wi-Fi or Ethernet network before sharing, or choose an advanced network binding |  |
| src-tauri/src/sharing.rs | Content-Range |  |
| src-tauri/src/sharing.rs | Import an available archive before serving the library |  |
| src-tauri/src/sharing.rs | Invalid UTF-8 archive path |  |
| src-tauri/src/sharing.rs | Invalid archive identifier |  |
| src-tauri/src/sharing.rs | Invalid article target |  |
| src-tauri/src/sharing.rs | Invalid bootstrap JSON |  |
| src-tauri/src/sharing.rs | Invalid catalog filter |  |
| src-tauri/src/sharing.rs | Invalid library page |  |
| src-tauri/src/sharing.rs | Joining library book listener: {error} |  |
| src-tauri/src/sharing.rs | Joining sharing listener: {error} |  |
| src-tauri/src/sharing.rs | One or more files you chose to share are missing. Choose the files again and start sharing |  |
| src-tauri/src/sharing.rs | Search is busy; try again shortly |  |
| src-tauri/src/sharing.rs | Shared library · Kiwix |  |
| src-tauri/src/sharing.rs | Sharing connection is idle |  |
| src-tauri/src/sharing.rs | Sharing has stopped |  |
| src-tauri/src/sharing.rs | Sharing over a Wi-Fi hotspot requires the running application's hotspot reservation |  |
| src-tauri/src/sharing.rs | Sharing request head timed out |  |
| src-tauri/src/sharing.rs | Sharing stopped |  |
| src-tauri/src/sharing.rs | The hotspot interface has no usable IPv4 address; restart the hotspot and try again |  |
| src-tauri/src/sharing.rs | The selected IP address is not an available local interface; refresh interfaces and choose again |  |
| src-tauri/src/sharing.rs | The sharing PIN must contain {PIN_MIN_DIGITS}–{PIN_MAX_DIGITS} digits |  |
| src-tauri/src/sharing.rs | This archive could not be searched. |  |
| src-tauri/src/sharing.rs | This archive file was replaced; import the new archive separately |  |
| src-tauri/src/sharing.rs | This archive is no longer available in the library |  |
| src-tauri/src/sharing.rs | This archive is no longer in the library |  |
| src-tauri/src/sharing.rs | This archive is no longer shared by its library owner |  |
| src-tauri/src/sharing.rs | This archive was relinked or replaced; reopen it from the library index |  |
| src-tauri/src/sharing.rs | This selected archive is no longer shared |  |
| src-tauri/src/sharing.rs | This shared file is no longer available. Stop sharing and start again |  |
| src-tauri/src/sharing.rs | Unknown or repeated catalog parameter |  |
| src-tauri/src/sharing.rs | Unknown or repeated library page parameter |  |
| src-tauri/src/sharing.rs | Use JSON or a PIN form |  |
| src-tauri/src/sharing.rs | bytes */{size} |  |
| src-tauri/src/sharing.rs | bytes */{} |  |
| src-tauri/src/sharing_download.rs | Archive changed while opening |  |
| src-tauri/src/sharing_download.rs | Archive cohort does not end at its declared ZIM checksum boundary |  |
| src-tauri/src/sharing_download.rs | Archive part length changed |  |
| src-tauri/src/sharing_download.rs | Archive parts do not match the admitted ZIM length |  |
| src-tauri/src/sharing_download.rs | Archive source identity changed |  |
| src-tauri/src/sharing_download.rs | Archive source identity changed during read |  |
| src-tauri/src/sharing_download.rs | Invalid logical archive range |  |
| src-tauri/src/sharing_download.rs | Truncated ZIM cohort |  |
| src-tauri/src/sharing_reader.rs | <!doctype html><html lang=\"{}\" dir=\"{}\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>:root{{color-scheme:light;--ink:#183b35;--paper:#f6f2e8}}body{{max-width:900px;margin:auto;padding:2rem 1.5rem;background:var(--paper);color:var(--ink);font:18px/1.6 Georgia,serif}}h1{{border-top:8px solid var(--ink);padding-top:1rem;font-size:3rem;line-height:1.1}}a{{color:var(--ink)}}form{{display:flex;gap:1rem;flex-wrap:wrap;border-block:1px solid #c3cdc1;padding:1rem 0}}label{{display:flex;flex-direction:column}}input,select,button{{font:inherit;padding:.5rem;max-width:100%}}button{{cursor:pointer}}li{{padding:.5rem 0}}small{{display:block}}.note{{color:#4d625c}}[hidden]{{display:none}}</style><a href=\"/{}/\">{}</a><h1>{}</h1><form id=\"search\" role=\"search\"><label>{}<input id=\"q\" name=\"q\" type=\"search\" maxlength=\"256\" autocomplete=\"off\" aria-controls=\"suggestions\" required></label><label>{}<select id=\"book\"><option value=\"\">{}</option> |  |
| src-tauri/src/sharing_reader.rs | </select></label><label>{}<select id=\"mode\"><option value=\"combined\">{}</option><option value=\"title\">{}</option><option value=\"fulltext\">{}</option></select></label><button>{}</button></form><ul id=\"suggestions\" aria-label=\"{}\"></ul><p id=\"status\" role=\"status\" aria-live=\"polite\"></p><ol id=\"results\"></ol><p id=\"limit\" class=\"note\" role=\"status\" hidden>{}</p><button id=\"more\" hidden>{}</button><script src=\"/{}/reader.js\" defer></script></html> |  |
| src-tauri/src/sharing_reader.rs | <option value=\"{}\">{}</option> |  |
| src-tauri/src/sharing_reader.rs | All shared items |  |
| src-tauri/src/sharing_reader.rs | Archive |  |
| src-tauri/src/sharing_reader.rs | Archive identifier is too long |  |
| src-tauri/src/sharing_reader.rs | Archive is no longer shared |  |
| src-tauri/src/sharing_reader.rs | Browse the collection |  |
| src-tauri/src/sharing_reader.rs | Full text |  |
| src-tauri/src/sharing_reader.rs | Invalid search continuation |  |
| src-tauri/src/sharing_reader.rs | Invalid search offset |  |
| src-tauri/src/sharing_reader.rs | More results |  |
| src-tauri/src/sharing_reader.rs | Refine your search to see more results. |  |
| src-tauri/src/sharing_reader.rs | Search |  |
| src-tauri/src/sharing_reader.rs | Search URL is too long |  |
| src-tauri/src/sharing_reader.rs | Search articles |  |
| src-tauri/src/sharing_reader.rs | Search mode |  |
| src-tauri/src/sharing_reader.rs | Search must be at most 256 characters and 1024 bytes, without control characters |  |
| src-tauri/src/sharing_reader.rs | Title suggestions |  |
| src-tauri/src/sharing_reader.rs | Titles |  |
| src-tauri/src/sharing_reader.rs | Titles and full text |  |
| src-tauri/src/sharing_reader.rs | Unknown or repeated search parameter |  |
| src-tauri/src/sharing_reader.rs | mode |  |
| src-tauri/src/spelling.rs | Another spelling dictionary is already building |  |
| src-tauri/src/spelling.rs | Invalid or oversized spelling cache file |  |
| src-tauri/src/spelling.rs | Invalid, changing, or oversized spelling title listing (maximum 20 million titles) |  |
| src-tauri/src/spelling.rs | Missing spelling directory |  |
| src-tauri/src/spelling.rs | Missing spelling file |  |
| src-tauri/src/spelling.rs | No spelling job exists for this archive |  |
| src-tauri/src/spelling.rs | Spelling build cancelled |  |
| src-tauri/src/spelling.rs | Spelling builds are disabled in private or read-only mode |  |
| src-tauri/src/spelling.rs | Spelling cache cleanup: {error} |  |
| src-tauri/src/spelling.rs | Spelling cache loading cannot be cancelled |  |
| src-tauri/src/spelling.rs | Spelling cache must be a regular file with a single link |  |
| src-tauri/src/spelling.rs | Spelling cache publication disabled by privacy policy |  |
| src-tauri/src/spelling.rs | Spelling dictionary exceeds 2 million distinct words |  |
| src-tauri/src/spelling.rs | Spelling directory changed before publication |  |
| src-tauri/src/spelling.rs | Spelling job identity changed |  |
| src-tauri/src/spelling.rs | Spelling load identity changed |  |
| src-tauri/src/spelling.rs | Spelling scratch file changed |  |
| src-tauri/src/spelling.rs | Spelling staging file changed during validation |  |
| src-tauri/src/spelling.rs | The published spelling file changed before recovery |  |
| src-tauri/src/spelling.rs | The spelling index could not be saved and recovery could not be confirmed. Check the spelling folder, available space and permissions before retrying. Previous backup name, if present: {}. {error}. Recovery: {recovery} |  |
| src-tauri/src/spelling.rs | The spelling index could not be saved. Check available space and permissions, then retry. {error} |  |
| src-tauri/src/spelling/index.rs | Spelling build exceeds 2 GiB of working storage |  |
| src-tauri/src/spelling/index.rs | Spelling candidate limit exceeded |  |
| src-tauri/src/storage.rs | Capability rename target must be a single filename |  |
| src-tauri/src/storage.rs | Directory capability changed identity during synchronization |  |
| src-tauri/src/storage.rs | Directory capability does not refer to a directory |  |
| src-tauri/src/storage.rs | Directory path contains a non-normal component |  |
| src-tauri/src/storage.rs | Directory path must be normalized and absolute |  |
| src-tauri/src/storage.rs | Directory {} is outside native iOS container {} |  |
| src-tauri/src/storage.rs | Download directory has no accessible filesystem ancestor |  |
| src-tauri/src/storage.rs | Inspecting download filesystem {}: {e} |  |
| src-tauri/src/storage.rs | Invalid native iOS container owner or type |  |
| src-tauri/src/storage.rs | Invalid native iOS container path |  |
| src-tauri/src/storage.rs | Native iOS container identity changed |  |
| src-tauri/src/storage.rs | Not a download directory: {} |  |
| src-tauri/src/storage.rs | Publication path contains NUL |  |
| src-tauri/src/storage.rs | Readable directory descriptor changed capability identity |  |
| src-tauri/src/storage.rs | {} uses FAT/FAT32, which cannot store one file larger than 4 GiB − 1 byte. Choose an exFAT, NTFS, APFS or other large-file-capable download directory. |  |
| src-tauri/src/store.rs | * |  |
| src-tauri/src/store.rs | A bookmark contains the same highlight twice. Remove one and save again |  |
| src-tauri/src/store.rs | A bookmark may contain at most 200 highlights |  |
| src-tauri/src/store.rs | A saved highlight is damaged. Remove the highlight, then add it again |  |
| src-tauri/src/store.rs | A saved library entry is damaged. Remove that archive from your library, then add it again |  |
| src-tauri/src/store.rs | A saved {kind} item could not be read and was skipped. |  |
| src-tauri/src/store.rs | A speech edit requires a language and an edited voice or speed |  |
| src-tauri/src/store.rs | Acquisition destination must be an absolute verified path |  |
| src-tauri/src/store.rs | Acquisition destination must be valid Unicode without NUL |  |
| src-tauri/src/store.rs | Article identity requires a nonempty book ID and a path without NUL |  |
| src-tauri/src/store.rs | Book identity is invalid |  |
| src-tauri/src/store.rs | Book path must be nonempty and contain no NUL |  |
| src-tauri/src/store.rs | Book zoom requires an archive identity and a value from 25% to 500% |  |
| src-tauri/src/store.rs | Bookmark ID must be nonempty and contain no control characters |  |
| src-tauri/src/store.rs | Cannot inspect saved policy in {}: {error} |  |
| src-tauri/src/store.rs | Cannot inspect saved policy in {}: {error}. The original is untouched. |  |
| src-tauri/src/store.rs | Cannot open {}: read-only startup requires an existing initialized profile |  |
| src-tauri/src/store.rs | Cannot open {}: {error} |  |
| src-tauri/src/store.rs | Cannot open {}: {e} |  |
| src-tauri/src/store.rs | Cannot resolve legacy library paths outside a Unicode working directory |  |
| src-tauri/src/store.rs | Catalog/proxy URLs must be HTTP(S) URLs without embedded credentials |  |
| src-tauri/src/store.rs | Close an external archive before opening another (32 session archives maximum) |  |
| src-tauri/src/store.rs | Directory paths cannot contain NUL bytes |  |
| src-tauri/src/store.rs | Duplicate book identity |  |
| src-tauri/src/store.rs | Duplicate book installation |  |
| src-tauri/src/store.rs | Duplicate bookmark identity |  |
| src-tauri/src/store.rs | Duplicate history identity |  |
| src-tauri/src/store.rs | Duplicate tab identity |  |
| src-tauri/src/store.rs | Highlight documentPath must be a bounded archive-relative location |  |
| src-tauri/src/store.rs | Highlight offsets must match the UTF-16 quote length |  |
| src-tauri/src/store.rs | Highlight text exceeds its allowed bounds |  |
| src-tauri/src/store.rs | History identity is invalid |  |
| src-tauri/src/store.rs | Invalid bookmark createdAt {:?}; expected RFC3339 |  |
| src-tauri/src/store.rs | Invalid or duplicate {kind} ID {id:?} |  |
| src-tauri/src/store.rs | Invalid saved reference revision |  |
| src-tauri/src/store.rs | Invalid {name}: {value:?} |  |
| src-tauri/src/store.rs | Leave read-only mode before changing this profile |  |
| src-tauri/src/store.rs | Leave read-only mode before publishing an acquisition |  |
| src-tauri/src/store.rs | Library entry in shell payload is missing {key} |  |
| src-tauri/src/store.rs | Library entry in shell payload is not an object |  |
| src-tauri/src/store.rs | Library filter selections are invalid or too large |  |
| src-tauri/src/store.rs | Pin at most {} library archives at a time |  |
| src-tauri/src/store.rs | Pinned archives must be unique |  |
| src-tauri/src/store.rs | Pinned archives require a nonempty archive identity |  |
| src-tauri/src/store.rs | Portable data path is not Unicode |  |
| src-tauri/src/store.rs | Portable downloads must use the adjacent data directory |  |
| src-tauri/src/store.rs | Portable mode fixes download and monitored folders to the adjacent data directory |  |
| src-tauri/src/store.rs | Portable mode requires the adjacent data directory; conflicting --data-dir is not permitted |  |
| src-tauri/src/store.rs | Private reading sync is unavailable in private or read-only mode |  |
| src-tauri/src/store.rs | Profile no longer identifies this application's retained process lock |  |
| src-tauri/src/store.rs | Provider source required |  |
| src-tauri/src/store.rs | Read-only mode is enabled; the saved window layout was not reset |  |
| src-tauri/src/store.rs | Read-only mode prevents saving files |  |
| src-tauri/src/store.rs | Recent searches exceed the supported history bounds |  |
| src-tauri/src/store.rs | Remembered picker folders must be absolute paths of at most 32768 bytes without NUL |  |
| src-tauri/src/store.rs | Replacement must create a unique new archive installation |  |
| src-tauri/src/store.rs | Saved settings are not a JSON object |  |
| src-tauri/src/store.rs | Scroll position must be a finite nonnegative number |  |
| src-tauri/src/store.rs | Session-only external archive activity cannot be saved to the profile |  |
| src-tauri/src/store.rs | Session-only provider references cannot be saved to the profile |  |
| src-tauri/src/store.rs | Sharing name must be at most 160 UTF-8 bytes, without surrounding whitespace, controls or direction overrides |  |
| src-tauri/src/store.rs | Some saved items could not be read by this version of Kiwix and are not shown. |  |
| src-tauri/src/store.rs | Some saved items could not be read by this version of Kiwix and are not shown. A copy of the saved profile that still contains them was kept at {}. Putting that copy in place of state.json lets a version of Kiwix that can read them show them again. |  |
| src-tauri/src/store.rs | Speech preferences require a language, an installed voice identifier and a speed from 0.25 to 2 |  |
| src-tauri/src/store.rs | State exceeds 128 MiB; transaction was not committed |  |
| src-tauri/src/store.rs | State file is missing its version |  |
| src-tauri/src/store.rs | State file must be a JSON object |  |
| src-tauri/src/store.rs | Tab identity is invalid |  |
| src-tauri/src/store.rs | Tab paths cannot contain NUL |  |
| src-tauri/src/store.rs | The highlight no longer exists |  |
| src-tauri/src/store.rs | The profile relocation is incomplete. Keep the profile writable and try again after the remaining files are repaired. |  |
| src-tauri/src/store.rs | The saved reading list is damaged. Its causal sync history was preserved; restore or repair this profile before syncing so recovery cannot become a deletion |  |
| src-tauri/src/store.rs | The saved session exceeds the supported size |  |
| src-tauri/src/store.rs | The saved session exceeds the supported size of {MAX_SESSION_BYTES} bytes. The previous saved session was retained. |  |
| src-tauri/src/store.rs | The saved session has no main window |  |
| src-tauri/src/store.rs | The saved session has too many windows |  |
| src-tauri/src/store.rs | The saved session names an unknown window |  |
| src-tauri/src/store.rs | The session archive changed or is no longer available |  |
| src-tauri/src/store.rs | This archive can only be used for the current session. Reopen it from its original app |  |
| src-tauri/src/store.rs | This archive or installation already exists in the library |  |
| src-tauri/src/store.rs | This profile was opened read-only; state changes are disabled |  |
| src-tauri/src/store.rs | This profile was saved by a different version of Kiwix (version {version}; this app reads version {}). Open it with that version, or choose another profile. Your saved file was not changed. |  |
| src-tauri/src/store.rs | This profile was saved by a different version of Kiwix (version {}; this app reads version {}). Open it with that version, or choose another profile. Your saved file was not changed. |  |
| src-tauri/src/store.rs | Too many speech language preferences |  |
| src-tauri/src/store.rs | Transient provider admission requires private or read-only mode |  |
| src-tauri/src/store.rs | Transient provider replacement requires private or read-only mode |  |
| src-tauri/src/store.rs | Unknown window surface |  |
| src-tauri/src/store.rs | Unsupported saved-session version |  |
| src-tauri/src/store.rs | activeTab must identify an existing tab in its window |  |
| src-tauri/src/store.rs | editionBindings |  |
| src-tauri/src/store.rs | editionTransactions |  |
| src-tauri/src/store.rs | locale must be a nonempty language tag of at most 128 bytes |  |
| src-tauri/src/store.rs | mainPath |  |
| src-tauri/src/store.rs | privateSync |  |
| src-tauri/src/store.rs | readingSync |  |
| src-tauri/src/store.rs | state.json |  |
| src-tauri/src/store.rs | state.skipped-{}.json |  |
| src-tauri/src/store.rs | store.lock |  |
| src-tauri/src/store.rs | validated Unicode destination |  |
| src-tauri/src/store.rs | {SKIPPED} Kiwix could not keep a copy of the saved profile ({error}). Back up {original} now: the next save does not include them. |  |
| src-tauri/src/store.rs | {name} must be a finite number from {min} to {max} |  |
| src-tauri/src/store.rs | {} exceeds the 128 MiB state limit; preserve the file and export/reduce it with a compatible application |  |
| src-tauri/src/store.rs | {} exceeds the 128 MiB state limit; the original is untouched |  |
| src-tauri/src/store.rs | {} must be a bounded regular non-symlink file |  |
| src-tauri/src/store.rs | {} must be a regular file |  |
| src-tauri/src/timestamps.rs | 60 |  |
| src-tauri/src/timestamps.rs | :59 |  |
| src-tauri/src/timestamps.rs | Invalid mail date offset |  |
| src-tauri/src/timestamps.rs | Invalid mail date weekday |  |
| src-tauri/src/timestamps.rs | Invalid timestamp separator |  |
| src-tauri/src/torrent.rs | .kiwix-publish-cleanup- |  |
| src-tauri/src/torrent.rs | .kiwix-torrent-{suffix} |  |
| src-tauri/src/torrent.rs | ; removing empty owned capture: {error} |  |
| src-tauri/src/torrent.rs | ; restoring ownership checkpoint: {error} |  |
| src-tauri/src/torrent.rs | A saved shared file entry is damaged. Kiwix set it aside; share the file again if it is missing |  |
| src-tauri/src/torrent.rs | A saved transfer could not be read and was skipped. |  |
| src-tauri/src/torrent.rs | A saved transfer detail is damaged. Kiwix set it aside; download the file again if it is missing |  |
| src-tauri/src/torrent.rs | A saved transfer is damaged. Kiwix set it aside; download the file again if it is missing |  |
| src-tauri/src/torrent.rs | An unexpected file appeared in a temporary folder. Kiwix left it in place; restart the app |  |
| src-tauri/src/torrent.rs | Cannot validate owned cleanup capture: {error} |  |
| src-tauri/src/torrent.rs | Captured publication no longer matches its original bytes |  |
| src-tauri/src/torrent.rs | Cleanup capture contains a replaced payload |  |
| src-tauri/src/torrent.rs | Cleanup capture directory was replaced; files were preserved |  |
| src-tauri/src/torrent.rs | Cleanup directory was replaced; no files were authorized for deletion |  |
| src-tauri/src/torrent.rs | Cleanup file has no parent |  |
| src-tauri/src/torrent.rs | Complete torrent metadata temporary is missing; its journal was preserved |  |
| src-tauri/src/torrent.rs | Completed torrent path has no original publication proof; library deletion was not authorized |  |
| src-tauri/src/torrent.rs | Download file is not its originally owned inode; cleanup was not authorized |  |
| src-tauri/src/torrent.rs | Invalid cleanup capture name |  |
| src-tauri/src/torrent.rs | Invalid cleanup capture ownership journal |  |
| src-tauri/src/torrent.rs | Invalid cleanup filename |  |
| src-tauri/src/torrent.rs | Invalid download cleanup capture |  |
| src-tauri/src/torrent.rs | Invalid download cleanup ownership journal |  |
| src-tauri/src/torrent.rs | Invalid published torrent filename |  |
| src-tauri/src/torrent.rs | Invalid saved transfer history JSON: {error} |  |
| src-tauri/src/torrent.rs | Invalid saved transfer history: expected a list of jobs |  |
| src-tauri/src/torrent.rs | Invalid saved transfer history: expected an object |  |
| src-tauri/src/torrent.rs | Invalid saved transfer history: missing version |  |
| src-tauri/src/torrent.rs | Invalid torrent job identifier |  |
| src-tauri/src/torrent.rs | Invalid torrent metadata filename |  |
| src-tauri/src/torrent.rs | Invalid torrent publication intent |  |
| src-tauri/src/torrent.rs | Invalid torrent share fields |  |
| src-tauri/src/torrent.rs | Kiwix found an unexpected temporary file and left it alone. Restart the app |  |
| src-tauri/src/torrent.rs | Kiwix found an unexpected temporary folder and left it alone. Restart the app |  |
| src-tauri/src/torrent.rs | Legacy torrent metadata lacks original file ownership proof |  |
| src-tauri/src/torrent.rs | Legacy torrent publication lacks original directory ownership proof; library deletion was not authorized |  |
| src-tauri/src/torrent.rs | Magnet must contain a BitTorrent v1 info hash |  |
| src-tauri/src/torrent.rs | Missing cleanup capture |  |
| src-tauri/src/torrent.rs | Missing metadata size |  |
| src-tauri/src/torrent.rs | Missing new cleanup capture |  |
| src-tauri/src/torrent.rs | Missing published torrent filename |  |
| src-tauri/src/torrent.rs | Missing torrent archive filename |  |
| src-tauri/src/torrent.rs | New cleanup capture was replaced; unrelated directory preserved |  |
| src-tauri/src/torrent.rs | No metadata parent |  |
| src-tauri/src/torrent.rs | Owned cleanup entry disappeared during validation |  |
| src-tauri/src/torrent.rs | Owned cleanup file has no parent |  |
| src-tauri/src/torrent.rs | Owned download directory was replaced; cleanup was not authorized |  |
| src-tauri/src/torrent.rs | Owned download file changed during provenance validation |  |
| src-tauri/src/torrent.rs | Owned download file has no parent |  |
| src-tauri/src/torrent.rs | Owned download filename does not match its journal |  |
| src-tauri/src/torrent.rs | Owned staging file disappeared during provenance validation |  |
| src-tauri/src/torrent.rs | Published torrent archive disappeared during authorization |  |
| src-tauri/src/torrent.rs | Published torrent archive disappeared during checksum verification |  |
| src-tauri/src/torrent.rs | Published torrent archive lacks its original ownership proof; library deletion was not authorized |  |
| src-tauri/src/torrent.rs | Published torrent archive was replaced; library deletion was not authorized |  |
| src-tauri/src/torrent.rs | Published torrent metadata no longer matches its complete durable bytes |  |
| src-tauri/src/torrent.rs | Saved transfer history was written by a different version of Kiwix ({version}). Open it with that version, or start with a new profile |  |
| src-tauri/src/torrent.rs | The downloaded file changed since it was saved. Remove it from the library and delete the file yourself |  |
| src-tauri/src/torrent.rs | The downloaded file changed while Kiwix was checking it. Download it again |  |
| src-tauri/src/torrent.rs | The same temporary transfer appears twice in saved history. Kiwix skipped the duplicate |  |
| src-tauri/src/torrent.rs | The same transfer appears twice in saved history. Kiwix skipped the duplicate |  |
| src-tauri/src/torrent.rs | This file cannot be shared because it is not a normal file. Copy it into your library and try again |  |
| src-tauri/src/torrent.rs | This shared file is no longer where Kiwix last saw it. Add it to the library again |  |
| src-tauri/src/torrent.rs | Too many torrent metadata redirects |  |
| src-tauri/src/torrent.rs | Torrent URLs cannot contain credentials |  |
| src-tauri/src/torrent.rs | Torrent bandwidth must be zero (unlimited) or 16384–4294967295 bytes/second |  |
| src-tauri/src/torrent.rs | Torrent capacity requirement overflow |  |
| src-tauri/src/torrent.rs | Torrent cleanup capture contains an unowned entry |  |
| src-tauri/src/torrent.rs | Torrent cleanup capture directory was replaced |  |
| src-tauri/src/torrent.rs | Torrent contains an absolute path or traversal |  |
| src-tauri/src/torrent.rs | Torrent contains an unsafe portable filename |  |
| src-tauri/src/torrent.rs | Torrent filename is not UTF-8 |  |
| src-tauri/src/torrent.rs | Torrent metadata disappeared during provenance validation |  |
| src-tauri/src/torrent.rs | Torrent metadata exceeds 16 MiB |  |
| src-tauri/src/torrent.rs | Torrent metadata has no parent directory |  |
| src-tauri/src/torrent.rs | Torrent metadata is empty or exceeds 16 MiB |  |
| src-tauri/src/torrent.rs | Torrent metadata is missing |  |
| src-tauri/src/torrent.rs | Torrent metadata is not a regular file of at most 16 MiB |  |
| src-tauri/src/torrent.rs | Torrent metadata no longer matches its complete durable publication |  |
| src-tauri/src/torrent.rs | Torrent metadata publication ownership does not match its temporary |  |
| src-tauri/src/torrent.rs | Torrent metadata requires HTTPS (literal loopback HTTP is allowed for local testing), or a magnet URI |  |
| src-tauri/src/torrent.rs | Torrent metadata temporary no longer matches its complete durable bytes |  |
| src-tauri/src/torrent.rs | Torrent publication size overflow |  |
| src-tauri/src/torrent.rs | Torrent size overflow |  |
| src-tauri/src/torrent.rs | Torrent staging directory was replaced; cleanup was not authorized |  |
| src-tauri/src/torrent.rs | Torrent staging has no filename |  |
| src-tauri/src/torrent.rs | Torrent staging has no parent |  |
| src-tauri/src/torrent.rs | Torrent staging is not its owned directory; cleanup was not authorized |  |
| src-tauri/src/torrent.rs | Torrent staging lacks its original directory ownership proof |  |
| src-tauri/src/torrent.rs | payload |  |
| src-tauri/src/torrent.rs | {error}; metadata temporary cleanup: {cleanup} |  |
| src-tauri/src/torrent.rs | {error}{}{} |  |
| src-tauri/src/torrent.rs | {suffix}.torrent |  |
| src-tauri/src/torrent/manager.rs | Archive has no primary file |  |
| src-tauri/src/torrent/manager.rs | Cannot prove ownership of legacy torrent metadata; file and history were preserved |  |
| src-tauri/src/torrent/manager.rs | Cannot prove ownership of legacy torrent staging; files and history were preserved |  |
| src-tauri/src/torrent/manager.rs | Cannot stage {}: {e} |  |
| src-tauri/src/torrent/manager.rs | Choose an existing download directory |  |
| src-tauri/src/torrent/manager.rs | Created torrent info hash is inconsistent |  |
| src-tauri/src/torrent/manager.rs | LOW_DISK_SPACE: |  |
| src-tauri/src/torrent/manager.rs | Missing archive filename |  |
| src-tauri/src/torrent/manager.rs | Missing publication size |  |
| src-tauri/src/torrent/manager.rs | Missing torrent filename |  |
| src-tauri/src/torrent/manager.rs | Only a single complete .zim archive can be shared |  |
| src-tauri/src/torrent/manager.rs | Only finished torrents can authorize cleanup |  |
| src-tauri/src/torrent/manager.rs | Only finished, non-seeding torrents can be removed from history |  |
| src-tauri/src/torrent/manager.rs | Only the complete .zim archive can be shared |  |
| src-tauri/src/torrent/manager.rs | Publication cleanup journal does not match the original download |  |
| src-tauri/src/torrent/manager.rs | Published archive has no ownership intent |  |
| src-tauri/src/torrent/manager.rs | Removing owned empty torrent staging (unknown files are preserved): {error} |  |
| src-tauri/src/torrent/manager.rs | Split archives cannot be shared as a single torrent |  |
| src-tauri/src/torrent/manager.rs | Staging has no filename |  |
| src-tauri/src/torrent/manager.rs | Staging has no parent |  |
| src-tauri/src/torrent/manager.rs | Start a new job to acquire a cancelled torrent |  |
| src-tauri/src/torrent/manager.rs | The archive was removed from the library |  |
| src-tauri/src/torrent/manager.rs | The library archive changed after selection |  |
| src-tauri/src/torrent/manager.rs | The library archive changed or was relinked |  |
| src-tauri/src/torrent/manager.rs | The library archive changed or was relinked after selection |  |
| src-tauri/src/torrent/manager.rs | The library archive identity or size changed |  |
| src-tauri/src/torrent/manager.rs | The previous transfer of this torrent is still stopping. Try again in a moment |  |
| src-tauri/src/torrent/manager.rs | The selected archive is unavailable; reconnect it before sharing |  |
| src-tauri/src/torrent/manager.rs | The share was removed from history |  |
| src-tauri/src/torrent/manager.rs | The shared file changed, so sharing stopped. Start sharing again |  |
| src-tauri/src/torrent/manager.rs | The transfer did not stop cleanly, so Kiwix did not delete any files. Restart the app, then retry |  |
| src-tauri/src/torrent/manager.rs | The transfer did not stop cleanly. No files were deleted. Restart Kiwix, then try again |  |
| src-tauri/src/torrent/manager.rs | The transfer did not stop cleanly. Restart Kiwix, then resume |  |
| src-tauri/src/torrent/manager.rs | The transfer did not stop cleanly. Restart the app, then retry |  |
| src-tauri/src/torrent/manager.rs | The transfer is already stopping. Wait a moment |  |
| src-tauri/src/torrent/manager.rs | The transfer is still stopping. Wait a moment, then try again |  |
| src-tauri/src/torrent/manager.rs | The transfer stopped unexpectedly |  |
| src-tauri/src/torrent/manager.rs | This archive is already being shared |  |
| src-tauri/src/torrent/manager.rs | This file cannot be shared because it has no name. Rename it and try again |  |
| src-tauri/src/torrent/manager.rs | This file is no longer in your library, so sharing stopped |  |
| src-tauri/src/torrent/manager.rs | This file name cannot be shared. Rename the file and try again |  |
| src-tauri/src/torrent/manager.rs | This torrent is already managed by another acquisition job |  |
| src-tauri/src/torrent/manager.rs | Torrent archives have colliding destination filenames |  |
| src-tauri/src/torrent/manager.rs | Torrent changed during cleanup authorization; retry with its current state |  |
| src-tauri/src/torrent/manager.rs | Torrent changed while its history was being removed |  |
| src-tauri/src/torrent/manager.rs | Torrent contains duplicate paths |  |
| src-tauri/src/torrent/manager.rs | Torrent does not contain a complete .zim archive |  |
| src-tauri/src/torrent/manager.rs | Torrent download no longer exists |  |
| src-tauri/src/torrent/manager.rs | Torrent file has no staging directory |  |
| src-tauri/src/torrent/manager.rs | Torrent has an invalid number of files |  |
| src-tauri/src/torrent/manager.rs | Torrent manager has shut down |  |
| src-tauri/src/torrent/manager.rs | Torrent manager is suspended |  |
| src-tauri/src/torrent/manager.rs | Torrent metadata changed since this job was created |  |
| src-tauri/src/torrent/manager.rs | Torrent metadata changed since this share was created |  |
| src-tauri/src/torrent/manager.rs | Torrent metadata does not describe the selected archive |  |
| src-tauri/src/torrent/manager.rs | Torrent metadata is no longer available |  |
| src-tauri/src/torrent/manager.rs | Torrent publication changed during verification; retry the import |  |
| src-tauri/src/torrent/manager.rs | Torrent staging directory was replaced; files and history were preserved |  |
| src-tauri/src/torrent/manager.rs | Torrent staging is no longer an owned directory; entry and history were preserved |  |
| src-tauri/src/torrent/manager.rs | Torrent staging subdirectory was replaced; directory preserved |  |
| src-tauri/src/torrent/manager.rs | Torrent symlink entries are not permitted |  |
| src-tauri/src/torrent/manager.rs | Torrent was cancelled |  |
| src-tauri/src/torrent/manager.rs | Torrent was removed from history or file cleanup has started |  |
| src-tauri/src/torrent/manager.rs | Torrent was removed from history or file cleanup has started; retry deletion instead |  |
| src-tauri/src/torrent/manager.rs | Torrent worker failed before acknowledging its pause |  |
| src-tauri/src/torrent/manager.rs | Torrent worker failed during shutdown: {error} |  |
| src-tauri/src/torrent/manager.rs | Torrent worker failed during suspension: {error} |  |
| src-tauri/src/torrent/manager.rs | Torrent worker is still retiring |  |
| src-tauri/src/torrent/manager.rs | Torrent worker stopped unexpectedly; its last durable checkpoint was retained |  |
| src-tauri/src/torrent/manager.rs | Unknown torrent job |  |
| src-tauri/src/torrent/manager.rs | Unsupported torrent history version |  |
| src-tauri/src/torrent/manager.rs | Unsupported torrent monitor history version |  |
| src-tauri/src/torrent/manager.rs | seeding |  |
| src-tauri/src/torrent/publication.rs | in {}: {error} |  |
| src-tauri/src/torrent/publication.rs | A saved transfer is damaged. Download the file again |  |
| src-tauri/src/torrent/publication.rs | Cannot publish {final_path}: an unowned destination already exists |  |
| src-tauri/src/torrent/publication.rs | Cannot safely recover torrent publication: {error} |  |
| src-tauri/src/torrent/publication.rs | Captured download no longer matches its original publication bytes; file preserved{} |  |
| src-tauri/src/torrent/publication.rs | Invalid torrent temporary name |  |
| src-tauri/src/torrent/publication.rs | Kiwix cannot verify this torrent's original publication. Add the archive to your library manually |  |
| src-tauri/src/torrent/publication.rs | Kiwix needs a folder owned by your account that other users cannot write to. Choose a private download folder. If it is already private, check the permissions on the torrents folder inside your Kiwix data folder. |  |
| src-tauri/src/torrent/publication.rs | Missing completed torrent archive filename |  |
| src-tauri/src/torrent/publication.rs | Missing engine-verified torrent piece receipt |  |
| src-tauri/src/torrent/publication.rs | Missing engine-verified torrent publication source |  |
| src-tauri/src/torrent/publication.rs | This torrent publication is no longer available for import |  |
| src-tauri/src/torrent/publication.rs | Torrent import path does not match its published archive |  |
| src-tauri/src/torrent/publication.rs | Torrent publication cancelled |  |
| src-tauri/src/torrent/publication.rs | Torrent publication cancelled; staging was retained |  |
| src-tauri/src/torrent/publication.rs | Torrent publication destination does not match its durable SHA-256 intent; it was not changed |  |
| src-tauri/src/torrent/publication.rs | Torrent publication destination is not the intended copied file; it was not changed |  |
| src-tauri/src/torrent/publication.rs | Torrent publication destination was replaced during recovery; it was not changed |  |
| src-tauri/src/torrent/publication.rs | Torrent publication directory was replaced; files were preserved |  |
| src-tauri/src/torrent/publication.rs | Torrent publication file was truncated |  |
| src-tauri/src/torrent/publication.rs | Torrent publication has no filename |  |
| src-tauri/src/torrent/publication.rs | Torrent publication output changed while copying |  |
| src-tauri/src/torrent/publication.rs | Torrent publication output changed while copying: {} |  |
| src-tauri/src/torrent/publication.rs | Torrent publication ownership is ambiguous |  |
| src-tauri/src/torrent/publication.rs | Torrent publication source changed before copying: {} |  |
| src-tauri/src/torrent/publication.rs | Torrent publication source changed while copying: {} |  |
| src-tauri/src/torrent/publication.rs | Torrent temporary capture directory changed; files preserved |  |
| src-tauri/src/torrent/publication.rs | Torrent temporary capture directory is missing; file preserved |  |
| src-tauri/src/torrent/publication.rs | Torrent temporary capture directory was replaced; files preserved |  |
| src-tauri/src/torrent/publication.rs | Torrent temporary was replaced; unrelated entry preserved{} |  |
| src-tauri/src/torrent/publication.rs | Unsupported torrent backup evidence version |  |
| src-tauri/src/torrent/publication.rs | {error}; temporary cleanup: {cleanup} |  |
| src-tauri/src/torrent/publication.rs | {} is not a ZIM archive |  |
| src-tauri/src/torrent/storage.rs | Engine directory deletion is disabled |  |
| src-tauri/src/torrent/storage.rs | Engine file deletion is disabled |  |
| src-tauri/src/torrent/storage.rs | Kiwix could not open the shared file. Add it to the library again |  |
| src-tauri/src/torrent/storage.rs | Nonzero torrent padding |  |
| src-tauri/src/torrent/storage.rs | Owned torrent staging file was replaced; refusing to resume writes |  |
| src-tauri/src/torrent/storage.rs | Partial copy buffer is too small |  |
| src-tauri/src/torrent/storage.rs | Partial range is outside the file |  |
| src-tauri/src/torrent/storage.rs | Shared library archives are read-only |  |
| src-tauri/src/torrent/storage.rs | Skipping LocalSwarm partial {}: {error} |  |
| src-tauri/src/torrent/storage.rs | Symlink in torrent storage |  |
| src-tauri/src/torrent/storage.rs | The shared file changed while sharing started. Start sharing again |  |
| src-tauri/src/torrent/storage.rs | The shared library archive no longer matches its torrent |  |
| src-tauri/src/torrent/storage.rs | This file cannot be shared because it has no content. Download it again |  |
| src-tauri/src/torrent/storage.rs | This file cannot be shared because its contents are not valid. Download it again |  |
| src-tauri/src/torrent/storage.rs | This file contains a shortcut and cannot be shared. Download it again |  |
| src-tauri/src/torrent/storage.rs | This torrent stores its archives in pieces shared with other files, so Kiwix cannot verify them. Download the archive directly instead |  |
| src-tauri/src/torrent/storage.rs | Torrent archive has no verifiable pieces |  |
| src-tauri/src/torrent/storage.rs | Torrent archive size overflow |  |
| src-tauri/src/torrent/storage.rs | Torrent file path or advertised length changed |  |
| src-tauri/src/torrent/storage.rs | Torrent metadata does not describe the selected library archive |  |
| src-tauri/src/torrent/storage.rs | Torrent metadata is missing a piece hash |  |
| src-tauri/src/torrent/storage.rs | Torrent paths changed or collide on a case-insensitive filesystem |  |
| src-tauri/src/torrent/storage.rs | Torrent piece range overflow |  |
| src-tauri/src/torrent/storage.rs | Torrent publication source was truncated |  |
| src-tauri/src/torrent/storage.rs | Torrent source is missing for its piece receipt |  |
| src-tauri/src/torrent/storage.rs | Torrent storage is not a singly linked regular file |  |
| src-tauri/src/torrent/storage.rs | Unknown torrent file |  |
| src-tauri/src/torrent/web_seed.rs | Torrent attribute is not a byte string |  |
| src-tauri/src/torrent/web_seed.rs | Torrent file has an empty path |  |
| src-tauri/src/torrent/web_seed.rs | Torrent file is missing its path |  |
| src-tauri/src/torrent/web_seed.rs | Torrent file length is negative |  |
| src-tauri/src/torrent/web_seed.rs | Torrent file path contains a non-string |  |
| src-tauri/src/torrent/web_seed.rs | Torrent file path is not a safe relative path |  |
| src-tauri/src/torrent/web_seed.rs | Torrent has no files |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata ended early |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata ended inside a byte string |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata ended inside a dictionary |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata ended inside a list |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata has an invalid byte string |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata has an invalid integer |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata has duplicate dictionary keys |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata has trailing bytes |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata is missing a byte string |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata is missing a dictionary |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata is missing an integer |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata is missing its file list |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata is nested too deeply |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata is not valid bencode |  |
| src-tauri/src/torrent/web_seed.rs | Torrent metadata length overflow |  |
| src-tauri/src/torrent/web_seed.rs | Torrent piece count does not match its size |  |
| src-tauri/src/torrent/web_seed.rs | Torrent piece count is inconsistent |  |
| src-tauri/src/torrent/web_seed.rs | Torrent piece hashes are invalid |  |
| src-tauri/src/torrent/web_seed.rs | Torrent piece length is out of range |  |
| src-tauri/src/torrent/web_seed.rs | Torrent url-list contains a non-string |  |
| src-tauri/src/torrent/web_seed.rs | Torrent url-list has an invalid shape |  |
| src-tauri/src/torrent/web_seed.rs | Web seed URLs cannot contain credentials |  |
| src-tauri/src/torrent/web_seed.rs | Web seed URLs require HTTPS (literal loopback HTTP is allowed for local testing) |  |
| src-tauri/src/torrent/web_seed.rs | Web seed content failed verification at piece {} |  |
| src-tauri/src/torrent/web_seed.rs | Web seed did not provide the complete file |  |
| src-tauri/src/torrent/web_seed.rs | Web seed file handle is missing |  |
| src-tauri/src/torrent/web_seed.rs | Web seed file is not the size the torrent describes |  |
| src-tauri/src/torrent/web_seed.rs | Web seed file length mismatch: expected {}, got {actual} |  |
| src-tauri/src/torrent/web_seed.rs | Web seed file path is not UTF-8 |  |
| src-tauri/src/torrent/web_seed.rs | Web seed file path is not relative |  |
| src-tauri/src/torrent/web_seed.rs | Web seed response exceeded the torrent file length |  |
| src-tauri/src/torrent/web_seed.rs | Web seed response is larger than the torrent file |  |
| src-tauri/src/torrent/web_seed.rs | Web seed returned a short file |  |
| src-tauri/src/torrent/web_seed.rs | Web seed returned an unexpected byte range |  |
| src-tauri/src/torrent/web_seed.rs | attr |  |
| src-tauri/src/torrent/web_seed.rs | pieces |  |
| src-tauri/src/torrent/web_seed.rs | url-list |  |
| src-tauri/src/transfer_policy.rs | A confirmed Wi-Fi or Ethernet route is required for local sharing |  |
| src-tauri/src/transfer_policy.rs | Native LAN binding has no route receipt |  |
| src-tauri/src/transfer_policy.rs | Native bound-route locality reply is invalid |  |
| src-tauri/src/transfer_policy.rs | Native transfer policy is not installed |  |
| src-tauri/src/transfer_policy.rs | The bound Wi-Fi or Ethernet route is no longer available |  |
| src-tauri/src/transfer_policy.rs | bindLocalSocket |  |
| src-tauri/src/transfer_policy.rs | fd |  |
| src-tauri/src/transfer_policy.rs | isLocalPeer |  |
| src-tauri/src/transfer_policy.rs | local |  |
| src-tauri/src/transfer_policy.rs | route |  |
| src-tauri/src/update.rs | Installing an update requires confirmation. |  |
| src-tauri/src/update.rs | KIWIX_UPDATER_PUBKEY |  |
| src-tauri/src/update.rs | KIWIX_UPDATE_EXIT cannot save the profile before installing: {error} |  |
| src-tauri/src/update.rs | Kiwix could not check whether a transfer is in progress. Wait a moment, then try again. |  |
| src-tauri/src/update.rs | Kiwix does not check for updates automatically on a metered connection. Connect to Wi-Fi or allow metered downloads in Settings. |  |
| src-tauri/src/update.rs | The available update changed. Check for updates again. |  |
| src-tauri/src/update.rs | The update key in this build is not valid. Ask whoever supplied Kiwix for an updated build. |  |
| src-tauri/src/update.rs | Unknown update action |  |
| src-tauri/src/update.rs | Updates are not available until Kiwix finishes starting. Try again in a moment. |  |
| src-tauri/src/watch.rs | Monitored folders: cannot watch {}: {error} |  |
| src-tauri/src/watch.rs | Monitored folders: native watching is unavailable, falling back to periodic scans: {error} |  |
| src-tauri/src/windows.rs | A window could not be restored: {error} |  |
| src-tauri/src/windows.rs | Hide Tab Bar |  |
| src-tauri/src/windows.rs | Kiwix could not read the current window groups: {e} |  |
| src-tauri/src/windows.rs | Merge All Windows |  |
| src-tauri/src/windows.rs | Minimize |  |
| src-tauri/src/windows.rs | Move Tab to New Window |  |
| src-tauri/src/windows.rs | Native window menu labels could not be updated |  |
| src-tauri/src/windows.rs | Native window tab groups could not be captured |  |
| src-tauri/src/windows.rs | Native window tab groups could not be restored |  |
| src-tauri/src/windows.rs | Show Next Tab |  |
| src-tauri/src/windows.rs | Show Previous Tab |  |
| src-tauri/src/windows.rs | Show Tab Bar |  |
| src-tauri/src/windows.rs | Window |  |
| src-tauri/src/zim_scheme.rs | // |  |
| src-tauri/src/zim_scheme.rs | Empty zim:// link |  |
| src-tauri/src/zim_scheme.rs | Malformed zim:// link |  |
| src-tauri/src/zim_scheme.rs | Only zim:// links are accepted |  |
| src-tauri/src/zim_scheme.rs | This link is missing its archive. Open the article from your library |  |
| src-tauri/src/zim_scheme.rs | zim |  |
| src-tauri/src/zim_scheme.rs | zim:// article fragment is too long |  |
| src-tauri/src/zim_scheme.rs | zim:// article path contains a backslash |  |
| src-tauri/src/zim_scheme.rs | zim:// article path contains a traversal segment |  |
| src-tauri/src/zim_scheme.rs | zim:// article path contains control characters |  |
| src-tauri/src/zim_scheme.rs | zim:// article path is too long |  |
| src-tauri/src/zim_scheme.rs | zim:// article query is too long |  |
| src-tauri/src/zim_scheme.rs | zim:// article query or fragment contains control characters |  |
| src-tauri/src/zim_scheme.rs | zim:// link is too long |  |
| src-tauri/src/zim_scheme.rs | zim:// link must name an archive and path |  |
| src-tauri/src/zim_scheme.rs | zim:// links cannot carry a port |  |
| src-tauri/src/zim_scheme.rs | zim:// links cannot carry credentials |  |

## Frontend error and notice candidates

| Source | Extracted literal | Vocabulary review candidates |
| --- | --- | --- |
| src/App.tsx | Could not reconcile restored archive editions. Retry restoring the session. |  |
| src/App.tsx | Session flush was cancelled. |  |
| src/api.ts | Article PDF export requires the native application. |  |
| src/api.ts | Choose an app storage location in Settings, or use Add folder to read originals. |  |
| src/api.ts | Disk space confirmation is already registered |  |
| src/api.ts | Download cancelled. |  |
| src/api.ts | Import exceeds the 16 MiB limit. Split the export into smaller files. |  |
| src/api.ts | Leave private or read-only mode before downloading an edition. |  |
| src/api.ts | Search cancelled |  |
| src/api.ts | Search is only available inside the Kiwix window. |  |
| src/api.ts | Search request identity exhausted |  |
| src/api.ts | Use Add files to read originals, or explicitly import a managed copy. |  |
| src/components/Catalog.tsx | Browsing session expired after a privacy transition |  |
| src/components/Catalog.tsx | Online preview requires HTTPS. |  |
| src/components/Catalog.tsx | The catalog page link is invalid. |  |
| src/components/CrashReport.tsx | The report could not be copied. |  |
| src/components/Downloads.tsx | Cancel this edition update? Your original archive remains installed and its files are kept. |  |
| src/components/Downloads.tsx | Roll back to the retained original archive? Reading references move back to that edition; both archive files are kept. |  |
| src/components/Downloads.tsx | The installed archive changed. Check for editions again. |  |
| src/components/Downloads.tsx | This download changed. Close this dialog and try again. |  |
| src/components/Downloads.tsx | This edition decision changed. Check for editions again. |  |
| src/components/Library.tsx | No archive files were found. Nothing was deleted. |  |
| src/components/LocalSwarmControls.tsx | This share changed or is no longer available. Choose the sender again. |  |
| src/components/LocalSwarmControls.tsx | This share is not a supported ZIM archive or split archive. |  |
| src/components/LocalSwarmControls.tsx | Use a shorter device name without control or direction-override characters. |  |
| src/components/Maps.tsx | Clear the map assembly cache? Installed maps will not be removed. |  |
| src/components/Maps.tsx | Download settings changed. Start this map download again. |  |
| src/components/Maps.tsx | Map catalog changed. Reload the map registry before downloading. |  |
| src/components/Maps.tsx | The download or saved map changed. Close this dialog and try again. |  |
| src/components/PrivateReadingSync.tsx | \|\| owned.phase === |  |
| src/components/PrivateReadingSync.tsx | Clear saved sync positions and reset connections |  |
| src/components/PrivateReadingSync.tsx | Disconnect device |  |
| src/components/PrivateReadingSync.tsx | Disconnect this device? It will no longer receive private reading data. Data already saved on either device is kept. |  |
| src/components/PrivateReadingSync.tsx | Forget all saved sync positions and disconnect every device? Reading history, reading list and open tabs are kept. Saved sync position candidates are removed and will not be restored from unchanged history or tabs. Reset your other devices too before reconnecting. |  |
| src/components/PrivateReadingSync.tsx | Reset private sync history |  |
| src/components/PrivateReadingSync.tsx | Reset private sync history and disconnect every device? Your reading list and this device’s reading positions are kept. Positions received from other devices are removed. Reset private sync history on your other devices too before reconnecting. |  |
| src/components/Reader.tsx | Archive storage is waiting for persistence acknowledgement |  |
| src/components/Reader.tsx | Dismiss error |  |
| src/components/Reader.tsx | The archive changed; reopen the article before exporting. |  |
| src/components/Reader.tsx | The article attempted to leave its archive. |  |
| src/components/Reader.tsx | The article changed before export completed. |  |
| src/components/Reader.tsx | The live page could not be captured. |  |
| src/components/SelectedStorageActions.tsx | The managed copies are ready. Some originals were kept; restart Kiwix to finish cleanup. |  |
| src/components/SettingsAutosave.tsx | Enter a complete HTTP or HTTPS URL without embedded credentials. |  |
| src/components/SettingsAutosave.tsx | Enter a font size from 10 to 48. |  |
| src/components/SettingsAutosave.tsx | The download directory or privacy settings changed. Previous downloads have not been moved. |  |
| src/main.tsx | Missing application root |  |
| src/native-speech.ts | Read-aloud did not start. Check the selected voice and device volume, or download the natural voice model. |  |
| src/native-speech.ts | The article did not provide text for read-aloud. |  |
| src/private-sync-api.ts | Private device sync is unavailable in private or read-only mode. |  |
| src/reader-storage.ts | Archive storage rejected an invalid mutation. |  |
| src/reader-storage.ts | Invalid native storage acknowledgement |  |
| src/reader-storage.ts | Kiwix could not finish closing the reader. Close and reopen this window to continue. |  |
| src/reader-storage.ts | The archive did not acknowledge its storage drain. |  |

## Native prompt candidates

| Source | Extracted literal | Vocabulary review candidates |
| --- | --- | --- |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | + rest.substringBefore('/') + |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | ${('a'.code + index / 26).toChar()}${('a'.code + index % 26).toChar()} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | ${MediaStore.MediaColumns.DISPLAY_NAME} LIKE ? |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | ) -> normalized.removePrefix( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | ) -> path.removePrefix( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | , entries).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | , true) \|\| name.equals( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | Android 13 and later only exposes shared-storage files you grant a folder for. Add folder access to scan Downloads or Documents. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | Could not scan ${tree.lastPathSegment ?: tree.authority ?: |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | Folder access expired for ${tree.authority ?: |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | MediaStore could not be searched: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | No folder access has been granted yet. Add folder access to include ZIM files outside app storage. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | Shared storage could not be searched: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | Shared-storage access was denied; only app-visible files were searched. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | Split archive is incomplete or has extra parts; import a copy. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | Stopped after $cap archive candidates; narrow the granted folders. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/DeviceZimScan.kt | This location is not readable in place; import a copy. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | \|\| name.startsWith( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | ') \|\| !value.endsWith(' |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | ) == quicLease, |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | ) \|\| name.startsWith( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | )) nearbyRequire(args.getString( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | , iface != null && args.getString( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | , iface.address.hostAddress).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | , legacySsid).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | , ssid).put( |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android could not start the Wi-Fi hotspot |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android did not publish the active hotspot's network name and password |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android did not release the hotspot: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android refused the hotspot. Allow nearby Wi-Fi access and turn on Location services, then try again. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android refused the hotspot: ${error.message ?: |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android refused the hotspot: ${error.message ?: error.javaClass.simpleName} |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android started a hotspot but did not publish its network name and password |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Android stopped the hotspot before it started |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Hotspot use is blocked by device policy |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | The Wi-Fi hotspot is already starting |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | The Wi-Fi hotspot was stopped before it finished starting |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | This device cannot create a Wi-Fi hotspot |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | Wi-Fi is busy with another mode. Turn off Wi-Fi Direct or tethering and try again. |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | calling startLocalOnlyHotspot |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | onFailed reason=$reason |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | onStarted pending=${pending != null} stopRequested=$stopRequested |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | onStopped closing=$closing |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | quic interface candidates=${candidates.size} current=$current before=$beforeInterfaces client=$clientInterfaces beforeClient=$beforeClientInterfaces |  |
| src-tauri/plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/LocalHotspot.kt | stop active=${reservation != null} pending=${pending != null} |  |
