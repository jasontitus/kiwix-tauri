## Verdict for lane startup, round 3 and v2.1: only non-blocking findings. Part A is closed. The spec is not final yet: two leaks (R3-B1, R3-B2) should be fixed in it before the ports pin it.

Both fixes only add behaviour. I prototyped them in a scratch copy: all 215 existing vectors still pass, so no existing expected value changes. The ports would only gain the new vectors listed below.

What I ran:
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- crash:: quit:: launch_tests instance::` → `test result: ok. 37 passed; 0 failed; … 1472 filtered out`.
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 0, `Finished`.
- A scratch copy of `crash.rs:419-1067` (`.../adv-startup/redact3`) reproduces 215 of 215 vectors. I ran it against my 66 round-2 probes, 52 new shapes, timing inputs, and the prototype fixes.
- I did not re-run Windows-gnu clippy.

### R3-B1: a `://` not preceded by a kept scheme protects the path after it
- **Severity:** Should-fix, in the spec before the ports pin it.
- **Evidence:** reproduced.
- **Location:**
  - spec §2 step 5, "Exception";
  - `crash.rs` `path_at`, the `first`/`second` checks.
- **Leaking inputs:** each comes out of the pipeline unchanged.
  - `1://home/jane doe/a.zim failed`
  - `relative URL ://Users/Jane Doe/Private`
  - `_://Users/Jane Doe/Private`
- **Causal path:**
  1. Step 3 finds no scheme letter, so it does not touch these.
  2. Step 5 then exempts both slashes of any `://`, so the path that follows is never redacted.
- **Smallest fix:** apply the exception only when the scalar before `://` is an ASCII letter. After step 3, that is the case only for a kept `http`/`https` URL.
- **New vectors**, from my prototype:
  - `1://home/jane doe/a.zim failed` → `1:[path]`
  - `relative URL ://Users/Jane Doe/Private` → `relative URL :[path]`
  - `_://Users/Jane Doe/Private` → `_:[path]`

### R3-B2: the most common secret-key names are still missing
- **Severity:** Should-fix.
- **Evidence:** reproduced.
- **Location:** spec step 2 keyword list; `crash.rs` `SECRET_KEYS`.
- **Leaking input:** `AWS_SECRET_ACCESS_KEY=abcd1234 secret_key=hunter2 SECRET_KEY=hunter2 access_key=AKIA1` passes through unchanged.
- **Causal path:** `secret` is followed by `_`, so the right-side rule rejects it, and `key` is not a keyword.
- **Smallest fix:** add `secret_access_key`, `secret_key`, `secret-key`, `secretkey`, `access_key`, `access-key` and `accesskey`, placed before `secret`.
- **New vector:** `AWS_SECRET_ACCESS_KEY=abcd1234 secret_key=hunter2 SECRET_KEY=hunter2 access_key=AKIA1 secretKey=x accessKey=y` → `AWS_SECRET_ACCESS_KEY=[redacted] secret_key=[redacted] SECRET_KEY=[redacted] access_key=[redacted] secretKey=[redacted] accessKey=[redacted]`.

### R3-B3: residuals the spec does not list (Nit; owner decides whether to list or fix)
- **Evidence:** reproduced. Each of these inputs comes out of the pipeline unchanged:
  - **A keyword followed by a camelCase suffix:** `accessTokenValue=abc`, `authTokenString=def`, `passwordHash=ghi`. The right-side rule rejects the uppercase letter that follows.
  - **`~user` paths:** `~jane/Documents/Private Notes/a.zim`.
  - **A rooted path whose first component has a space:** `\Program Files\Kiwix\data\a.zim`. The rooted rule needs a second `\` before the first space.
  - **A drive-relative path:** `D:Users\Jane Doe`.
  - **Personal or secret-bearing host names** in kept URLs:
    - `https://jane-doe.github.io/a` → `https://jane-doe.github.io`;
    - `https://jane-private-medical.s3.amazonaws.com/x` → `https://jane-private-medical.s3.amazonaws.com`.

    §4 should say plainly that the host of an external URL is kept.
  - **`git@github.com:janedoe/private-notes.git`** → `[email]:janedoe/private-notes.git`. §5 lists this under "removes harmless text", but it keeps a user name and a repository name. Move it to §4.
  - **E-mail addresses** with a punycode TLD (`jane@example.xn--p1ai`) or a quoted local part (`"jane doe"@example.org`).

