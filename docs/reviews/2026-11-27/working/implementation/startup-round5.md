# Lane `startup` — round 5 report (crash spec v2.3)

Both shapes are closed in Rust and written into the spec. The fixture went from 228 to 238 cases and none of the 228 existing expected values changed (checked by comparing the old and new files). I also closed one sibling shape you did not ask for; it is described under rule 3 below.

## The rules
1. **Wrapper followed by whitespace.** A wrapper is now `Name(` followed by any number of White_Space scalars, line breaks included. Nested wrappers each allow it. The quoted value after them is found and redacted as before (`crash.rs: skip_wrappers`).
2. **Quoted key followed by `)`.** After the closing quote of a quoted key, any number of `)` may come before the whitespace and separator. This applies only to quoted keys (`crash.rs: secret_at`).
3. **Added by me: unquoted value inside a wrapper.** The token of value form 3 is now measured from after the wrappers, and the value still starts at the wrapper. So `Some(1234)` behaves as before, and the pretty-printed form with the value on the next line is covered too. The path extension likewise starts from inside the wrapper.
   - Without this, the same `{:#?}` output that motivated rule 1 leaked every non-string value: `pin: Some(\n    1234,\n)` kept `1234`.
   - The leak search found the path half of this (2,090 hits on its first run, all `token: Some(` + newline + a path with spaces).

All three stay linear: each new scan covers whitespace or `)` directly after one wrapper or one key. Two inputs for them were added to the linear-time test.

## Vectors added (10)

| Input | Expected |
|---|---|
| `token: Some(\n        "abc def",\n    )` | `token: Some( "[redacted]", )` |
| `token=Some( "x y") z` | `token=Some( "[redacted]") z` |
| `password: Some(\n    Some(\n        'p w',\n    ),\n)` | `password: Some( Some( '[redacted]', ), )` |
| `[AnyHashable("token"): abc def, AnyHashable("password"): hunter2]` | `[AnyHashable("token"): [redacted] def, AnyHashable("password"): [redacted]]` |
| `Optional(AnyHashable("secret")): "s 1" and ("pin") = 12` | `Optional(AnyHashable("secret")): "[redacted]" and ("pin") = [redacted]` |
| `token: Some(\n    abc\n)` | `token: [redacted] )` |
| `token: Some(\n    /home/jane doe/My Keys/id\n)` | `token: [redacted] )` |
| `pin: Some(\n    1234,\n) and code: Ok(77) end` | `pin: [redacted], ) and code: Ok(77) end` |
| `-[KiwixSession setAuthorization:forHost:] + 88` | `-[KiwixSession setAuthorization:[redacted]] + 88` |
| `+[NSData(Kiwix) data:encoding:] + 88` | `+[NSData(Kiwix) [url]` |

One thing to know about the Swift dictionary vector: `abc def` is an unquoted value with a space, so only `abc` goes and ` def` stays. That is the existing §4 residual ("a secret with spaces that is neither quoted nor path-like"), not something this round changes. `hunter2` is fully redacted.

## Spec changes
- **Step 2, item 4.1:** `)` may follow a quoted key's closing quote.
- **Step 2, value form 1:** the wrapper definition includes the whitespace.
- **Step 2, value form 3:** the token is measured from after the wrappers; the path extension uses `path_end` from there.
- **§3:** new note that frame lines can lose selector text after a keyword or a `data:`/`file:` scheme, with the reviewer's two examples (also pinned as the last two vectors).
- **§4:** removed the "wrapper followed by whitespace before its quote" line. The empty-quoted-value line still holds and stays. The "Notes for porters" section is untouched.

## Results
- Leak search with seven new contexts (pretty-printed and spaced wrappers, quoted and unquoted; `AnyHashable("…")` keys): `0 leaks in 224048`.
- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib -- crash::` → `test result: ok. 10 passed; 0 failed; 0 ignored`. This includes the 238-vector fixture test and the linear-time test.
- Host clippy (`--lib --tests --features custom-protocol -- -D warnings`) → exit 0, `Finished`.

Files changed: `src-tauri/src/crash.rs`, `src-tauri/tests/fixtures/crash-redaction-spec.md`, `src-tauri/tests/fixtures/crash-redaction-cases.json`.

For the Swift and Kotlin ports: rule 3 is a third change beyond the two in your brief. The three token-form vectors (rows 6–8) are the ones that need it.
