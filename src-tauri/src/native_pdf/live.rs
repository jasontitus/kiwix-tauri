// SPDX-License-Identifier: GPL-3.0-or-later
//! A self-contained snapshot of the authenticated reader frame, not a reload of
//! the archive. Scripts never run in the export renderer; all visual resources
//! must already have been captured by the bridge as data URLs.
use crate::exports::html::{self, Html};
use dom_query::NodeRef;
use serde::Deserialize;
use std::{collections::HashMap, fs::OpenOptions, io::Write, path::{Path, PathBuf}};
use tokio_util::sync::CancellationToken;

pub(super) struct Document {
    pub root: String,
    pub resources: HashMap<String, (String, PathBuf, u64)>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub html: String,
    pub width: u32,
    pub height: u32,
    pub document_url: String,
}
impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.html.is_empty() || self.html.len() > 16 * 1024 * 1024 {
            return Err("The article capture is empty or exceeds the 16 MiB limit".into());
        }
        if !(1..=16384).contains(&self.width) || !(1..=10_000_000).contains(&self.height) {
            return Err("The live article has unsupported capture dimensions".into());
        }
        if u64::from(self.height) * 52328 > u64::from(self.width) * 76989 * 512 {
            return Err("The live article exceeds the 512-page PDF limit".into());
        }
        if self.document_url.len() > 32768 { return Err("Invalid live article identity".into()); }
        Ok(())
    }
    pub(super) fn document(self, directory: &Path, cancelled: &CancellationToken) -> Result<Document, String> {
        self.validate()?;
        let html = sanitize(&self.html, self.width, cancelled)?;
        let path = directory.join("live-document");
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file = options.open(&path).map_err(|error| format!("Creating live article snapshot: {error}"))?;
        file.write_all(html.as_bytes()).map_err(|error| format!("Writing live article snapshot: {error}"))?;
        let size = html.len() as u64;
        let resources = HashMap::from([("live.html".into(), ("text/html; charset=utf-8".into(), path, size))]);
        #[cfg(any(target_os = "android", test))]
        let resources = {
            let mut resources = resources;
            browser_icon(directory, &mut resources, cancelled)?;
            resources
        };
        Ok(Document { root: "/live.html".into(), resources })
    }
}