### R3-B4: the spec read as a porter would
I found no text that contradicts the code or the vectors. Specifically:
- the left/right rules and `\"` key quotes;
- the `=>`, `,` and no-separator cases;
- wrapper fall-back and value form order;
- the `'` "last on the line, else line end" rule;
- the scheme front-trim and replaced-range limits;
- the authority end (`min` with token end);
- classify steps 2–8, including `:0443` kept and `:00080` dropped;
- the e-mail trim and "go on there";
- the rooted rule;
- the step 1 partial-token rule;
- step 7 dropping empty pieces.

The only change needed is R3-B1's narrower exception. One porting note: the e-mail step and the URL scheme scan both need a "previous replacement end" kept in input coordinates. The spec says this for both; porters should keep it per step.

### Checks that came out clean
- **Leaks:** every round-2 leak is closed. That covers:
  - escaped JSON keys, tuple and `Pair` forms, camelCase;
  - the added keys, `Some(`/`Optional(` quoted values;
  - NBSP, NEL, CRLF and U+2028 around separators;
  - `=>`, every B4 boundary shape, and `\Device\…`/`\Users\…`;
  - 127.x loopback spellings and LAN or IP literal URLs (now `[url]`);
  - sharing, catalog and nearby URLs;
  - `.local` hosts;
  - `+https://` and `1.http://`;
  - e-mail addresses inside paths, after `mailto:`, and as userinfo.

  I found no new leak in:
  - backslash authorities or `#`/`?` with raw spaces;
  - a bad or empty port, or an unterminated `[`;
  - `wss://`, punycode hosts, `http:/x` and `http:\\x`;
  - BOM, soft hyphen and RLO boundaries;
  - `\\?\UNC\…`.
- **The URL rule:** the textual classify is portable. The private-suffix list only turns things into placeholders and never keeps more, so it is safe. `.home` and `.internal` are owner choices.
- **Kept URL with a path consumes its clause:** safe; it costs words, for example `see https://example.org/x and http://example.org` → `see https://example.org`.
- **camelCase digit hump:** fine.
- **B4 over-redaction:** matches §5.
- **E-mail step position:** after URLs and before paths is correct. `jane+tag@example.co.uk/Inbox Folder/Private` → `mail [email][path]`.
- **B5.6 partial-token drop:** a cut can only drop text after a keyword, so it cannot expose a value. A cut inside a reader host drops the whole token.
- **Linear time, measured** in a release build of the scratch copy. These are complete inputs, so the step-1 bound does not shorten them.

  | 32,768 scalars of | Time |
  |---|---|
  | `a://b<` (round 2: 98 ms) | 2.6 ms |
  | `file:x<` (round 2: 93 ms) | 2.0 ms |
  | `password="" ` (round 2: 40 ms) | 1.4 ms |
  | 24 other adversarial units (rooted `\a`, `@`/`.` runs, wrappers, `'`, `\"`, `[`, kept URLs) | 2.6 ms or less |

  By code reading, every backward or forward scan is bounded by text that is consumed or that belongs to its own candidate.

### Part A
- **Unshown-dialog rule: closed.**
  - Retry and Quit Without Saving always count as real answers, so nothing in this path can discard state.
  - The backoff (1, 2, 4 … 60 s) runs with `exiting` held, and requests held during the wait are honoured.
  - The counter wrapping does nothing harmful.
  - Nit 1: a main thread busier than 50 ms before the dialog shows makes an undisplayable dialog count as shown. With no window, that becomes an unthrottled loop of roughly 20 retries a second instead of the backoff. It is not a CPU spin, but it is a log line per attempt.
  - Nit 2: an undisplayable dialog after a failed retirement, with a window still open, leaves the shells frozen showing "Quitting…" and nothing on screen. That is the existing Retirement-Stay behaviour.
- **Unconditional stderr in `show_profile_busy` (`instance.rs:129-134`): closed.**
- **§1.1:** the new row matches the code.

### Lane status
Part A, round-1 items R1–R7, and B1–B6 are closed. The spec becomes final once R3-B1 and R3-B2 are adopted, with the four vectors above added, and the §4/§5 wording in R3-B3 is fixed. I changed no expected vector.

### Not verified
- No dialog was shown on any OS.
- The macOS code was not compiled.
- The port notes on Kotlin and Swift come from documented runtime behaviour; I ran no port.