# Crash-report redaction

This is the one rule for the text that the crash intakes write to disk. Three implementations follow it:

- Rust: `src-tauri/src/crash.rs` (`redact`),
- Swift: `plugins/tauri-plugin-kiwix-mobile/ios/Sources/CrashRedaction.swift` (called from `CrashIntake.swift`),
- Kotlin: `plugins/tauri-plugin-kiwix-mobile/android/src/main/java/org/kiwix/mobile/CrashRedaction.kt` (called from `CrashIntake.kt`).

`crash-redaction-cases.json`, next to this file, holds the test vectors. Each entry is `{"input", "expected"}`, and `expected` is the exact result of steps 1 to 8 below for `input`. Every implementation must reproduce every vector. When this text and a vector disagree, that is a defect in this text: report it, do not pick one.

The rule prefers losing words to keeping part of a path, a URL or a secret. Several steps therefore remove the rest of a clause.

## 1. Terms

**Scalar.** Text is a sequence of Unicode scalar values. Every index, count, scan and cut in this document is in scalar values.
- Swift: work on `String.unicodeScalars`, never on `Character` (a `Character` can hold several scalars, and `"\r\n"` is one `Character`).
- Kotlin: work on code points, never on `Char` indices that could split a surrogate pair.

**Whitespace.** A scalar with the Unicode `White_Space` property. That is exactly this list:

`U+0009`–`U+000D`, `U+0020`, `U+0085`, `U+00A0`, `U+1680`, `U+2000`–`U+200A`, `U+2028`, `U+2029`, `U+202F`, `U+205F`, `U+3000`.

Nothing else is whitespace. `U+001C`–`U+001F` and `U+200B` are not. Do not use a platform "is whitespace" function or a regular-expression `\s` unless it is proven equal to this list.

**Line break.** `U+000A` or `U+000D`. The **line end** of an index is the first line break at or after it, or the end of the text.

**ASCII letter, ASCII digit.** `A`–`Z`, `a`–`z`; `0`–`9`.

**Case.** Keywords, schemes and hosts are compared after lowercasing ASCII letters only. No other scalar is folded: `ſ` (U+017F) is not `s`, and `K` (U+212A) is not `k`.

**Placeholders.** `[redacted]`, `[url]`, `[reader-url]`, `[email]`, `[path]`.

**Linear time.** Each step must take time proportional to the length of its input, whatever the input. In particular, never rescan to the end of the line or of the text once per candidate: compute line ends (and, for the camelCase suffix of step 2, the end of each run of ASCII letters, digits and `_`) once, and make every other forward scan cover only text that the match then consumes.

### 1.1 `path_end(start)`

Where a path-like range that begins at `start` stops. Scan forward from `start` and stop before the first of:

- a line break;
- `<`, `>` or `` ` ``;
- a `"` for which the number of `\` directly before it, counted back no further than `start`, is even (zero is even);
- a `:` whose next scalar is whitespace;
- the end of the text.

Then move the end back over any whitespace directly before it, but not before `start`. The result is the end (exclusive).

Whitespace, `'`, `)`, `,` and `;` do not stop it: file names contain them.

## 2. The steps

Each step takes the text produced by the step before it. Placeholders written by an earlier step are ordinary text to a later one.

### Step 1: bound the input

Keep the first 32,768 scalars. If the text was longer and the first dropped scalar is not whitespace, the cut fell inside a run of non-whitespace: also remove every trailing non-whitespace scalar of what was kept. (Half a host name or half a path must not be judged as if it were whole.)

### Step 2: secrets

Scan from left to right. At each position `p`, look for a keyword that starts there.

**Keywords**, tried in this order:

`authorization`, `access_token`, `refresh_token`, `api_key`, `api-key`, `apikey`, `password`, `passwd`, `passphrase`, `pass`, `pwd`, `private_key`, `private-key`, `privatekey`, `client_secret`, `secret_access_key`, `secret_key`, `secret-key`, `secretkey`, `access_key`, `access-key`, `accesskey`, `secret`, `session_id`, `sessionid`, `jsessionid`, `credentials`, `credential`, `signature`, `cookie`, `bearer`, `token`, `auth`, `pin`.

A keyword matches at `p` when all of these hold:

1. The scalars at `p` equal the keyword, ignoring ASCII case.
2. Left side: `p` is the start of the text; or the scalar before `p` is not an ASCII letter or digit; or the scalar at `p` is an ASCII uppercase letter and the scalar before it is an ASCII lowercase letter or an ASCII digit (a camelCase hump: `accessToken`, `oauth2Token`).
3. Right side, one of:
   - the scalar after the keyword is not an ASCII letter, an ASCII digit or `_` (or the text ends there); or
   - a **camelCase suffix** follows: the scalar after the keyword is an ASCII uppercase letter and the keyword's last scalar, as written in the text, is an ASCII lowercase letter (`accessTokenValue`, `passwordHash`). The suffix is that letter and every ASCII letter, ASCII digit and `_` after it. It is skipped, and item 4 starts after it.
