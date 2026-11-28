# Lane `startup` — v2.1 addendum: bare e-mail addresses

Bare e-mail addresses are now redacted to `[email]`. No existing vector changed, the leak search with e-mail sentinels found nothing, and `crash::` plus host clippy pass.

## The rule (spec step 4, `crash.rs: redact_emails`)
Scan for `@`, then:
- **Local part:** the whole run of `[A-Za-z0-9._%+-]` directly before the `@`. It must not be empty and cannot reach into an earlier replacement.
- **Host:** the whole run of `[A-Za-z0-9.-]` directly after the `@`, minus any `.` or `-` at its very end (sentence punctuation, which stays in the text). It needs at least two labels, none empty, and a last label of ASCII letters only, two or more long.
- If both hold, the local part through the host becomes `[email]`.

Both runs are taken whole, as for opaque tokens. The scan is linear: each run is scanned only by the `@` next to it.

## Position
It is step 4: after URLs, before paths. Paths, opaque tokens, whitespace and length are now steps 5 to 8, and frame lines run steps 1 to 5.
- **After URLs:** `mailto:` is already `[url]`, and the userinfo of `scheme://user@host` is already gone, so URL outcomes do not change.
- **Before paths:** text directly after an address then starts at a boundary. `jane@example.org/Inbox Folder` becomes `[email][path]`; in the other order the `/Inbox Folder` would stay, because the letter before the `/` blocks a path start.
- **Before opaque tokens:** a 40-character local part would otherwise become `[redacted]@host`, leaving the domain.

## Vectors
The fixture now has 215 cases (13 added, none changed, none removed).

| Input | Expected |
|---|---|
| `contact jane.doe+kiwix@mail.example.co.uk for help` | `contact [email] for help` |
| `Jane Doe <jane@example.org>, bob_1%x@sub-1.example.museum.` | `Jane Doe <[email]>, [email].` |
| `user=jane@example.org id=7` | `user=[email] id=7` |
| `JANE@EXAMPLE.ORG` | `[email]` |
| `sent to jane@example.org: bounced` | `sent to [email]: bounced` |
| `mailto:jane@example.org tel:+15551234 jane@example.org` | `[url] [url] [email]` |
| `mailto: jane@example.org` | `[url] [email]` |
| `https://jane@example.org/a` | `https://example.org` |
| `git@github.com:kiwix/app.git` | `[email]:kiwix/app.git` |
| `/home/jane@example.org/My Docs/a.zim` | `[path]` |
| `jane@example.org/Inbox Folder` | `[email][path]` |
| `user@host and me@localhost and a@b.c and pkg@1.2.3 and x@y.z9 stay` | unchanged |
| `@jane and jane@ and a@.org and a@b..org stay` | unchanged |

The existing `@/home/jane doe/x.zim` → `@[path]` and `mailto:jane@example.org tel:+15551234 sent` → `[url] [url] sent` are unchanged.

## Spec
`crash-redaction-spec.md` is updated:
- new step 4, with the reason for its position;
- steps renumbered, and the frame-line text now says steps 1 to 5;
- `[email]` added to the placeholders;
- the "not caught" entry is now "an address whose host has non-ASCII letters or no dot";
- the "removes harmless text" list gains the `git@github.com:…` shape.

## Results
- Leak search with e-mail sentinels (11 new address shapes, including inside paths, after `mailto:`, as URL userinfo, comma-joined and with a trailing dot): `0 leaks in 173850`.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- crash::` → `test result: ok. 10 passed; 0 failed; 0 ignored`. This includes the 215-vector fixture test and the linear-time test, which now covers the e-mail step with two worst-case units.
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 0, `Finished`.
- Not rerun this round: Windows-gnu clippy and the other lane tests. The change is platform-neutral code in `crash.rs` only.

Files changed: `src-tauri/src/crash.rs`, `src-tauri/tests/fixtures/crash-redaction-spec.md`, `src-tauri/tests/fixtures/crash-redaction-cases.json`.
