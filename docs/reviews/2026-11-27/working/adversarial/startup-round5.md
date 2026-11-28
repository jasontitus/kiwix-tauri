## Spec final v2.3 (startup round 5)

The check is clean: no blocking or should-fix findings. I changed no vector and propose no change to any existing one.

**Fixture**
- The first 228 cases are identical, in the same order, to the round-4 file I kept.
- The 10 new cases make 238.
- A scratch copy of the current pipeline (`.../adv-startup/redact5`) reproduces 238 of 238.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- crash::` → `test result: ok. 10 passed; 0 failed; … 1499 filtered out`.

**Spec text matches the code** (`secret_at`, `secret_value`, `skip_wrappers`):
- **Item 4.1:** `)` only after a quoted key's quote, as `find_from(quote + 1, ≠ ')')`.
- **Wrapper:** `Name(` followed by whitespace, line breaks included, and nesting allowed.
- **Form 3:** the token is measured from `q`, the value starts at `v`, the `authorization` second token works as before, and `path_end(q)` is used.
- **Form order:** a quoted value at `q` comes before `cookie`, and `cookie` runs from `v`.
- **Wrapper with no token:** a value with a wrapper but an empty token is still non-empty and is redacted. For example, `token: Some()` → `token: [redacted])`. That matches "the value starts at `v`".

**New rules, probed**
- Redacted correctly:
  - pretty-printed `Some(\n "…" )`, including nested `Some(Token("…"))`;
  - `Optional(Optional('a b c'))`;
  - `Config { password: Some(\n "hunter two", ), … }`, where the following `user: "jane"` stays;
  - `token: Some(\n\n\n abc\n)`;
  - `pin: Some(PinCode(1234))`;
  - `token: Some(  /home/jane doe/My Keys/id  )` → `token: [redacted]`;
  - `authorization: Some(Basic abc)` and the quoted form;
  - `cookie: Some("sid=1; x=2")`;
  - `[AnyHashable("token")): x` and `("token")) : "a b"`.
- No over-match:
  - `I will Some( day ) come back and token stays` and `fn(x) and token fn( and the pin) chain ))) end` are unchanged;
  - `Optional("pin") (unused)` and `("token") and then prose` are unchanged, because there is no separator.

**Linear time**
- Each new scan covers only the whitespace or `)` directly after one wrapper or one key, and that text is consumed or belongs to that candidate alone.
- A `cookie` value cannot chain wrappers through the next keyword.
- Timings in a release build, 32,768 scalars each:
  - the five new units (`token=a(\n`, `cookie=a(\n`, `("pin")))`, `token=Some( `, `password: Some(\n`): 1.1–1.9 ms;
  - all 38 adversarial units, the old ones included: at most 5.8 ms.

**Residual (Nit, not a finding): add it to §4**
A keyword whose value is a struct, array or map loses only the first token. This is not new in this round; v2.2 had the same gap. Examples from the probes:
- `token: Some(\n    Token {\n        value: "abc def",\n    },\n)` → `token: [redacted] { value: "abc def", }, )`
- `token: Ok([1, 2])` keeps `1, 2`.
- `token: Some(["a b"])` keeps `"a b"`.

It is close to the existing "secret with spaces … only its first token goes" line but not covered by it. Suggested §4 wording: "A value that is a struct, array or map (`Some(Token { … })`, `Ok([…])`): only its first token goes."

**Lane residuals, unchanged from round 4**
- A name containing `: `, `<`, `>`, a backtick or an unescaped `"` ends redaction early.
- Paths without a start shape stay: relative paths, `%USERPROFILE%`, `\Program Files\…` and `D:Users\…`.
- An external URL's host is kept.
- `git@host:account/repo` keeps the account and repository names.
- Some secrets are only partly caught:
  - an unquoted value with spaces loses only its first token;
  - so does a struct, array or map value (the Nit above);
  - unlisted keywords, `OAuth=`, and a keyword with no separator are not caught;
  - a keyword followed by `_suffix` after a camelCase hump (`accessToken_value`) is not caught.
- Some e-mail addresses are not caught: non-ASCII or dotless hosts, and quoted local parts.
- Not verified on any OS: the live failed-Quit dialog and zenity/kdialog behaviour. The macOS code was not compiled, and the Swift and Kotlin ports must pass all 238 vectors plus their own bound and linear-time tests.