4. What follows the keyword (or its suffix) is, in order:
   1. optionally the closing quote of a quoted key: `"` or `'`, or one `\` followed by `"` or `'`; and, only after such a quote, any number of `)` (the key sits inside wrappers, as in Swift's `AnyHashable("token"): …`);
   2. any number of whitespace scalars;
   3. a **separator**: `=>`; or `=`; or `:` that is not followed by another `:`; or `,`, but only if the key was quoted (4.1) or the keyword is `authorization`;
   4. any number of whitespace scalars.

   Without a separator there is still a match if the keyword is `authorization` or `bearer`, it has no camelCase suffix, the key was not quoted, and at least one whitespace scalar was skipped in 4.2.
5. The **value**, which begins at the position `v` reached in 4, is not empty.

The first keyword in the list that matches wins. If none does, copy the scalar at `p` and go on at `p + 1`.

**The value.** Use the first form that applies:

1. **Quoted.** First skip any number of wrappers at `v`. A wrapper is an ASCII letter, then any ASCII letters, digits or `_`, then `(`, then any number of whitespace scalars, line breaks included (for example `Some(`, `Optional(`, and the pretty-printed `Some(` followed by a new line and indentation). Call the position after them `q`; without a wrapper `q` is `v`. If one of the following is at `q`, it is the value; otherwise go to form 2.
   - `"` at `q`: the value starts after it and ends before the first `"` on the same line for which the number of `\` directly before it (counted back no further than the value's start) is even; or at the line end if there is none.
   - `\` then `"` at `q`: the value starts after the `"` and ends before the `\` of the first `\"` on the same line whose `"` has exactly one `\` directly before it (counted back no further than the value's start); or at the line end if there is none.
   - `'` at `q`: the value starts after it and ends before the last `'` on the same line, or at the line end if there is none after `q`. If the value contains `/` or `\`, its end is the later of that and `path_end(value start)`.
2. **`cookie`.** For the keyword `cookie`: from `v` to the line end.
3. **Token.** The value starts at `v`. It ends at the first whitespace scalar or one of `,` `;` `&` `)` `]` `}` at or after `q`, or at the end of the text. So wrappers and the unquoted value inside them go together (`Some(1234)`, and the pretty-printed form with the value on the next line). The token is the part from `q`.
   - For the keyword `authorization`, if the token is not empty: skip whitespace after it, and if a non-empty token (same definition) follows, the value ends at the end of that second token.
   - If the token (with a second one, if any) contains `/` or `\`, the value's end is the later of that and `path_end(q)`.

Replace the value, and only the value, with `[redacted]`. Quotes, wrappers, the keyword and the separator stay. Go on at the end of the value.

### Step 3: URLs

Scan from left to right for `:`. A `:` inside a range that this step already replaced is skipped.

**Scheme.** Take the longest run of ASCII letters, ASCII digits, `+`, `-` and `.` that ends directly before the `:`. It must not reach back into a range this step already replaced. Drop scalars from its front until it starts with an ASCII letter. If nothing is left, this `:` is not a URL. Lowercase the scheme.

**Tail.** The text after the `:`.

This is a URL if the tail starts with `//`, or if the scheme is one of `blob`, `content`, `data`, `file`, `mailto`, `tauri`, `tel`, `zim` and the tail does not start with `:`. Otherwise go on after the `:`.

**Token end.** The first whitespace scalar, `"`, `<`, `>` or `` ` `` in the tail, or the end of the text.

The range to replace starts at the scheme. Its end and its replacement are the first case that applies:

1. Scheme `file`. If the tail is empty or starts with whitespace, the range is just `file:`. Otherwise it ends at `path_end(tail start)`. Replacement: `[url]`.
2. Scheme `data`. If the tail is empty or starts with whitespace, the range is just `data:`. Otherwise it ends at the line end. Replacement: `[url]`.
3. The tail starts with `//`.
   - The **authority** is the text after `//` up to the first `/`, `\`, `?` or `#`, or the token end if that comes first.
   - The replacement is `classify(scheme, authority)` (below).
   - If the replacement is a placeholder, or if the authority ends before the token end (the URL has a path, query or fragment), the range ends at the later of the token end and `path_end(tail start)`. Otherwise it ends at the token end.
4. Otherwise (`mailto:`, `tel:`, `blob:` and so on): the range ends at the token end. Replacement: `[url]`.

Go on at the end of the range.

**`classify(scheme, authority)`.**

