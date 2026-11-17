# Authenticated loopback API

The desktop application can run without a native window and serve the real
React shell plus a privileged JSON command endpoint on an authenticated
loopback HTTP server. This document describes that implementation as it is in
the source; it is not a stable public API, a remote API, or a promise of
compatibility between releases.

Code references:

- `src-tauri/src/lib.rs::parse_options` and the `options.headless` branch of
  `run_application`.
- `src-tauri/src/server.rs::start_preview` and `preview_request`.
- `src-tauri/src/service.rs::dispatch` / `operation`.
- `src-tauri/src/service/dispatch.rs` (command table).
- `src/api.ts::dispatchOnce` (the bundled shell's own client).

## Starting the server

```sh
kiwix-workbench --headless [--port NUMBER] [--data-dir PATH] \
  [--private] [--read-only] [--article PATH] [ARCHIVE.zim ...]
```

- `--headless` is a desktop build option (`#[cfg(desktop)]`). The mobile entry
  points do not expose it.
- On success the process prints exactly one line to standard output:

  ```text
  KIWIX_READY http://kiwix-shell-<nonce>.localhost:<port>/#token=<token>
  ```

  `<nonce>` and `<token>` are each 32 lowercase hexadecimal characters
  (simple-form UUID v4). The URL always ends with `/#token=<token>`; the token
  is in the **fragment**, after `#`.
- `--port NUMBER` parses as a `u16`. Omitted or `0` means the operating system
  chooses an available port; the printed URL contains the actual port.
- The listener binds `127.0.0.1` only. It is plain HTTP, not TLS.
- `--data-dir PATH` selects the profile to open. Without it
  `directories::ProjectDirs::from("org", "kiwix", "KiwixWorkbench")` supplies
  the platform-specific data directory:

  | Platform | Profile directory |
  | --- | --- |
  | macOS | `~/Library/Application Support/org.kiwix.KiwixWorkbench` |
  | Linux | `$XDG_DATA_HOME/kiwixworkbench`, falling back to `~/.local/share/kiwixworkbench` |
  | Windows | `%APPDATA%\kiwix\KiwixWorkbench\data` |

  The Tauri bundle identifier `org.kiwix.workbench` (used for bundle/application
  identity) is a separate value; it is not the profile directory name on any of
  these platforms. Portable distributions keep their own adjacent `data/` profile
  and reject a conflicting `--data-dir`.
- Positional arguments are archive paths to open before serving. They are
  optional: with no archives the server still starts and the library is empty.
  `--article PATH` selects an article location and requires exactly one
  positional archive.
- `--private` keeps browsing/session/archive-storage changes in memory instead
  of writing the profile. `--read-only` opens an existing initialized profile
  and prohibits application-managed filesystem mutation; archives not already
  registered in that profile cannot be added.
- The process serves until interrupted. `Ctrl+C` cancels the maintenance task
  and shuts the service down.
- On Unix, if the profile is already owned by another instance, the launch is
  forwarded to that owner and the headless process exits 0 **without** printing
  a `KIWIX_READY` line. Other command-line and startup failures go to standard
  error as `Kiwix: <message>` and exit 1; for example, a `--port` that is
  already in use fails the bind. Windows profile contention was not exercised
  while preparing this document.

The same listener also serves the embedded production shell (`dist/` assets) at
`/` for the printed URL. Programmatic clients only need `/api`.

## Authentication and required headers

Every request, for any path, must satisfy these checks; failures never reach
command dispatch.

| Header | Required | Rule |
| --- | --- | --- |
| `Host` | always | Exactly one value, byte-for-byte equal to `kiwix-shell-<nonce>.localhost:<port>` from the printed URL. A mismatch or duplicate is `403` with `Unknown preview host`. |
| `Origin` | `POST /api` | Exactly one value, equal to `http://kiwix-shell-<nonce>.localhost:<port>` (the printed origin, no trailing slash). Missing on `/api` is `403` (`Preview origin required`); any foreign, malformed, opaque or duplicate value is `403` (`Foreign preview origin`). On other paths the header may be omitted, but if present it must match. |
| `Authorization` | `POST /api` | Exactly one value, exactly `Bearer <token>` using the fragment token. Missing, wrong, duplicated, or any method other than `POST` is `401` (`Preview authorization required`). |
| `Content-Type` | recommended | The bundled shell sends `application/json`. The body is parsed as JSON regardless of the declared type. |

Service workers are rejected server-wide: a request carrying a `Service-Worker`
header, or `Sec-Fetch-Dest: serviceworker`, gets `403`
(`Shell service workers are not supported`). The served shell's CSP also sets
`worker-src 'none'`.

The token is a per-launch capability. The bundled shell reads it from the URL
fragment, stores it in `sessionStorage`, and removes the fragment with
`history.replaceState`, so it is not part of the HTTP request or `Referer`. A
new launch generates a new hostname, origin and token.

> **Treat the token as a secret.** It is a bearer credential for every command
> in the dispatch table, including mutating ones. There is no command allowlist
> and no separate privilege level: whoever has the token — plus the printed host
> and origin, which appear in the same `KIWIX_READY` line — can call anything the
> native application can. Do not paste the `KIWIX_READY` line into logs, shell
> history, issue reports or chat; do not pass the token on a command line, where
> `ps` and shell history expose it; and start a new launch if it may have leaked.
> The examples below keep it out of `argv` and out of shell history.

### Reaching the hostname

The server matches the `Host` header, not the socket's destination address, so
a client may connect to `127.0.0.1:<port>` and set the expected `Host` header
explicitly (this is what the examples below do). Relying on
`kiwix-shell-<nonce>.localhost` resolving to loopback depends on the OS
resolver; if it does not resolve, use the explicit `Host` header or a
`--resolve`-style DNS override. `Host` must include the port when the printed
host does.

## Request and response format

`POST /api` accepts:

```json
{ "command": "<command name>", "args": { } }
```

- `command` must be a string. A body that does not parse is `400` with the
  plain-text body `Invalid command JSON`; a parsed body without a string
  `command` is `400` with the plain-text body `Missing command`. Neither is
  JSON.
- `args` is a JSON object of command-specific fields. When the field is absent
  it is dispatched as `{}`; when present it is forwarded to dispatch unchanged,
  with two exceptions applied before dispatch: a supplied `__windowLabel` is
  removed, and a supplied `parentOrigin` that is not the shell origin is
  rejected with `400` and the plain-text body `Untrusted reader parent origin`.
  Commands therefore always run against the headless shell's single default
  window, `main`.
- Request bodies are limited to 32 MiB (`DefaultBodyLimit`). A larger body is
  rejected with `413` and the plain-text body `Failed to buffer the request
  body`. The limit is enforced while the body is buffered, before the handler
  runs, so a too-large body yields `413` even when the host, origin or token
  would also fail.

Responses:

| Status | Body | Meaning |
| --- | --- | --- |
| `200` | `{"result": <any JSON>}` | The command completed. Commands that return nothing produce `{"result":null}`. |
| `400` | `text/plain` (`Invalid command JSON`, `Missing command`, `Untrusted reader parent origin`) | The request body was not JSON, had no string `command`, or supplied a `parentOrigin` that is not the shell origin. |
| `400` | `{"error":"<sentence>"}` | Dispatch rejected the command or it failed: unknown command, bad or missing argument, or operational failure. Errors are the same plain-language sentences the native shell shows. |
| `401` | `text/plain` | `Preview authorization required`: missing/incorrect bearer token, or a non-`POST` request to `/api`. |
| `403` | `text/plain` | Host, origin or service-worker rejection, as described above. |
| `404` | `text/plain` | `GET`/`HEAD` of an unknown embedded shell asset. |
| `405` | `text/plain` | Any method other than `GET`/`HEAD` on a non-`/api` path (`Read-only application assets`). |
| `413` | `text/plain` | `Failed to buffer the request body`: the request exceeded the 32 MiB body limit, checked before dispatch. |

`GET` and `HEAD` on non-`/api` paths serve the embedded shell with
`Cache-Control: no-store`. `POST /api` has no CORS support: no
`Access-Control-Allow-*` headers are emitted, and `OPTIONS` is not a supported
method (`401` when the origin check passes, `403` when it does not). Only the
shell origin, which cannot be an archive origin, is accepted.

The `/api` route has no command allowlist: it reaches the same native command
table as the windowed application, including mutating commands. Everything is
gated by the host/origin/token checks above, not by a read-only command list.

## Command reference

The commands below are the read/navigation surface of the same dispatch table.
`privacyGeneration` is the integer from `snapshot`
(`result.privacyGeneration`); when required, a missing or stale value fails
with a "session expired" error. Book IDs and installation IDs come from
`snapshot` (`result.books[].id` and `result.books[].installationId`).

| Command | Arguments | Returns |
| --- | --- | --- |
| `snapshot` | none | Full shell state object; see below. |
| `open_article` | `bookId`, `parentOrigin`; optional `path`, `freshOrigin` | Reader session object with a URL on a fresh archive origin. |
| `random_article` | same as `open_article`; `path` ignored | Same, resolving a random article. |
| `article_preview` | `bookId`, `path` | `{title, path, excerpt, mime, thumbnail}`. |
| `search` | search handshake plus `privacyGeneration`, `query`; optional `bookIds`, `mode`, `offset`, `limit` | Search page `{hits, total, hasMore, errors, corrections, spellingStatus}`. |
| `suggest` | search handshake plus `privacyGeneration`, `prefix`, `bookIds`; optional `limit`, `token` | `{suggestions, hasMore, nextToken, errors}`. |
| `downloads` | none | Array of current transfer job rows. |
| `catalog` | `url`, `locale`, `privacyGeneration` | Catalog page `{entries, next, total, cache}`. |
| `catalog_facets` | `url`, `locale`, `privacyGeneration` | `{languages, categories, cache}`. |
| `book_illustration` | `id` | Base64 data URI string, or `null`. |
| `check_integrity` | `id`, `installationId`, `privacyGeneration` | Integrity job object. |
| `integrity_status` | `privacyGeneration` | `{job, books}`. |
| `cancel_integrity` | `privacyGeneration` | `null`. |
| `save_session` | `privacyGeneration`; optional `tabs` (missing deserializes as `[]`, which clears), `activeTab`, `activeSurface`, `referenceRevision` | `null`. |
| `record_visit` | `privacyGeneration`, `create`, `bookId`, `path`, `title`; optional `scroll` | `null`. |

The complete dispatch table includes many more desktop commands (imports,
settings, downloads, sharing, maps, exports, diagnostics). Mobile-only commands
are compiled only into mobile builds and are not present in the desktop
headless binary. Undocumented commands are not a supported client surface:
the endpoint is loopback-only and the token is a local capability.

### snapshot

```json
{ "command": "snapshot", "args": {} }
```

Returns an object with these keys:

`books`, `bookmarks`, `history`, `recentSearches`, `tabs`, `activeTab`,
`activeSurface`, `sessionRestored`, `settings`, `sharing`, `launchLocations`,
`notifications`, `privacyGeneration`, `privacyPolicyGeneration`,
`referenceRevision`, `referenceTransitions`, `startupWarnings`.

`books` entries include `id`, `title`, `installationId`, `path`, `available`,
`mainPath` (an encoded archive location) and archive metadata.
`settings` uses camelCase fields such as `historyEnabled`, `privateMode`,
`readOnly`, `spellingEnabled` and `showSearchSnippets`.

`snapshot` is a state read but not a pure one: it reconciles completed map
downloads, reports queued launch locations for `main`, queues the native
notification launch state on macOS, reports the native notification capability,
and reports sharing status.

### open_article

```json
{
  "command": "open_article",
  "args": {
    "bookId": "<book-id>",
    "path": "A/Article",
    "parentOrigin": "http://kiwix-shell-<nonce>.localhost:<port>"
  }
}
```

- `bookId` (required): an archive in the library.
- `parentOrigin` (required): the shell's **own** origin, exactly as printed;
  any other value is rejected as an untrusted reader parent (`400`, plain text
  `Untrusted reader parent origin`). It is not the archive origin:
  `open_article` creates the separate archive origin and returns it in `origin`.
  The API caller gets the shell origin from the printed URL.
- `path` (optional): an encoded archive location (path plus optional query
  and fragment). Omitted or empty opens the archive's main page; when there is
  no main page the reader falls back to a random article and reports
  `noMainPage`. Values returned by `snapshot` (`mainPath`) and
  `article_preview` are already encoded; a client constructing a location must
  percent-encode its path part, leaving an intentional `?`/`#` delimiter
  readable.
- `freshOrigin` (optional boolean, default `false`): discard the existing
  reader/server for this book and start a fresh archive origin.

Returns:

```json
{
  "bookId": "...",
  "path": "<encoded resolved location>",
  "title": "...",
  "mime": "...",
  "missing": false,
  "noMainPage": false,
  "url": "http://<archive-id>-<nonce16>.localhost:<archive-port>/<path>",
  "origin": "http://<archive-id>-<nonce16>.localhost:<archive-port>",
  "baseUrl": "...",
  "retirementUrl": "...",
  "retirementProof": "..."
}
```

`missing` is true when the requested path did not exist and the main page (or
random article) was returned instead; the returned `path` then does not carry
the original query/fragment. `random_article` takes the same arguments but
always resolves a random entry. The archive origin is a separate loopback
server bound to `127.0.0.1`; it cannot call `/api`.

### article_preview

```json
{ "command": "article_preview",
  "args": { "bookId": "<book-id>", "path": "A/Article" } }
```

- `bookId` (required), `path` (required, max 16,384 bytes).
- For HTML/text entries, at most 256 KiB is read and a bounded excerpt is
  produced; HTML is parsed with a reduced CPU budget and is never executed.
- Returns `{"title","path","excerpt","mime","thumbnail"}`. `path` is the
  encoded resolved location including any original query/fragment, and
  `thumbnail` is `null` when none was found or the entry is not HTML/text-readable.

### Search and suggestions

Search and suggestion calls require an ownership handshake per window. The
ownership commands `search_session`, `search_owner`, `search_release` and
`cancel_search` are routed in `src-tauri/src/service.rs::operation` before the
main match; the remaining work is dispatched by `search` and `suggest` in
`dispatch.rs`.

1. `search_session` with `{}` returns `{"sessionId":"..."}`. A new session
   supersedes the previous one for `main`.
2. `search_owner` with `{"sessionId":"..."}` returns `{"ownerId":"..."}`.
3. Every `search`/`suggest` call then includes `sessionId`, `ownerId`, a
   positive `requestId` that is strictly greater than the previous one for that
   owner, and `privacyGeneration`.
4. Optional cleanup: `search_release` with `{sessionId, ownerId}` (returns
   `{"status": ...}`), or `cancel_search` with `{sessionId, ownerId, requestId}`.

#### search

```json
{
  "command": "search",
  "args": {
    "sessionId": "...", "ownerId": "...", "requestId": 1,
    "privacyGeneration": 0,
    "query": "wikipedia",
    "bookIds": ["<book-id>"],
    "mode": "combined",
    "offset": 0,
    "limit": 50
  }
}
```

- `query` required (trimmed). Empty query returns an empty page without
  searching.
- `bookIds` optional; when omitted/empty, all available library books are
  searched, otherwise only the listed available books.
- `mode` is `combined` (default), `title`, or `fulltext`.
- `offset` defaults to `0`; `limit` defaults to `50` and is clamped to `1..=200`.
- Returns `{hits, total, hasMore, errors, corrections, spellingStatus}`.
  `errors` is a per-book array (`{bookId, error}`) and does not fail the whole
  request. `corrections`/`spellingStatus` are populated only when there are no
  hits and spelling is enabled outside private/read-only mode.

#### suggest

```json
{
  "command": "suggest",
  "args": {
    "sessionId": "...", "ownerId": "...", "requestId": 2,
    "privacyGeneration": 0,
    "prefix": "wiki",
    "bookIds": ["<book-id>"],
    "limit": 15
  }
}
```

- `prefix` required (max 512 bytes).
- `bookIds` is required and must be non-empty; suggestions only cover the
  listed available books.
- `limit` defaults to `15`, clamped to `1..=50`.
- `token` is the previous response's `nextToken` for continuation; a token that
  does not match the prefix and resolved scope is refused.
- Returns `{suggestions:[{bookId, path, title}], hasMore, nextToken, errors}`.

### downloads

```json
{ "command": "downloads", "args": {} }
```

Returns a JSON array of transfer job rows. In normal mode it reconciles
transfer state and commits ready imports before listing; private and read-only
launches return the saved rows without mutating state. Row fields are defined
by the HTTP download, BitTorrent and LocalSwarm managers and vary by transport;
this document does not enumerate them.

### catalog and catalog_facets

```json
{ "command": "catalog",
  "args": { "url": "https://library.kiwix.org/catalog/v2/entries?lang=en",
            "locale": "en", "privacyGeneration": 0 } }
```

```json
{ "command": "catalog_facets",
  "args": { "url": "...", "locale": "en", "privacyGeneration": 0 } }
```

- `url` must be an HTTPS feed URL, `locale` is passed as `Accept-Language`, and
  `privacyGeneration` must be current.
- `catalog` returns `{entries, next, total, cache}` where `cache` is
  `{status, stale, stored, error}` and `status` is `network`, `revalidated` or
  `offline`. An offline result can still carry cached entries, with the failure
  in `cache.error`.
- `catalog_facets` returns `{languages, categories, cache}`; each facet is
  `{value, label, count}`.
- These commands perform network access. They can fail, or return an offline
  cached representation, depending on connectivity and cache state.

### book_illustration

```json
{ "command": "book_illustration", "args": { "id": "<book-id>" } }
```

Returns a `data:<mime>;base64,...` string for a PNG, JPEG, WebP or GIF
illustration, or `null` when the archive has none or the MIME is not one of
those types.

### Integrity

```json
{ "command": "check_integrity",
  "args": { "id": "<book-id>", "installationId": "<installation-id>",
            "privacyGeneration": 0 } }
```

`check_integrity` validates one library book and returns the started job
object. `integrity_status` (no arguments beyond `privacyGeneration`) returns
`{job, books}`; `cancel_integrity` cancels and waits, returning `null`.

### save_session

```json
{
  "command": "save_session",
  "args": {
    "privacyGeneration": 0,
    "tabs": [
      { "id": "tab-1", "bookId": "<book-id>", "path": "A/Article",
        "title": "Article", "scroll": 0.0 }
    ],
    "activeTab": "tab-1",
    "activeSurface": "reader"
  }
}
```

- `privacyGeneration` is required. `tabs` is optional: a missing `tabs` field
  deserializes as `[]`, exactly like an explicit empty array. (An explicit
  `"tabs": null` is a `400` command error, not an empty array.) That means
  omitting `tabs` can clear the window's saved session — the tabs and active tab
  for the window, and for `main` the top-level `tabs`. The one exception is a
  window that was just created by explicit session restore: while that window is
  still pending and the profile holds non-empty saved tabs, an empty save is
  ignored so the restored tabs survive until hydration. If you want to keep the
  current session, send the tabs explicitly.
- When present, `tabs` is an array of at most 200 tab objects (`id`, optional
  `bookId`, `path`, `title`, `scroll`, optional `back`/`forward` location
  arrays); more than 200 is a `400` command error (`Too many tabs`).
- `activeSurface` defaults to `library` and must be one of `library`,
  `catalog`, `downloads`, `nearby`, `maps`, `bookmarks`, `history`, `settings`,
  `reader`.
- `referenceRevision` (optional integer, default `0`) is used by the shell for
  archive-edition transitions.
- Returns `null`. In strict read-only mode the command returns `null` without
  writing. In private mode the update stays in the in-memory session.
- The restore guard above exists because a window created by explicit session
  restore may report empty tabs before its snapshot arrives.

### record_visit

```json
{
  "command": "record_visit",
  "args": {
    "privacyGeneration": 0,
    "create": true,
    "bookId": "<book-id>",
    "path": "A/Article",
    "title": "Article",
    "scroll": 0.25
  }
}
```

- All of `privacyGeneration`, `create` (boolean), `bookId`, `path`, `title` are
  required; `scroll` is optional and defaults to `0`.
- `create: true` inserts a history entry and drops older entries for the same
  archive and normalized location; `create: false` updates the newest existing
  entry for that archive/location instead.
- Returns `null`. It is a no-op returning `null` when the profile is in private
  mode, when the launch is read-only (the top-level dispatch returns `null`),
  or when `settings.historyEnabled` is false. New entries are capped at 10,000.

## curl examples

Set these values from the `KIWIX_READY` line. Do not retype the token as a
literal argument: parse the line into variables once, keep them unexported, and
pass them to `curl` through a config stream so they never appear in `argv`
(visible to `ps`) or in shell history. The snippets use `bash`/`zsh`/`ksh`
syntax; adapt them for other shells.

```sh
# In a second terminal, paste the KIWIX_READY line once at the prompt.
# A portable prompt keeps `-p` (bash-only) out of the snippet: zsh and ksh use
# `-p` for coprocess input. `read -s` disables echo, so the token is neither
# printed nor saved in history.
printf '%s' 'KIWIX_READY line: ' >&2
IFS= read -rs READY
SHELL_URL=${READY#KIWIX_READY }
TOKEN=${SHELL_URL##*#token=}
SHELL_ORIGIN=${SHELL_URL%%/#*}
SHELL_HOST=${SHELL_ORIGIN#http://}
SHELL_PORT=${SHELL_HOST##*:}
API_URL="http://127.0.0.1:$SHELL_PORT/api"
```

If your terminal still echoes a bracketed paste, clear its scrollback after the
prompt returns, and prefer a 0600 file that you delete over a world-readable
one if you must stage the line somewhere.

`curl --config -` reads the following here-document from stdin; the expanded
token is never a command-line argument and never written to disk. The request
body travels the same way, so `parentOrigin` stays out of `argv` too.

```sh
# Snapshot: library, settings, privacyGeneration, launch locations, ...
curl --silent --show-error --config - <<EOF
url = "$API_URL"
request = "POST"
header = "Host: $SHELL_HOST"
header = "Origin: $SHELL_ORIGIN"
header = "Authorization: Bearer $TOKEN"
header = "Content-Type: application/json"
data = "{\"command\":\"snapshot\",\"args\":{}}"
EOF
```

```sh
# Open an article. parentOrigin is the shell origin, not the archive origin;
# the response's "origin" is the separate archive origin the reader will use.
curl --silent --show-error --config - <<EOF
url = "$API_URL"
request = "POST"
header = "Host: $SHELL_HOST"
header = "Origin: $SHELL_ORIGIN"
header = "Authorization: Bearer $TOKEN"
header = "Content-Type: application/json"
data = "{\"command\":\"open_article\",\"args\":{\"bookId\":\"<book-id>\",\"path\":\"A/Article\",\"parentOrigin\":\"$SHELL_ORIGIN\"}}"
EOF
```

Inside a `curl` config file, a double-quoted value ends at the next unescaped
`"`, so the JSON body's own quotes are backslash-escaped. The shell still
expands `$SHELL_ORIGIN` in the here-document. A `data` value starting with `@`
would be read as a filename; this one does not.

Connecting to `127.0.0.1` while overriding the `Host` header is intentional:
the server validates the header, not the connection address. If
`kiwix-shell-<nonce>.localhost` resolves to loopback on the machine, the URL
can be used directly instead, with the header values unchanged.

Notes:

- Do not `export TOKEN`, and `unset READY TOKEN SHELL_URL SHELL_ORIGIN SHELL_HOST API_URL`
  when finished. `READY` still contains the full token-bearing line; same-user
  processes can read an exported variable from a child's environment, and some
  shells echo commands under `set -x`.
- If a token does reach a file, terminal scrollback or a process listing,
  restart the headless process: the old token stops working with the old launch.
- The body passed to `--data` is not secret; the token and the printed host are.

## Security boundaries

- **Loopback only.** Both the shell/API server and per-archive reader servers
  bind `127.0.0.1`. There is no LAN listener and no remote authentication
  story; do not reverse-proxy or publish this endpoint.
- **Per-launch capability.** A fresh random `kiwix-shell-<nonce>.localhost`
  hostname, a fresh origin and a fresh bearer token are generated at every
  launch. Reusing an old loopback port cannot inherit an archive origin.
- **Exact Host and Origin.** Requests must name the printed host and, on
  `/api`, the printed origin. Duplicate headers are rejected. This blocks DNS
  rebinding and cross-origin browser calls; there is no CORS allowance.
- **Bearer token.** `/api` requires exactly one `Authorization: Bearer`
  header value equal to the fragment token, and `POST`. The token is delivered
  in the URL fragment so it is never sent as part of an HTTP request line or
  `Referer`; the shell strips it from the address bar after storing it in
  sessionStorage.
- **Service workers refused.** `Service-Worker` and
  `Sec-Fetch-Dest: serviceworker` requests are rejected, and the shell CSP uses
  `worker-src 'none'`.
- **Archive origins are not the shell.** The shell names its **own** origin as
  `parentOrigin`; `open_article` then returns a *separate* archive origin
  (`<archive-id>-<nonce>.localhost`) for the reader. Archive pages cannot reach
  `/api` because their origin is not the shell origin, and the shell origin is
  the only value accepted as `parentOrigin`.
- **Read-only and private modes.** `--read-only` requires an initialized
  profile, blocks application-managed mutations, and makes
  `save_session`/`record_visit`/`record_search` return `null`. `--private` keeps
  browsing/session changes in memory and `record_visit` is a no-op. Neither
  mode is an anonymity guarantee; OS and webview caches are outside app control.
- **No telemetry.** The application contains no telemetry or usage analytics.
  Diagnostics are opt-in and local. Most network access is requested by an
  operation: catalog fetches, downloads, explicit sharing, `update_check`. One
  exception: the **windowed** application starts a background update check at
  launch, at most once per 24 hours, which fetches update metadata and never
  downloads or installs anything. It is skipped in private/read-only mode, in
  the sandboxed macOS build, and when the update channel is not configured. The
  **headless** process does not run that windowed setup path, so it never starts
  the daily check. Headless maintenance still reconciles transfer state, and a
  transfer started by the API (an HTTP download or torrent) is not stopped by
  headless mode, so it continues to use the network.
- **Bounded input.** The `/api` body limit is 32 MiB. Article previews read at
  most 256 KiB, sessions are limited to 200 tabs, suggestions to a 512-byte
  prefix, and history to 10,000 entries.

## Platform support and limitations

- Headless mode is compiled only for desktop targets (`#[cfg(desktop)]`):
  macOS, Windows and Linux. iOS and Android use their mobile entry points and
  do not expose `--headless`, `--port`, or a loopback `/api`; mobile-only
  commands are reachable through native dispatch, not this server.
- Sandboxed macOS bundles keep the profile in the app container, and previously
  granted folder access is restored at startup; folders that were never granted
  cannot be read by a headless launch. Windows runtime behavior was not
  exercised while preparing this document; the checks above are source-level.
- The server is plain HTTP on loopback. It has no TLS, no CORS, no OPTIONS
  handling, no streaming responses and no API versioning.
- Command semantics are the native ones. Some commands are window-scoped,
  perform background maintenance, require current privacy state, refuse in
  private/read-only mode, or depend on network availability. A dispatched
  command's `400` carries a human-readable reason in JSON; the request-level
  `400`/`413` bodies are plain text (`Invalid command JSON`, `Missing command`,
  `Untrusted reader parent origin`, `Failed to buffer the request body`).
  Clients should not match on message text.
- `snapshot` and `open_article` have side effects described above. Polling
  `downloads` is the documented way to observe transfer progress, but its row
  schema is not a stable contract.
- The printed `.localhost` name may not resolve on every OS configuration; the
  explicit `Host` header against `127.0.0.1` is the portable approach.
- This document describes the source references listed at the top; it is not a
  substitute for reading them at a given revision. Deliberately undocumented:
  exact transfer-row fields, the precise offline-cache eviction policy, and the
  behavior of commands outside the table above.