#[cfg(any(target_os = "android", test))]
fn browser_icon(directory: &Path, resources: &mut HashMap<String, (String, PathBuf, u64)>, cancelled: &CancellationToken) -> Result<(), String> {
    // WebView's implicit browser metadata is not an archive visual resource.
    // One transparent 1x1 ICO prevents an incidental /favicon.ico fetch from
    // being confused with a missing live-page image.
    const ICON: &[u8] = &[
        0, 0, 1, 0, 1, 0,
        1, 1, 0, 0, 1, 0, 32, 0, 48, 0, 0, 0, 22, 0, 0, 0,
        40, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 32, 0,
        0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 128, 0, 0, 0,
    ];
    if cancelled.is_cancelled() { return Err("Article PDF export was cancelled".into()); }
    let path = directory.join("browser-icon");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
    options.open(&path).and_then(|mut file| file.write_all(ICON))
        .map_err(|error| format!("Creating PDF renderer metadata: {error}"))?;
    resources.insert("favicon.ico".into(), ("image/x-icon".into(), path, ICON.len() as u64));
    Ok(())
}
fn escaped(value: &str, output: &mut String) {
    for ch in value.chars() {
        match ch { '&' => output.push_str("&amp;"), '<' => output.push_str("&lt;"), '>' => output.push_str("&gt;"), '"' => output.push_str("&quot;"), _ => output.push(ch) }
    }
}
/// Writes HTML `<style>` text. It is raw text, which the serializer cannot
/// escape, so a `<` that could open a tag (one followed by an ASCII letter or
/// `/`) is rewritten so that CSS reads the same tokens. The CSS tokenizer
/// tells the two cases apart:
/// - a `<` delimiter token gets an empty comment after it (`</**/`). CSS drops
///   the comment; HTML sees `</` and a non-letter, which is text in raw text
///   and a bogus comment in foreign content;
/// - a `<` inside any other token (a string, URL, comment, or an escaped `\<`
///   in a name) becomes the escape `\3c `, replacing an existing `\<` escape
///   whole. That is the same character there, and a comment is ignored anyway.
///
/// Every other `<` is left byte for byte, including `<!--` and `<=` in a media
/// query. No tag can then open inside the text, so a reparse cannot create an
/// element the renderer did not write, even in another namespace.
fn raw_style(text: &str, output: &mut String) -> Result<(), String> {
    // One pass, nothing kept per token: only a token whose text holds a `<`
    // that could open a tag is rewritten, and everything before it is copied.
    let bytes = text.as_bytes();
    let mut written = 0;
    crate::exports::css_tokens(text, |start, token_text, token| {
        if !token_text.contains('<') { return Ok(()); }
        let delimiter = matches!(token, cssparser::Token::Delim('<'));
        for (offset, _) in token_text.match_indices('<') {
            let index = start + offset;
            if !bytes.get(index + 1).is_some_and(|&next| next.is_ascii_alphabetic() || next == b'/') { continue; }
            if delimiter {
                output.push_str(&text[written..=index]);
                output.push_str("/**/");
            } else {
                // An escaping backslash is in the same token as the `<`.
                let backslashes = bytes[written..index].iter().rev().take_while(|&&byte| byte == b'\\').count();
                output.push_str(&text[written..index - backslashes % 2]);
                output.push_str("\\3c ");
            }
            written = index + 1;
        }
        Ok(())
    })?;
    output.push_str(&text[written..]);
    Ok(())
}
fn visual_url(value: &str) -> Result<(), String> {
    let value = value.trim();
    if value.is_empty() || value.starts_with('#') || value.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:")) { Ok(()) }
    else { Err("The live article contains an uncaptured visual resource; no PDF was saved".into()) }
}
fn css(value: &str) -> Result<(), String> {
    for reference in crate::exports::css_references(value, crate::exports::Malformed::Refuse)? { visual_url(&reference)?; }
    Ok(())
}
fn render_element(element: NodeRef<'_>, output: &mut String, depth: usize, cancelled: &CancellationToken) -> Result<(), String> {
    if cancelled.is_cancelled() { return Err("Article PDF export was cancelled".into()); }
    if depth > 512 { return Err("This article has too many nested elements to export as a PDF".into()); }
    let node = html::element(&element);
    let name = html::name(&node);
    // Only an HTML-namespace <style> is raw text. An SVG or MathML <style> is
    // parsed as markup, so its decoded text must be escaped like any other.
    let raw_text_style = name == "style" && node.name.ns == html5ever::ns!(html);
    match name {
        "script" | "noscript" | "base" | "meta" | "link" => return Ok(()),
        "iframe" | "frame" | "frameset" | "object" | "embed" | "video" | "audio" | "canvas" => return Err("The live article contains an uncaptured frame, media surface, or canvas; no PDF was saved".into()),
        "animate" | "animateMotion" | "animateTransform" | "set" | "discard" => return Err("The live article contains an unfrozen SVG animation; no PDF was saved".into()),
        _ => {}
    }
    output.push('<'); output.push_str(name);
    for (key, value) in html::attrs(&node) {
        let key_lower = key.to_ascii_lowercase();
        if key_lower.starts_with("on") || matches!(key_lower.as_str(), "href" | "xlink:href" | "action" | "formaction" | "target" | "ping" | "download" | "srcdoc" | "nonce" | "integrity" | "crossorigin" | "autofocus") {
            // Local SVG <use> references are visual, not navigation.
            if matches!(name, "use" | "image" | "feImage" | "feimage") && matches!(key_lower.as_str(), "href" | "xlink:href") {
                visual_url(value)?;
            } else { continue; }
        }
        if matches!(key_lower.as_str(), "src" | "poster" | "background") { visual_url(value)?; }
        if matches!(key_lower.as_str(), "srcset" | "imagesrcset") && !value.trim().is_empty() {
            return Err("The live article contains an unresolved responsive image; no PDF was saved".into());
        }
        if matches!(key_lower.as_str(), "style" | "fill" | "stroke" | "filter" | "clip-path" | "mask" | "cursor" | "marker-start" | "marker-mid" | "marker-end") { css(value)?; }
        output.push(' '); output.push_str(key); output.push_str("=\""); escaped(value, output); output.push('"');
    }
    output.push('>');
    if matches!(name, "area" | "br" | "col" | "hr" | "img" | "input" | "param" | "source" | "track" | "wbr") { return Ok(()); }
    if name == "style" {
        // The style sheet is the element's child text content: all of its
        // direct text children, joined. Comments and dropped elements between
        // them are not written, so the output joins them too, and the resource
        // check must see the joined sheet, not each piece.
        let sheet: String = element.children_it(false).filter_map(|child| html::text_node(&child)).map(|text| text.to_string()).collect();
        css(&sheet)?;
        if raw_text_style {
            // The HTML parser never gives a raw-text element child elements.
            if element.children_it(false).any(|child| child.is_element()) {
                return Err("The live article contains a style element with child elements; no PDF was saved".into());
            }
            raw_style(&sheet, output)?;
            output.push_str("</style>");
            return Ok(());
        }
    }
    // A <template>'s contents are not its children, so they are never rendered.
    for child in element.children_it(false) {
        if child.is_element() { render_element(child, output, depth + 1, cancelled)?; }
        else if let Some(text) = html::text_node(&child) {
            // Text outside an HTML <style> is escaped. That includes RCDATA (<title>,
            // <textarea>), which decodes the escapes back on reparse, and the
            // raw-text <xmp>, <noembed>, <noframes> and <plaintext>, which then
            // show the escapes literally but can never contain a tag.
            // <script>, <noscript> and <iframe> are dropped or refused above.
            escaped(&text, output);
        }
    }
    output.push_str("</"); output.push_str(name); output.push('>');
    Ok(())
}
fn sanitize(source: &str, width: u32, cancelled: &CancellationToken) -> Result<String, String> {
    // The snapshot is page-controlled; refuse hostile nesting while parsing,
    // before html5ever's quadratic scans or render_element's depth check.
    // The bridge stops at 100,000 captured nodes, text included; frames and
    // media add a few wrappers. A forged snapshot gets no more than a real one.
    let bounds = html::Bounds { nodes: (262_144, usize::MAX), ..html::ARTICLE_BOUNDS };
    let document = Html::parse_document_bounded(source, &bounds, Some(cancelled)).map_err(|refused| match refused {
        html::Refused::Cancelled => "Article PDF export was cancelled",
        html::Refused::TooComplex => "This article's markup is too deeply nested or too complex to export as a PDF",
    })?;
    let mut output = String::with_capacity(source.len());
    output.push_str("<!doctype html>");
    render_element(document.root_element(), &mut output, 0, cancelled)?;
    // Use the captured screen layout, not archive print-media reflow. The bridge
    // has frozen computed styles and live form/details/canvas/media state.
    let style = format!("<meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width={width},initial-scale=1\"><style>html{{width:{width}px!important;min-width:{width}px!important}}*{{animation:none!important;transition:none!important}}</style>");
    if let Some(head) = output.find("<head>") { output.insert_str(head + 6, &style); }
    else if let Some(head) = output.find("</head>") { output.insert_str(head, &style); }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn attr(node: &NodeRef<'_>, name: &str) -> Option<String> {
        html::attr(&html::element(node), name).map(str::to_owned)
    }
    #[test]
    fn live_state_survives_without_executable_or_navigation_authority() {
        let html = sanitize(r#"<html><head><script>throw 1</script><meta http-equiv="refresh" content="0;url=https://bad.invalid"></head><body><details open><summary>Changed</summary>Live</details><input value="edited" checked onclick="bad()"><a href="https://bad.invalid">Link</a><img src="data:image/png;base64,AAAA"><style>p{background:url(data:image/png;base64,AAAA)}</style></body></html>"#, 390, &CancellationToken::new()).unwrap();
        let document = Html::parse_document(&html);
        assert_eq!(attr(&document.select("input")[0], "value").as_deref(), Some("edited"));
        assert!(!document.select("details[open]").is_empty());
        assert!(document.select("script,[onclick],a[href],meta[http-equiv]").is_empty());
        assert!(html.contains("width:390px"));
    }
    #[test]
    fn flattened_child_state_and_namespaced_local_svg_resources_remain_visual() {
        let html = sanitize(r##"<div data-kiwix-export-frame="child" style="overflow:hidden;width:320px;height:200px"><svg><defs><g id="child-icon"><path d="M0 0L1 1"/></g></defs><use href="#child-icon" style="fill:url(#child-pattern)"/></svg><input value="child edit"></div>"##, 390, &CancellationToken::new()).unwrap();
        let document = Html::parse_document(&html);
        assert_eq!(attr(&document.select("[data-kiwix-export-frame] input")[0], "value").as_deref(), Some("child edit"));
        let icon = document.select("use")[0];
        assert_eq!(attr(&icon, "href").as_deref(), Some("#child-icon"));
        assert_eq!(attr(&icon, "style").as_deref(), Some("fill:url(#child-pattern)"));
        assert!(document.select("iframe,frame,object").is_empty());
    }
    #[test]
    fn uncaptured_resources_and_surfaces_fail_instead_of_exporting_a_partial_page() {
        for source in ["<img src='/secret'>", "<p style=\"background:u\\72l(https://bad.invalid/a)\">x</p>", "<iframe src='child'></iframe>", "<canvas></canvas>", "<style>@import 'https://bad.invalid/a';</style>"] {
            assert!(sanitize(source, 390, &CancellationToken::new()).is_err(), "{source}");
        }
        let cancelled = CancellationToken::new(); cancelled.cancel();
        assert!(sanitize("<p>live</p>", 390, &cancelled).is_err());
    }
    #[test]
    fn staged_document_uses_live_edits_without_loading_original_archive_content() {
        let directory = std::env::temp_dir().join(format!("kiwix-live-pdf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let snapshot = Snapshot { html: "<p id='changed'>Live edited text</p><input value='current value'>".into(),
            width: 390, height: 800, document_url: "http://127.0.0.1:1234/reader/article".into() };
        let document = snapshot.document(&directory, &CancellationToken::new()).unwrap();
        let bytes = std::fs::read(&document.resources["live.html"].1).unwrap();
        let parsed = Html::parse_document(std::str::from_utf8(&bytes).unwrap());
        assert_eq!(html::text(&parsed.select("#changed")[0]), "Live edited text");
        assert_eq!(attr(&parsed.select("input")[0], "value").as_deref(), Some("current value"));
        assert!(document.resources.contains_key("favicon.ico"));
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn bridge_audio_captures_prepare_with_the_real_pdf_sanitizer() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let output = std::process::Command::new("node")
            .current_dir(repository)
            .args(["scripts/test-reader-live-export.mjs", "--audio-snapshots"])
            .output()
            .expect("audio capture regression requires Node.js and the installed frontend dependencies");
        assert!(output.status.success(), "production bridge capture failed: {}",
            String::from_utf8_lossy(&output.stderr));
        let snapshots: Vec<Snapshot> = serde_json::from_slice(&output.stdout)
            .expect("production capture must return hidden and visible audio snapshots");
        assert_eq!(snapshots.len(), 2);
        for (index, snapshot) in snapshots.into_iter().enumerate() {
            let directory = std::env::temp_dir().join(format!("kiwix-audio-pdf-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let document = snapshot.document(&directory, &CancellationToken::new()).unwrap();
            let html = std::fs::read_to_string(&document.resources["live.html"].1).unwrap();
            let parsed = Html::parse_document(&html);
            let article = parsed.select("p").iter().map(html::text).collect::<String>();
            assert_eq!(article, "Article before audioArticle after audio");
            assert!(parsed.select("audio,source,[src],[autoplay],[onplay]").is_empty());
            assert!(!html.contains("uncaptured.invalid"));
            let player = parsed.select("[role=img]").first().copied();
            if index == 0 {
                assert!(player.is_none(), "hidden audio has no exported surface");
            } else {
                let player = player.expect("visible audio retains an inert player surface");
                assert_eq!(attr(&player, "aria-label").as_deref(), Some("Recorded interview"));
                assert_eq!(html::text(&player), "▶ 0:12 / 2:05");
                assert!(attr(&player, "style").unwrap().contains("width:300px;height:54px"));
            }
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
    #[test]
    fn sanitizer_output_keeps_its_parser_and_attribute_rules() {
        let source = r##"<head><noscript><p>hidden</p></noscript></head><body><p title="t" class="c" onclick="bad()">shown &amp; <b>bold</b></p><svg><use xlink:href="#icon" fill="red"/></svg><template><script>bad()</script><img src="/secret"></template></body>"##;
        let html = sanitize(source, 390, &CancellationToken::new()).unwrap();
        // Head <noscript> is raw text under scripting-enabled parsing and is dropped.
        assert!(!html.contains("hidden"), "{html}");
        // Attributes are written by local name, ordered by qualified name.
        assert!(html.contains(r#"<p class="c" title="t">shown &amp; <b>bold</b></p>"#), "{html}");
        assert!(html.contains(r##"<use fill="red" href="#icon"></use>"##), "{html}");
        // Template contents are neither rendered nor validated.
        assert!(html.contains("<template></template>"), "{html}");
        assert!(!html.contains("bad()") && !html.contains("secret"), "{html}");
        // html5ever 0.39 keeps <select> content, so an uncaptured image there now
        // fails the export instead of being silently dropped by the parser.
        assert!(sanitize("<select><img src='/secret'></select>", 390, &CancellationToken::new()).is_err());
    }
    /// Reparses sanitizer output the way the renderer will and checks that text
    /// stayed text: the elements are exactly `expected` (after the four the
    /// sanitizer adds to <html> and <head>), and no handler attribute exists.
    fn reparse_is_inert(output: &str, expected: &[&str]) -> Html {
        let document = Html::parse_document(output);
        let names: Vec<String> = document.elements().map(|e| html::name(&html::element(&e)).to_owned()).collect();
        let mut wanted = vec!["html", "head", "meta", "meta", "style", "body"];
        wanted.extend_from_slice(expected);
        assert_eq!(names, wanted, "{output}");
        for element in document.elements() {
            for (key, _) in html::attrs(&html::element(&element)) {
                assert!(!key.to_ascii_lowercase().starts_with("on"), "{key} in {output}");
            }
        }
        document
    }
    fn texts_of(document: &Html, name: &str, ns: html5ever::Namespace) -> Vec<String> {
        document.elements().filter(|e| { let e = html::element(e); html::name(&e) == name && e.name.ns == ns })
            .map(|e| html::text(&e)).collect()
    }
    #[test]
    fn foreign_style_text_cannot_become_markup_on_reparse() {
        let payload = "</style><img src=x onerror=alert(1)>";
        for (source, root, ns) in [
            ("<svg><style>&lt;/style&gt;&lt;img src=x onerror=alert(1)&gt;</style></svg>", "svg", html5ever::ns!(svg)),
            ("<math><style>&lt;/style&gt;&lt;img src=x onerror=alert(1)&gt;</style></math>", "math", html5ever::ns!(mathml)),
        ] {
            let output = sanitize(source, 390, &CancellationToken::new()).unwrap();
            assert!(!output.contains("<img"), "{output}");
            let document = reparse_is_inert(&output, &[root, "style"]);
            // The foreign <style> keeps exactly its original text.
            assert_eq!(texts_of(&document, "style", ns), [payload]);
        }
    }
    #[test]
    fn html_style_text_never_writes_a_tag() {
        let source = r#"<p>x</p><style>a::after{content:"<img src=x onerror=alert(1)>"}b::after{content:"\<p"}c::after{content:"\\<i"}</style>"#;
        let output = sanitize(source, 390, &CancellationToken::new()).unwrap();
        // `<` becomes the CSS escape `\3c `; an existing `\<` escape is replaced
        // whole, and an escaped backslash before `<` is kept.
        assert!(output.contains(r#"<style>a::after{content:"\3c img src=x onerror=alert(1)>"}b::after{content:"\3c p"}c::after{content:"\\\3c i"}</style>"#), "{output}");
        reparse_is_inert(&output, &["p", "style"]);
        // Even if a reparse put this <style> in foreign content, where its text
        // is tokenized as markup, there is no tag in it.
        let start = output.find("<style>a::").unwrap();
        let end = start + output[start..].find("</style>").unwrap() + "</style>".len();
        let foreign = Html::parse_document(&format!("<svg>{}</svg>", &output[start..end]));
        let names: Vec<String> = foreign.elements().map(|e| html::name(&html::element(&e)).to_owned()).collect();
        assert_eq!(names, ["html", "head", "body", "svg", "style"]);
    }
    /// The CSS token stream, comments dropped, with nested blocks walked.
    fn css_tokens(css: &str) -> Vec<String> {
        fn walk(parser: &mut cssparser::Parser<'_, '_>, out: &mut Vec<String>) {
            while let Ok(token) = parser.next_including_whitespace() {
                let token = token.clone();
                out.push(format!("{token:?}"));
                if matches!(token, cssparser::Token::Function(_) | cssparser::Token::ParenthesisBlock
                    | cssparser::Token::SquareBracketBlock | cssparser::Token::CurlyBracketBlock) {
                    let _ = parser.parse_nested_block(|nested| -> Result<(), cssparser::ParseError<'_, ()>> { walk(nested, out); Ok(()) });
                    out.push("end".into());
                }
            }
        }
        let mut input = cssparser::ParserInput::new(css);
        let mut out = Vec::new();
        walk(&mut cssparser::Parser::new(&mut input), &mut out);
        out
    }
    /// The span-per-token version this replaced, kept to show the output is
    /// byte for byte the same.
    fn raw_style_with_spans(text: &str, output: &mut String) -> Result<(), String> {
        // Byte ranges of every token except a `<` delimiter, in source order and
        // not overlapping (a block's token covers only its opening bracket).
        let mut spans = Vec::new();
        crate::exports::css_tokens(text, |start, token_text, token| {
            if !matches!(token, cssparser::Token::Delim('<')) { spans.push((start, start + token_text.len())); }
            Ok(())
        })?;
        let bytes = text.as_bytes();
        let mut written = 0;
        for (index, _) in text.match_indices('<') {
            if !bytes.get(index + 1).is_some_and(|&next| next.is_ascii_alphabetic() || next == b'/') { continue; }
            let token = spans.partition_point(|&(start, _)| start <= index);
            if token > 0 && spans[token - 1].1 > index {
                let backslashes = bytes[..index].iter().rev().take_while(|&&byte| byte == b'\\').count();
                output.push_str(&text[written..index - backslashes % 2]);
                output.push_str("\\3c ");
            } else {
                output.push_str(&text[written..=index]);
                output.push_str("/**/");
            }
            written = index + 1;
        }
        output.push_str(&text[written..]);
        Ok(())
    }
    #[test]
    fn raw_style_streaming_matches_the_span_version_and_keeps_no_per_token_state() {
        const PIECES: &[&str] = &["<", "</", "<a", "<!--", "-->", "/*", "*/", "\"", "'", "\\", "\\\\", "url(", "URL( ", "\\75 rl(", "myurl(", ")", "(", "{", "}", "[", "]", "a", " ", "\n", "/", "*", "style", ">", "<=", ";", "@media", "\\<", "\\\n", "é"];
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for _ in 0..20_000 {
            let mut css = String::new();
            seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
            for _ in 0..(1 + seed % 24) {
                seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17;
                css.push_str(PIECES[(seed % PIECES.len() as u64) as usize]);
            }
            let (mut streamed, mut spanned) = (String::new(), String::new());
            raw_style(&css, &mut streamed).unwrap();
            raw_style_with_spans(&css, &mut spanned).unwrap();
            assert_eq!(streamed, spanned, "{css:?}");
        }
        // 16 MiB of one-byte tokens: the output is the input, and the only
        // allocation is the output (about 16 MiB, not one entry per token).
        let sheet = ";".repeat(16 << 20);
        let mut written = String::new();
        raw_style(&sheet, &mut written).unwrap();
        assert!(written == sheet);
    }
    #[test]
    fn raw_style_keeps_css_tokens_and_opens_no_tag() {
        let cases = [
            // Unchanged byte for byte: no `<` here can open a tag.
            ("@media (width < 600px){p{color:red}}", "@media (width < 600px){p{color:red}}"),
            ("@media (400px <= width){p{color:red}}", "@media (400px <= width){p{color:red}}"),
            ("<!-- p{color:red} -->", "<!-- p{color:red} -->"),
            ("a\\\\<{color:red}", "a\\\\<{color:red}"),
            // A delimiter `<` before a letter or `/` gets an empty comment.
            ("a\\\\<b{color:red}", "a\\\\</**/b{color:red}"),
            ("p{}</style><img src=x onerror=alert(1)>", "p{}</**//style></**/img src=x onerror=alert(1)>"),
            // Inside strings, url() and comments, and for an escaped `<`, it
            // becomes the escape `\3c `.
            ("p::after{content:\"</style><img src=x onerror=alert(1)>\"}", "p::after{content:\"\\3c /style>\\3c img src=x onerror=alert(1)>\"}"),
            ("p{background:url(data:image/svg+xml,<svg></svg>)}", "p{background:url(data:image/svg+xml,\\3c svg>\\3c /svg>)}"),
            ("p{background:url( 'data:,<b>' )}", "p{background:url( 'data:,\\3c b>' )}"),
            ("/* </style><b> */p{}", "/* \\3c /style>\\3c b> */p{}"),
            ("a\\<b{}", "a\\3c b{}"),
            // A function that only ends in "url" is not a URL token, but an
            // escaped name that reads "url" is.
            ("p{x:myurl(<b)}", "p{x:myurl(</**/b)}"),
            ("p{background:\\75 rl(<b)}", "p{background:\\75 rl(\\3c b)}"),
            ("p{x:\\<url(<b)}", "p{x:\\3c url(</**/b)}"),
        ];
        // Nesting depth costs no stack.
        let deep = format!("{}<a{}", "(".repeat(200_000), ")".repeat(200_000));
        let mut written = String::new();
        raw_style(&deep, &mut written).unwrap();
        assert_eq!(written, format!("{}</**/a{}", "(".repeat(200_000), ")".repeat(200_000)));
        for (css, expected) in cases {
            let mut written = String::new();
            raw_style(css, &mut written).unwrap();
            assert_eq!(written, expected, "{css}");
            assert_eq!(css_tokens(&written), css_tokens(css), "{css}");
            // Reparsed as HTML raw text, the <style> holds exactly this text.
            let document = Html::parse_document(&format!("<style>{written}</style><p>x</p>"));
            let names: Vec<String> = document.elements().map(|e| html::name(&html::element(&e)).to_owned()).collect();
            assert_eq!(names, ["html", "head", "style", "body", "p"], "{written}");
            assert_eq!(texts_of(&document, "style", html5ever::ns!(html)), [written.as_str()]);
            // Reparsed as foreign content, where text is markup, nothing new
            // appears either (a `<!--` may hide what follows, never add to it).
            let document = Html::parse_document(&format!("<svg><style>{written}</style></svg><p>x</p>"));
            for element in document.elements() {
                let element = html::element(&element);
                assert!(["html", "head", "body", "svg", "style", "p"].contains(&html::name(&element)), "{written}");
                assert!(html::attrs(&element).is_empty(), "{written}");
            }
        }
    }
    #[test]
    fn split_foreign_style_is_checked_as_one_sheet() {
        for source in [
            "<svg><style>body{background:u<!---->rl(https://evil.invalid/b.png)}</style></svg>",
            "<svg><style>body{background:u<x></x>rl(https://evil.invalid/b.png)}</style></svg>",
            "<svg><style>body{background:u<script></script>rl(https://evil.invalid/b.png)}</style></svg>",
        ] {
            assert!(sanitize(source, 390, &CancellationToken::new()).is_err(), "{source}");
        }
        // Malformed URLs the browser skips are still refused here, in a <style>
        // and in a style attribute.
        for source in ["<style>a{b:url(a(b))}</style>", "<style>a{b:url(a\"b)}</style>", "<p style='background:url(a(b))'>x</p>", "<p style='background:url(a\"b)'>x</p>"] {
            assert!(sanitize(source, 390, &CancellationToken::new()).is_err(), "{source}");
        }
        let output = sanitize("<svg><style>p{fill:red}<!---->q{fill:blue}</style></svg>", 390, &CancellationToken::new()).unwrap();
        assert!(output.contains("<svg><style>p{fill:red}q{fill:blue}</style></svg>"), "{output}");
    }
    #[test]
    fn rcdata_and_raw_text_elements_stay_text_on_reparse() {
        let payload = "&lt;/{0}&gt;&lt;img src=x onerror=alert(1)&gt;";
        let mut source = String::from("<p>a</p>");
        let names = ["title", "textarea", "xmp", "noembed", "noframes"];
        for name in names { source.push_str(&format!("<{name}>{}</{name}>", payload.replace("{0}", name))); }
        source.push_str("<svg><title>&lt;/title&gt;&lt;img src=x onerror=alert(1)&gt;</title></svg>");
        let output = sanitize(&source, 390, &CancellationToken::new()).unwrap();
        assert!(!output.contains("<img"), "{output}");
        let document = reparse_is_inert(&output, &["p", "title", "textarea", "xmp", "noembed", "noframes", "svg", "title"]);
        // RCDATA decodes the escapes again. Raw text was never decoded, so its
        // literal `&lt;` is escaped once more and shows the escapes literally.
        assert_eq!(texts_of(&document, "textarea", html5ever::ns!(html)), ["</textarea><img src=x onerror=alert(1)>"]);
        assert_eq!(texts_of(&document, "xmp", html5ever::ns!(html)), ["&amp;lt;/xmp&amp;gt;&amp;lt;img src=x onerror=alert(1)&amp;gt;"]);
        assert_eq!(texts_of(&document, "title", html5ever::ns!(svg)), ["</title><img src=x onerror=alert(1)>"]);
        // <script>, <noscript> and <iframe> never reach the output.
        assert_eq!(sanitize("<p>a</p><script>&lt;img src=x onerror=1&gt;</script><noscript>&lt;img src=x onerror=1&gt;</noscript>", 390, &CancellationToken::new()).unwrap().matches("img").count(), 0);
        assert!(sanitize("<iframe>&lt;img src=x onerror=1&gt;</iframe>", 390, &CancellationToken::new()).is_err());
    }
    #[test]
    fn sanitizer_depth_limit_still_applies() {
        let ok = format!("{}x", "<div>".repeat(500));
        assert!(sanitize(&ok, 390, &CancellationToken::new()).is_ok());
        let deep = format!("{}x", "<div>".repeat(600));
        assert_eq!(sanitize(&deep, 390, &CancellationToken::new()).unwrap_err(),
            "This article has too many nested elements to export as a PDF");
    }
    #[test]
    fn hostile_nesting_is_refused_while_parsing_within_milliseconds() {
        struct Remove(std::path::PathBuf);
        impl Drop for Remove {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let directory = std::env::temp_dir().join(format!("kiwix-deep-pdf-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let _remove = Remove(directory.clone());
        // Unbounded, html5ever alone takes seconds on 40,000 nested elements.
        let snapshot = Snapshot { html: "<div>".repeat(40_000), width: 390, height: 800,
            document_url: "http://127.0.0.1:1234/reader/article".into() };
        let started = std::time::Instant::now();
        let error = snapshot.document(&directory, &CancellationToken::new()).err().unwrap();
        let elapsed = started.elapsed();
        assert_eq!(error, "This article's markup is too deeply nested or too complex to export as a PDF");
        assert!(elapsed < std::time::Duration::from_millis(if cfg!(debug_assertions) { 400 } else { 50 }), "{elapsed:?}");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0, "nothing is staged");
        // A forged snapshot of comments and text gets no more nodes than a
        // real capture.
        assert_eq!(sanitize(&"x<!>".repeat(200_000), 390, &CancellationToken::new()).err().unwrap(),
            "This article's markup is too deeply nested or too complex to export as a PDF");
        // A Wikipedia-sized article (about 100,000 nodes, the bridge's capture
        // limit) still exports.
        assert!(sanitize(&crate::exports::test_article(6_000), 390, &CancellationToken::new()).is_ok());
        // Render depth 512 (<html> is 0, <body> 1) still exports; the renderer,
        // not the parser, refuses 513.
        let at_limit = format!("<body>{}x", "<div>".repeat(511));
        assert!(sanitize(&at_limit, 390, &CancellationToken::new()).is_ok());
        let over = format!("<body>{}x", "<div>".repeat(512));
        assert_eq!(sanitize(&over, 390, &CancellationToken::new()).err().unwrap(),
            "This article has too many nested elements to export as a PDF");
    }
    #[test]
    fn snapshot_page_budget_checks_the_full_layout_without_truncation() {
        let mut snapshot = Snapshot { html: "<p>live</p>".into(), width: 100,
            height: (100u64 * 76989 * 512 / 52328) as u32, document_url: "http://localhost/article".into() };
        assert!(snapshot.validate().is_ok());
        snapshot.height += 1;
        assert!(snapshot.validate().is_err());
        snapshot.height = 1; snapshot.width = 0;
        assert!(snapshot.validate().is_err());
    }
}