1. If the scheme is neither `http` nor `https`: `[url]`.
2. Remove the userinfo: everything up to and including the last `@` in the authority.
3. Split off the port. If what is left starts with `[`, the host runs through the first `]` (no `]`: `[url]`) and the rest is the port part. Otherwise the port part starts at the last `:`, if there is one, and the host is what is before it.
4. Lowercase the host (ASCII only). Remove one trailing `.` if there is one.
5. `[reader-url]` if the host is `localhost`; or ends with `.localhost`; or starts with `127.` and every scalar after that is an ASCII digit or `.`; or is `[::1]`.
6. The port part must be empty, or `:` followed by one to five ASCII digits. Anything else: `[url]`.
7. The host is kept only if it is a DNS name: split at `.` into at least two labels; every label is non-empty and made only of `a`–`z`, `0`–`9` and `-`; the last label starts with a letter; and the last label is not one of `local`, `localdomain`, `lan`, `home`, `internal`, `arpa`. Anything else is `[url]`. That includes IPv4 and IPv6 literals in every spelling, single-label hosts and names with other scalars.
8. Kept: `scheme://host`, followed by `:` and the port digits as written unless there is no port or the port, ignoring leading zeros, is the default (`80` for `http`, `443` for `https`).

Nothing else of a kept URL survives. Its path, query and fragment are always dropped, because a path can name the article being read.

### Step 4: e-mail addresses

Scan from left to right for `@`. A `@` inside a range that this step already replaced is skipped.

- **Local part.** The longest run of ASCII letters, ASCII digits, `.`, `_`, `%`, `+` and `-` that ends directly before the `@`. It must not reach back into a range this step already replaced. It must not be empty.
- **Host.** The longest run of ASCII letters, ASCII digits, `.` and `-` that starts directly after the `@`, with every `.` and `-` at its very end removed (those are sentence punctuation and stay in the text). Split it at `.`: there must be at least two labels, none empty, and the last label must be either at least two scalars long and made of ASCII letters only, or longer than four scalars and start with `xn--` (ignoring ASCII case; a punycode label such as `xn--p1ai`).

If both hold, replace from the start of the local part to the end of the host with `[email]` and go on there. Otherwise go on after the `@`.

Both runs are taken whole. An address is never matched inside a longer run, so `a@b.c`, `pkg@1.2.3` and `user@host` stay.

This step comes after step 3 because that step has already replaced `mailto:` URLs and dropped the userinfo of `scheme://user@host`. It comes before step 5 so that text directly after an address starts at a boundary: `jane@example.org/Inbox Folder` becomes `[email][path]`, where the other order would leave `/Inbox Folder` behind.

### Step 5: paths

Scan from left to right. A path starts at index `i` when the boundary rule and one start shape hold, and the exception does not.

**Boundary.** `i` is the start of the text, or the scalar before `i` is not an ASCII letter, an ASCII digit, `_`, `.` or `-`.

**Start shapes.**
- Drive: an ASCII letter, then `:`, then `\` or `/`.
- Absolute: `/` followed by a scalar that is neither `/` nor whitespace.
- Home: `~`, then any number (possibly none) of ASCII letters, ASCII digits, `.`, `_` and `-`, then `/` or `\` (`~/…`, `~jane/…`).
- Share: `\\`; or `//` followed by a scalar that is neither `/` nor whitespace.
- Rooted: `\` followed by an ASCII letter or digit, where another `\` occurs after it and before the next whitespace scalar or the end of the text.

**Exception.** Neither `/` of a `://` starts a path when the scalar directly before that `:` is an ASCII letter. (That is the kept `scheme://host` from step 3. Without a letter there, as in `1://home/…` or ` ://Users/…`, step 3 kept nothing and the first `/` starts a share-shaped path.)

Replace from `i` to `path_end(i)` with `[path]` and go on at that end. Otherwise copy the scalar and go on at `i + 1`.

### Step 6: opaque tokens

A token scalar is an ASCII letter, an ASCII digit, or one of `_` `-` `+` `/` `=` `.`. Replace every maximal run of token scalars that is at least 40 scalars long and contains at least one ASCII digit and at least one ASCII letter with `[redacted]`.

### Step 7: whitespace

Split the text at whitespace, drop the empty pieces, and join the rest with one `U+0020`. (So there is no leading or trailing space.)

### Step 8: length

If the text is longer than 2,000 scalars, keep the first 1,999 and append `…` (U+2026). No vector is long enough to exercise this.

## 3. Where the steps are applied

A message, a thread name, an operation name or an identifier goes through all eight steps. Each platform then applies its own shorter limit to some fields (for example 64 scalars for a thread name).

Stack frames are formatted differently on each platform, so their handling is per platform. Every platform must still run steps 1 to 5, in that order, on any frame text it keeps, and must cut a frame by scalars. What Rust does:

