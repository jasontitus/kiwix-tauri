## Startup lane, round 4 (spec v2.2): spec final, lane closed

No blocking or should-fix findings. I changed no expected vector.

**What I checked**
- **Fixture:** none of the existing 215 expected values changed. I compared the round-3 file with the current one: no case missing, none changed, and the first 215 entries are identical in order. There are 13 new cases, 228 in total.
- **Tests:** `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- crash:: quit:: launch_tests` → `test result: ok. 32 passed; 0 failed; … 1480 filtered out`.
  - I did not re-run host clippy; the implementer reports exit 0.
- **Scratch copy:** I copied the current `crash.rs` pipeline (from `file_name` up to the test module) into `.../adv-startup/redact4`. It reproduces 228 of 228 vectors.

**The new rules, probed in the scratch copy**
- **camelCase suffix, no prose over-match.** These all stay unchanged: `Tokenizer failed, Tokenizer=abc, tokenizer: x, Passport: x, Pinyin: y, Authority: z, Authored=1`. Also unchanged:
  - `TOKENValue=1`, because the keyword's last letter must be lowercase;
  - `TokenStream::new()`, because `::` is not a separator;
  - `TokenValue abc`, `BearerToken abc` and `authorizationHeader Basic abc`, because a suffix rules out the no-separator form.
- **camelCase suffix, caught.** These values are now redacted:
  - `passwordHash_v2=x`, `apiKeyID=y`, `pinCodeV2: 1234`;
  - `"accessTokenValue":"abc def"` and the escaped form `\"passwordHash\":\"x y\"`.
- **Accepted over-redaction**, as §5 lists: `SecretString: …`, `PassThrough: …`, `AuthMethod: …` and `tokenType: …` lose their value.
- **Residual (Nit, not a finding):** `accessToken_value=abc` and `refreshToken_v2=def` still pass. The suffix must start with an uppercase letter, so a `_` after the hump is rejected. This falls under §4's "a form the left and right sides reject"; it could be named there explicitly.
- **`~user` paths:**
  - `~jane.doe/My Docs/a.zim` and `(~jane/Private Notes)` become `[path]` and `([path]`; `~./a b`, `~../x y` and `~-/x y` become `[path]`.
  - `a~b/c` stays.
  - `approx ~3/4 of the files` becomes `approx [path]`. That is the same accepted over-redaction as the `~5/10 done` entry in §5.
- **Scheme-less `://`:** `x://…`, `X://…` and `ab1://…` go to `[url]` in step 3. A kept `https://example.org/x y` still gives `https://example.org`.
- **Punycode e-mail:**
  - `jane@example.xn--p1ai.` → `[email].`
  - `jane@xn--bcher-kva.xn--p1ai` → `[email]`
  - `a@b.xn--` → `[email]--`. The trailing `-` is trimmed as punctuation, which leaves `b.xn`, a valid address. That is harmless.

**Linear time**
I timed 33 adversarial units of 32,768 scalars each in a release build, including the new `~a`, `~a~`, `Pin`, `aTokenB`, `tokenX` and `passwordHash `. Every one ran in 3.7 ms or less, and the round-2 quadratic shapes now take 1.7–3.0 ms. The suffix skip uses the precomputed word-end table, and the `~name` scan stops at the next `~` or separator, so runs never overlap.

**Porting**
Read as a porter would, the spec's new text matches the code:
- §1 (linear time, mentioning the word-end table);
- step 2 right side and the no-separator sentence;
- step 4 last label;
- step 5 Home shape and the narrowed `://` exception.

I found no remaining ambiguity.

**Part A:** `failed_quit_retry_delay` puts at least 1 s between automatic retries when no window remains; a pressed Retry, or a Quit chosen again with a window open, is still immediate. §1.1 says the same. Closed.

**Lane residuals (accepted, listed in the spec's §4 unless noted)**
- A file or folder name containing `: `, `<`, `>`, a backtick or an unescaped `"` ends redaction early.
- Paths without a start shape stay: relative paths (`My Books/…`, `./`, `../`), `%USERPROFILE%`, `\Program Files\…` (a space in the first component) and `D:Users\…`.
- The host of an external URL is kept, and it may be personal.
- `git@host:account/repo` keeps the account and repository names.
- An unquoted secret with spaces loses only its first token.
- Some secrets are not recognised:
  - keywords outside the list;
  - `OAuth=…`;
  - `token hunter2` without a separator;
  - keyword + `_suffix` after a camelCase hump (`accessToken_value`) — not named in §4; a wording addition.
- Some e-mail addresses are not caught: non-ASCII or dotless hosts, and quoted local parts.
- Not verified on any OS: the live failed-Quit dialog, its window parenting, the backoff sleeps, and the zenity and kdialog exit codes. The macOS code was not compiled, and the Swift and Kotlin ports must still pass the 228 vectors and their own bound and linear-time tests.