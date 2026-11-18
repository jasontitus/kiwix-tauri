# HTML parsing in the archive reader

The app handles ZIM article bytes and embedded index fields supplied by archive authors. The same text can reach several consumers. Treat rendered HTML, text extraction and export resource discovery as distinct operations. The observations below trace the locked Rust sources and normal workbench call sites on commit `9c0325b`. They identify where untrusted input reaches unsafe implementations. This review found no memory-corruption defect.

| App operation | Input and processing | Remaining boundary |
|---|---|---|
| Search rows and shared search | The Rust archive bridge produces an escaped plain title or an escaped generated excerpt. An index can also supply arbitrary stored snippet HTML. `search::snippet_text` recognizes up to 2,048 bytes of text with only the exact `&amp;`, `&lt;` and `&gt;` escapes. It removes one leading BOM, decodes once and checks cancellation. Raw markup, CR, NUL, other ampersands and longer inputs take the original bounded fragment path. | The accepted text subset uses Rust string operations. Other snippets use html5ever. The app displays the result in React text nodes, the shared-reader script uses `textContent`, and the shared HTML response escapes its excerpt. |
| Agent article text and preview excerpts | `agent_connector.rs` and `service/dispatch.rs` pass article HTML through `exports::html::Html::parse_document_bounded`, then inspect the tree. Selection rules distinguish hidden content, templates, whitespace and document structure. | Tokenizer, tree builder, atom and text storage. |
| HTML bundle resource discovery | `exports::markup_references` follows potentially nested HTML, including base URLs, styles, foreign content and template semantics. Multiple parses charge a shared work/CPU allowance. `exports::css_tokens` interprets stylesheet tokens. | HTML stack plus cssparser. A parser that loses an attribute namespace, stylesheet escape or base URL can drop a required resource or collect the wrong one. |
| Live PDF | `native_pdf::live::sanitize` builds a bounded DOM from the captured document, validates element/attribute context and CSS, and serializes an inert document for the platform print renderer. Raw text, RCDATA and foreign-content namespaces affect escaping. | HTML and CSS parsing, followed by separate platform rendering. The live sanitizer and the WebView remain required for this workflow. |
| Ordinary article display | The reader serves article bytes to the OS WebView. | Browser HTML, script, image and media processing have their own implementation and update cycle. Changing the Rust fragment parser does not alter this path. |

## Dependency worklist

| Locked component | Source and conditions | Replacement and acceptance criteria |
|---|---|---|
| `html5ever 0.39.0` | `tokenizer/mod.rs` selects an unsafe SSE2 or NEON data-state scanner at runtime on supported targets. Exact-error mode can skip this path and also changes tokenizer accounting and callback frequency. | Preserve the HTML5 tokenizer and tree builder. Prototype a checked bulk character scanner using borrowed UTF-8 slices. Compare token streams and full DOM serialization, then measure the bounded app sink under desktop and mobile budgets. |
| `markup5ever 0.39.0` | `util/buffer_queue.rs::pop_except_from` uses unchecked Tendril slicing and prefix removal after finding a run of ordinary characters. | Port this operation to a UTF-8-boundary-checked buffer. Preserve chunk consumption, CR/LF state across feeds and EOF handling. |
| `tendril 0.5.1` | `src/buf32.rs`, `src/tendril.rs` and `src/fmt.rs` implement manual allocation, inline/shared representation, refcount transitions, raw pointer access and UTF-8 conversions. Both markup5ever and the DOM consume `StrTendril`; changing just the app's input type leaves these paths active. | Prototype a checked copy-on-write string and byte buffer with shared backing and explicit slice offsets. Exercise clone, slicing, repeated front removal, append-after-slice, splitting multibyte characters across feeds and the decoder. Repeated copying at every token or front removal risks quadratic work. Keep upstream dual MIT/Apache-2.0 notices with reused source. |
| `web_atoms 0.2.6` and `string_cache 0.9.0` | Markup names use static atoms and a dynamic interner. `string_cache/src/atom.rs` and `dynamic_set.rs` use tagged representations, pointers and reference counts. Dynamic archive-controlled element and attribute names can reach the interner. | Preserve the generated static names and namespace/equality contracts. Test a safe owned/shared representation for dynamic names with mixed-case foreign attributes and concurrent distinct names. |
| `dom_query 0.28.0` | The app's declaration disables default features. A source scan found no explicit unsafe block or unsafe trait implementation in its `src/` tree. It stores parser-provided names and text. Tauri build tooling also resolves the same DOM library. | Retain the existing tree semantics when changing the storage layer. Compare normal elements, detached/reparented nodes, foster-parented table text, templates, duplicate attributes and deep input. Verify both the app and Tauri's build-time HTML processing after dependency patching. |
| `cssparser 0.37.0` | PDF and export code visit actual CSS tokens. `cow_rc_str.rs` uses raw pointers for borrowed/owned reference-counted strings. | Evaluate checked owned-or-borrowed token strings. Require the current URL-token, escape, nested block, malformed stylesheet and raw-style serialization tests to pass. |

The normal feature graph has no safe-storage switch for Tendril. The `encoding_rs` feature changes decoding support. dom_query's `atomic` feature changes sharing, and html5ever's `serde` feature changes serialization. None of these implements a checked string representation.

A development comparison used html5ever and dom_query directly to evaluate the existing exact-error option. The documents were entries 17 and 43 from the pinned Climate and Nollywood Wikipedia fixtures. Their serialized DOMs matched between modes. With the host cache warm on ARM64 macOS, 20 alternating rounds of ten parses measured medians of 107 versus 512 microseconds for the 13,395-byte Climate article and 664 versus 1,903 microseconds for the 53,657-byte Nollywood article (default versus exact errors). This measurement used dom_query's ordinary sink, which records parser errors. The app uses a bounded sink that discards diagnostics. Exact-error mode retains Tendril and atom storage and needs a separate app-budget evaluation.

## Verification

Run the search equivalence test and the manual release timing comparison from the app checkout.

```sh
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib search::tests::escaped_and_plain_snippets_match_the_bounded_html_fragment_parser
cargo test --locked --release --features custom-protocol --manifest-path src-tauri/Cargo.toml --lib search::tests::measure_search_snippet_pages -- --ignored --nocapture
cargo tree --locked --manifest-path src-tauri/Cargo.toml --target aarch64-apple-darwin -e features -i tendril
```

The fast-path test compares its result to the previous bounded fragment parser, including repeated high bytes at the byte cap, named and numeric references, markup, CR, NUL, control characters, single and repeated BOMs, Unicode and cancellation. The timing comparison processes fifty result strings per page and alternates both implementations over twenty measured rounds after two warmups. It measures text conversion inside the process. Archive I/O, IPC, layout, and mobile rendering require separate measurements.

On ARM64 macOS 27.2.1 with rustc 1.99.1, the median of five consecutive release-test process medians was:

| Fifty strings | Always parse DOM (µs) | Selected path (µs) |
|---|---:|---:|
| Plain text | 59.94 | 9.23 |
| Escaped text | 292.67 | 18.54 |
| Stored markup | 289.67 | 289.54 |

Independent differential checks matched all 34,196 accepted mixed strings. The app suite passed 1,191 tests with temporary local network fixtures enabled. Mobile CPU and memory behavior remain unmeasured.