- A frame is a line of the backtrace that starts with decimal digits and `:`. That line goes through steps 1 to 5 only, so its spacing is kept.
- If the next line starts with `at `, it is the source location. Rust runs step 3 on the rest of that line, keeps only what follows the last `/` or `\`, runs step 2 on that, and appends it to the frame after ` at `.
- Each frame is cut to 240 scalars (239 and `…`). At most 24 frames are kept.
- The panic location is `file name:line`, built from the compiler's own file name with everything up to the last `/` or `\` removed, and then goes through all eight steps. `lib.rs:1234` must come out unchanged.

Because frame lines run the same steps as any other text, a symbol that contains a keyword or a `data:` or `file:` scheme loses what follows it. This costs diagnostic detail only, and it is expected:

- `-[KiwixSession setAuthorization:forHost:] + 88` becomes `-[KiwixSession setAuthorization:[redacted]] + 88`;
- `+[NSData(Kiwix) data:encoding:] + 88` becomes `+[NSData(Kiwix) [url]`, which loses the offset too.

## 4. What this rule does not catch

These are known and accepted. They are not in the vectors as expected output.

- A file or folder name that contains `: `, `<`, `>`, `` ` `` or an unescaped `"`: redaction stops there, and the rest of the path stays.
- A path that does not start with one of the start shapes: `My Books/a.zim`, `%USERPROFILE%\Jane Doe`. That includes a rooted Windows path whose first component contains a space (`\Program Files\Kiwix\a.zim`) and a drive-relative path (`D:Users\Jane Doe`).
- The host of an external `http` or `https` URL. It is kept on purpose, and it can itself be personal: `https://jane-doe.github.io/a` becomes `https://jane-doe.github.io`, and `https://jane-private-medical.s3.amazonaws.com/x` becomes `https://jane-private-medical.s3.amazonaws.com`.
- What follows the `:` of an address-shaped remote: `git@github.com:janedoe/private-notes.git` becomes `[email]:janedoe/private-notes.git`, which still names the account and the repository.
- A secret with spaces that is neither quoted nor path-like: only its first token goes.
- A secret behind a keyword that is not in the list, or in a form the left and right sides reject (`OAuth=…`, `token hunter2` without a separator, or a `_` suffix after a camelCase hump such as `accessToken_value=…` or `refreshToken_v2=…`).
- An empty quoted value (`password=""`, `Some("")`) is no match for that keyword; the value forms after it are not tried, so `cookie="" a=b` is left unchanged.
- An e-mail address whose host has non-ASCII letters or no dot (`jane@exämple.org`, `jane@host`), or whose local part is quoted (`"jane doe"@example.org`).

## 5. What it removes that was harmless

Also known and accepted.

- The words after a path, a `file:` URL, a placeholder URL or a URL with a path, up to the next stop of `path_end`.
- A `/` after punctuation or a non-ASCII letter starts a path: `+/- 5` becomes `+[path]`, `café/menu` becomes `café[path]`, `</p>` becomes `<[path]>`. A `/` after an ASCII letter or digit does not: `and/or`, `1/2`, `I/O` stay.
- Several escape sequences in a row look like a rooted path: `\d+\.\d+` becomes `[path]`.
- A word after `pass:`, `auth:`, `pin:`, `signature:` and the other keywords, even in prose. With the camelCase suffix this includes type and field names: `TokenStream: empty` becomes `TokenStream: [redacted]`.
- A `~` directly before a number or a word with a `/` after it looks like a home path: `~5/10 done` becomes `[path]`.

## Notes for porters

These follow from the rules above and from the Rust reference; each is pinned by the vectors or by both ports' differential runs against Rust.

- Only `\"` opens an escaped quoted value. `\'` is accepted before a key but is not a quoted value form.
- `classify` checks for a reader host before it checks the port, so `http://localhost:abc/x` is `[reader-url]`.
- A kept URL is written as the lowercased scheme, `://`, and the lowercased host without one trailing dot, plus a non-default port. A default port is recognised with leading zeros ignored (`:080`, `:0443` are dropped); `:000` is kept as written.
- Every field cut (2,000 for a message, 240 for a frame line, 64 for a thread name) keeps one scalar fewer than the limit and adds `…`.
- Frame lines are platform-specific in form. Every implementation runs steps 1 to 5 on each frame line and then cuts it to 240 scalars. Rust does this for its `at file:line` lines as described above; Swift does it for each `callStackSymbols` line (at most 24); Kotlin for `class.method(file:line)`.
- Kotlin strings may contain an unpaired surrogate, which is not a scalar value. Treat it as one opaque non-whitespace code point: never split it, drop it or merge it. Swift and Rust strings cannot hold one.
