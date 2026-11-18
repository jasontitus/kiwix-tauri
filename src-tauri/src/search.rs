// SPDX-License-Identifier: GPL-3.0-or-later
//! Rank-fused, title-inclusive search with bounded result pages.
//!
//! Title suggestions page book-major through the resolved archive scope. The
//! continuation token binds the prefix, scope, archive cursor, and local offset,
//! so a page produced for another query or scope can never be appended.
use crate::archive::{Archive, SearchHit};

use serde_json::{json, Value};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashSet, VecDeque},
};
use tokio_util::sync::CancellationToken;

const PAGE: u32 = 200;
/// One suggestion page stays small enough to paint without delaying typing.
pub const SUGGEST_PAGE_MAX: u32 = 50;
/// Bound per-archive paging so a malformed token cannot walk an index forever.
const SUGGEST_OFFSET_MAX: u32 = 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Combined,
    Title,
    Fulltext,
}

impl Mode {
    pub fn parse(mode: Option<&str>) -> Result<Self, String> {
        match mode {
            None | Some("combined") => Ok(Self::Combined),
            Some("title") => Ok(Self::Title),
            Some("fulltext") => Ok(Self::Fulltext),
            Some(_) => Err("Unknown search mode".into()),
        }
    }
}
struct Cursor {
    id: String,
    archive: Archive,
    buffered: VecDeque<SearchHit>,
    next: u32,
    exhausted: bool,
}
struct Head {
    slot: usize,
    rank: u32,
    hit: SearchHit,
}
impl PartialEq for Head {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Head {}
impl PartialOrd for Head {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Head {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max heap; lower rank/title comes first.
        other
            .rank
            .cmp(&self.rank)
            .then_with(|| other.hit.title.cmp(&self.hit.title))
            .then_with(|| other.slot.cmp(&self.slot))
    }
}
pub(crate) fn check_cancel(cancel: &CancellationToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("Search cancelled".into())
    } else {
        Ok(())
    }
}

/// Locally generated excerpts and titles are escaped text. Index-supplied snippets
/// can also contain HTML. Decode only the producer's three exact escapes, once.
fn short_text_snippet(html: &str) -> Option<String> {
    // A normal generated excerpt fits in 2 KiB even with escaped punctuation.
    // Longer fields retain the fragment parser's bounds and cancellation polling.
    if html.len() > 2048 || html.contains(['<', '\0', '\r']) {
        return None;
    }
    // parse_chunked_as removes one leading byte-order mark before tokenization.
    let html = html.strip_prefix('\u{feff}').unwrap_or(html);
    let mut parts = html.split('&');
    let mut text = String::with_capacity(html.len());
    text.push_str(parts.next().unwrap_or_default());
    for part in parts {
        let (decoded, rest) = if let Some(rest) = part.strip_prefix("amp;") {
            ('&', rest)
        } else if let Some(rest) = part.strip_prefix("lt;") {
            ('<', rest)
        } else {
            ('>', part.strip_prefix("gt;")?)
        };
        text.push(decoded);
        text.push_str(rest);
    }
    Some(text)
}

pub(crate) fn snippet_text(html: &str, cancel: &CancellationToken) -> Result<String, String> {
    check_cancel(cancel)?;
    if let Some(text) = short_text_snippet(html) {
        check_cancel(cancel)?;
        return Ok(text);
    }
    // Archive-supplied snippets are parsed under a snippet-sized work
    // budget and the request's cancellation token, so a hostile result
    // cannot hold the search request after it was retired.
    let fragment = crate::exports::html::Html::parse_fragment_bounded(
        html,
        &crate::exports::html::SNIPPET_BOUNDS,
        Some(cancel),
    )
    .map_err(|refused| match refused {
        crate::exports::html::Refused::Cancelled => "Search cancelled".to_string(),
        crate::exports::html::Refused::TooComplex => "A search result snippet was too complex to display".to_string(),
    })?;
    check_cancel(cancel)?;
    Ok(crate::exports::html::text(&fragment.root_element()))
}

fn row(id: &str, rank: u32, hit: SearchHit, show_snippets: bool, cancel: &CancellationToken) -> Result<Value, String> {
    check_cancel(cancel)?;
    let snippet = if show_snippets {
        snippet_text(&hit.snippet, cancel)?
    } else {
        String::new()
    };
    check_cancel(cancel)?;
    Ok(json!({"bookId":id,"path":crate::server::encode_path(&hit.path),"title":hit.title,"snippet":snippet,"score":1.0/(60.0+f64::from(rank))}))
}

fn append_rows(
    rows: &mut Vec<Value>,
    id: &str,
    offset: u32,
    hits: impl IntoIterator<Item = SearchHit>,
    show_snippets: bool,
    cancel: &CancellationToken,
) -> Result<(), String> {
    for (index, hit) in hits.into_iter().enumerate() {
        rows.push(row(id, offset.saturating_add(index as u32), hit, show_snippets, cancel)?);
    }
    check_cancel(cancel)
}
pub fn run(
    archives: Vec<(String, Result<Archive, String>)>,
    query: &str,
    mode: Mode,
    offset: u32,
    limit: u32,
    show_snippets: bool,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    check_cancel(cancel)?;
    // A caller-supplied limit must never
    // reserve or request an unbounded page.
    let limit = limit.clamp(1, PAGE);
    if mode == Mode::Combined {
        return run_combined(archives, query, offset, limit, show_snippets, cancel);
    }
    let fulltext = mode == Mode::Fulltext;
    let mut errors = Vec::new();
    let mut hits = Vec::with_capacity(limit as usize);
    let mut total = 0u64;
    let first_page = offset.saturating_add(limit).clamp(1, PAGE);
    if archives.len() == 1 {
        let (id, result) = archives.into_iter().next().unwrap();
        let page = result.and_then(|archive| archive.search_cancellable(query, fulltext, offset, limit, show_snippets, cancel));
        check_cancel(cancel)?;
        match page {
            Ok(page) => {
                total = page.total;
                append_rows(&mut hits, &id, offset, page.hits, show_snippets, cancel)?;
            }
            Err(error) => errors.push(json!({"bookId":id,"error":error})),
        }
        check_cancel(cancel)?;
        return Ok(json!({"hasMore":hits.len()==limit as usize,"hits":hits,"total":total,"errors":errors}));
    }
    let mut cursors = Vec::new();
    let mut heap = BinaryHeap::new();
    for (id, result) in archives {
        check_cancel(cancel)?;
        let page = result.and_then(|archive| {
            let page = archive.search_cancellable(query, fulltext, 0, first_page, show_snippets, cancel)?;
            Ok((archive, page))
        });
        check_cancel(cancel)?;
        match page {
            Ok((archive, page)) => {
                total = total.saturating_add(page.total);
                let count = page.hits.len() as u32;
                let mut cursor = Cursor {
                    id,
                    archive,
                    buffered: page.hits.into(),
                    next: count,
                    exhausted: count < first_page,
                };
                if let Some(hit) = cursor.buffered.pop_front() {
                    heap.push(Head {
                        slot: cursors.len(),
                        rank: 0,
                        hit,
                    });
                }
                cursors.push(cursor);
            }
            Err(error) => errors.push(json!({"bookId":id,"error":error})),
        }
    }
    let end = u64::from(offset) + u64::from(limit);
    let mut position = 0u64;
    while position < end {
        check_cancel(cancel)?;
        let Some(head) = heap.pop() else { break };
        let cursor = &mut cursors[head.slot];
        if position >= u64::from(offset) {
            hits.push(row(&cursor.id, head.rank, head.hit, show_snippets, cancel)?);
        }
        position += 1;
        if position == end {
            break;
        }
        if cursor.buffered.is_empty() && !cursor.exhausted {
            check_cancel(cancel)?;
            let page = cursor.archive.search_cancellable(query, fulltext, cursor.next, PAGE, show_snippets, cancel);
            check_cancel(cancel)?;
            match page {
                Ok(page) => {
                    let count = page.hits.len() as u32;
                    cursor.exhausted = count < PAGE;
                    cursor.next = cursor.next.saturating_add(count);
                    cursor.buffered = page.hits.into();
                }
                Err(error) => {
                    cursor.exhausted = true;
                    errors.push(json!({"bookId":cursor.id,"error":error}));
                }
            }
        }
        if let Some(hit) = cursor.buffered.pop_front() {
            if let Some(rank) = head.rank.checked_add(1) {
                heap.push(Head {
                    slot: head.slot,
                    rank,
                    hit,
                });
            }
        }
    }
    check_cancel(cancel)?;
    Ok(json!({"hasMore":hits.len()==limit as usize,"hits":hits,"total":total,"errors":errors}))
}

#[derive(Clone, Copy)]
struct Source {
    book: usize,
    fulltext: bool,
}

fn sources(book: usize, has_fulltext: bool) -> impl Iterator<Item = Source> {
    // Full text wins an equal-rank tie, retaining its richer snippet when a
    // title is also returned by the title index.
    has_fulltext.then_some(Source { book, fulltext: true })
        .into_iter()
        .chain(std::iter::once(Source { book, fulltext: false }))
}

#[derive(Default)]
struct CombinedCursor {
    buffered: VecDeque<SearchHit>,
    next: u32,
    exhausted: bool,
}

struct CombinedPage {
    hits: Vec<(usize, u32, SearchHit)>,
    total: u64,
    has_more: bool,
    errors: Vec<(usize, String)>,
}

/// Merge index streams by rank/title/scope order. Replaying the same
/// bounded streams for each offset makes pages deterministic without retaining
/// request-global state. Deduplication precedes offset/limit and is scoped to
/// archive identity: identical paths in different archives remain distinct.
fn collect_combined(
    sources: &[Source],
    offset: u32,
    limit: u32,
    cancel: &CancellationToken,
    mut fetch: impl FnMut(Source, u32, u32, &CancellationToken) -> Result<crate::archive::SearchResults, String>,
) -> Result<CombinedPage, String> {
    let mut cursors = Vec::with_capacity(sources.len());
    let mut heap = BinaryHeap::new();
    let mut total = 0u64;
    let mut errors = Vec::new();
    let first_page = offset.saturating_add(limit).clamp(1, PAGE);
    for (slot, source) in sources.iter().enumerate() {
        check_cancel(cancel)?;
        let result = fetch(*source, 0, first_page, cancel);
        check_cancel(cancel)?;
        let mut cursor = CombinedCursor::default();
        match result {
            Ok(page) => {
                total = total.saturating_add(page.total);
                cursor.next = page.hits.len() as u32;
                cursor.exhausted = cursor.next < first_page;
                cursor.buffered = page.hits.into();
                if let Some(hit) = cursor.buffered.pop_front() {
                    heap.push(Head { slot, rank: 0, hit });
                }
            }
            Err(error) => {
                cursor.exhausted = true;
                errors.push((source.book, error));
            }
        }
        cursors.push(cursor);
    }
    let mut hits = Vec::with_capacity(limit as usize);
    let book_count = sources.iter().map(|source| source.book + 1).max().unwrap_or(0);
    let mut seen: Vec<HashSet<String>> = (0..book_count).map(|_| HashSet::new()).collect();
    let mut position = 0u64;
    let end = u64::from(offset) + u64::from(limit);
    let mut has_more = false;
    while let Some(head) = heap.pop() {
        check_cancel(cancel)?;
        let source = sources[head.slot];
        if !seen[source.book].contains(&head.hit.path) {
            seen[source.book].insert(head.hit.path.clone());
            if position == end {
                has_more = true;
                break;
            }
            if position >= u64::from(offset) {
                hits.push((source.book, head.rank, head.hit));
            }
            position += 1;
        }
        let cursor = &mut cursors[head.slot];
        if cursor.buffered.is_empty() && !cursor.exhausted {
            check_cancel(cancel)?;
            let result = fetch(source, cursor.next, PAGE, cancel);
            check_cancel(cancel)?;
            match result {
                Ok(page) => {
                    let count = page.hits.len() as u32;
                    cursor.exhausted = count < PAGE;
                    cursor.next = cursor.next.saturating_add(count);
                    cursor.buffered = page.hits.into();
                }
                Err(error) => {
                    cursor.exhausted = true;
                    errors.push((source.book, error));
                }
            }
        }
        if let Some(hit) = cursor.buffered.pop_front() {
            if let Some(rank) = head.rank.checked_add(1) {
                heap.push(Head { slot: head.slot, rank, hit });
            }
        }
    }
    check_cancel(cancel)?;
    // Native totals are estimates and the two indexes overlap. Once exhausted,
    // the unique count is exact; before then expose only an estimated upper
    // count, never a false "no more" caused by duplicated title/fulltext rows.
    total = if has_more { total.max(position + 1) } else { position };
    Ok(CombinedPage { hits, total, has_more, errors })
}

fn run_combined(
    archives: Vec<(String, Result<Archive, String>)>,
    query: &str,
    offset: u32,
    limit: u32,
    show_snippets: bool,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    let mut streams = Vec::new();
    let mut errors = Vec::new();
    for (book, (id, archive)) in archives.iter().enumerate() {
        check_cancel(cancel)?;
        match archive.as_ref().map_err(Clone::clone)
            .and_then(|archive| archive.metadata_cancellable(cancel)) {
            Ok(metadata) => streams.extend(sources(book, metadata.has_fulltext_index)),
            Err(error) => errors.push(json!({"bookId":id,"error":error})),
        }
        check_cancel(cancel)?;
    }
    let page = collect_combined(&streams, offset, limit, cancel, |source, offset, limit, cancel| {
        archives[source.book].1.as_ref().map_err(Clone::clone)?
            .search_cancellable(query, source.fulltext, offset, limit, show_snippets, cancel)
    })?;
    let mut hits = Vec::with_capacity(page.hits.len());
    for (book, rank, hit) in page.hits {
        hits.push(row(&archives[book].0, rank, hit, show_snippets, cancel)?);
    }
    for (book, error) in page.errors {
        errors.push(json!({"bookId":archives[book].0,"error":error}));
    }
    check_cancel(cancel)?;
    Ok(json!({"hasMore":page.has_more,"hits":hits,"total":page.total,"errors":errors}))
}

fn suggestion_token(prefix: &str, scope: &[String], book: usize, offset: u32) -> String {
    let mut digest = md5::Context::new();
    digest.consume(prefix.as_bytes());
    digest.consume([0]);
    for id in scope {
        digest.consume(id.as_bytes());
        digest.consume([0]);
    }
    digest.consume(book.to_string().as_bytes());
    digest.consume([0]);
    digest.consume(offset.to_string().as_bytes());
    format!("{book}:{offset}:{}", hex::encode(digest.finalize().0))
}

fn suggestion_cursor(token: &str, prefix: &str, scope: &[String]) -> Result<(usize, u32), String> {
    let expired = || "Suggestion continuation expired".to_string();
    let mut parts = token.split(':');
    let (Some(book), Some(offset), Some(_), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(expired());
    };
    let book: usize = book.parse().map_err(|_| expired())?;
    let offset: u32 = offset.parse().map_err(|_| expired())?;
    if book > scope.len()
        || offset > SUGGEST_OFFSET_MAX
        || suggestion_token(prefix, scope, book, offset) != token
    {
        return Err(expired());
    }
    Ok((book, offset))
}

#[derive(Debug)]
pub(crate) struct SuggestPage {
    pub(crate) hits: Vec<(usize, SearchHit)>,
    pub(crate) next: Option<(usize, u32)>,
    pub(crate) errors: Vec<(usize, String)>,
}

/// Page book-major: archive order, then libzim suggestion order within each
/// archive. A short page exhausts that archive, so the cursor advances without a
/// separate count call and no result is duplicated or skipped between pages.
pub(crate) fn collect_page(
    scope: &[String],
    cursor: (usize, u32),
    limit: u32,
    cancel: &CancellationToken,
    mut fetch: impl FnMut(usize, u32, u32, &CancellationToken) -> Result<Vec<SearchHit>, String>,
) -> Result<SuggestPage, String> {
    let (mut book, mut offset) = cursor;
    let mut hits = Vec::new();
    let mut errors = Vec::new();
    let mut remaining = limit;
    while remaining > 0 && book < scope.len() {
        check_cancel(cancel)?;
        match fetch(book, offset, remaining, cancel) {
            Ok(page) => {
                check_cancel(cancel)?;
                let count = page.len() as u32;
                hits.extend(page.into_iter().map(|hit| (book, hit)));
                remaining = remaining.saturating_sub(count);
                if count == 0 || remaining > 0 {
                    book += 1;
                    offset = 0;
                } else {
                    offset = offset.saturating_add(count);
                    if offset > SUGGEST_OFFSET_MAX {
                        book += 1;
                        offset = 0;
                    }
                    let next = (book < scope.len()).then_some((book, offset));
                    return Ok(SuggestPage { hits, next, errors });
                }
            }
            Err(error) => {
                errors.push((book, error));
                book += 1;
                offset = 0;
            }
        }
    }
    check_cancel(cancel)?;
    Ok(SuggestPage { hits, next: None, errors })
}

/// Title suggestions are title-only: no snippet, score, or full-text fallback is
/// computed, so the snippet preference can never leak article text here.
pub fn suggest(
    archives: Vec<(String, Result<Archive, String>)>,
    prefix: &str,
    token: Option<&str>,
    limit: u32,
    cancel: &CancellationToken,
) -> Result<Value, String> {
    check_cancel(cancel)?;
    if prefix.is_empty() {
        return Ok(json!({"suggestions":[],"hasMore":false,"nextToken":null,"errors":[]}));
    }
    let limit = limit.clamp(1, SUGGEST_PAGE_MAX);
    let scope: Vec<String> = archives.iter().map(|(id, _)| id.clone()).collect();
    let cursor = match token {
        Some(token) => suggestion_cursor(token, prefix, &scope)?,
        None => (0, 0),
    };
    let page = collect_page(&scope, cursor, limit, cancel, |book, offset, limit, cancel| {
        archives[book]
            .1
            .as_ref()
            .map_err(|error| error.clone())?
            .search_cancellable(prefix, false, offset, limit, false, cancel)
            .map(|page| page.hits)
    })?;
    let next = page.next
        .map(|(book, offset)| suggestion_token(prefix, &scope, book, offset));
    let suggestions: Vec<Value> = page
        .hits
        .into_iter()
        .map(|(book, hit)| {
            json!({
                "bookId": scope[book],
                "path": crate::server::encode_path(&hit.path),
                "title": hit.title,
            })
        })
        .collect();
    let errors: Vec<Value> = page
        .errors
        .into_iter()
        .map(|(book, error)| json!({"bookId": scope[book], "error": error}))
        .collect();
    Ok(json!({
        "suggestions": suggestions,
        "hasMore": next.is_some(),
        "nextToken": next,
        "errors": errors,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_and_plain_snippets_match_the_bounded_html_fragment_parser() {
        let cancel = CancellationToken::new();
        let controls = (1..=127).filter_map(char::from_u32).filter(|c| !matches!(c, '&' | '<' | '\r')).collect::<String>();
        for source in ["", "   ", "a\n\t b", "\u{feff}", "\u{feff}text", "\u{feff}\u{feff}text", "¿日🌐\u{feff}", "&lt;img src=x&gt;", "&amp;lt;script&amp;gt;",
            "&lt;&amp;&gt; &amp;amp; &gt;", &controls, &"é".repeat(1024)] {
            let fragment = crate::exports::html::Html::parse_fragment_bounded(source, &crate::exports::html::SNIPPET_BOUNDS, Some(&cancel)).unwrap();
            let expected = crate::exports::html::text(&fragment.root_element());
            assert_eq!(short_text_snippet(source).as_deref(), Some(expected.as_str()));
            assert_eq!(snippet_text(source, &cancel).unwrap(), expected);
        }
        for source in ["<b>x</b>", "&nbsp;", "&#38;", "&notin", "AT&T", "a\rb", "a\0b", &"a".repeat(2049)] {
            assert!(short_text_snippet(source).is_none());
            let fragment = crate::exports::html::Html::parse_fragment_bounded(source, &crate::exports::html::SNIPPET_BOUNDS, Some(&cancel)).unwrap();
            assert_eq!(snippet_text(source, &cancel).unwrap(), crate::exports::html::text(&fragment.root_element()));
        }
        cancel.cancel();
        assert!(snippet_text("short &amp; plain", &cancel).is_err());
    }

    #[test]
    #[ignore = "Manual release comparison of fifty search-result snippets"]
    fn measure_search_snippet_pages() {
        use crate::exports::html::{self, Html, SNIPPET_BOUNDS};
        use std::{hint::black_box, time::Instant};
        let cancel = CancellationToken::new();
        let reference = |s: &str| {
            let tree = Html::parse_fragment_bounded(s, &SNIPPET_BOUNDS, Some(&cancel)).unwrap();
            html::text(&tree.root_element())
        };
        for (name, source) in [
            ("plain", "Climate science examines the atmosphere, oceans and ice. ".repeat(4)),
            ("escaped", "Rock &amp; roll — 東京 &lt;music&gt; in the 1950s. ".repeat(4)),
            ("markup", "<b>Climate</b> change and &nbsp;global warming. ".repeat(4)),
        ] {
            assert_eq!(reference(&source), snippet_text(&source, &cancel).unwrap());
            let mut samples = [Vec::new(), Vec::new()];
            for round in 0..22 {
                for step in 0..2 {
                    let variant = (round + step) % 2;
                    let started = Instant::now();
                    for _ in 0..50 {
                        if variant == 0 {
                            black_box(reference(black_box(&source)));
                        } else {
                            black_box(snippet_text(black_box(&source), &cancel).unwrap());
                        }
                    }
                    if round >= 2 {
                        samples[variant].push(started.elapsed().as_nanos());
                    }
                }
            }
            let micros = samples.map(|mut times| {
                times.sort_unstable();
                (times[9] + times[10]) as f64 / 2000.0
            });
            eprintln!("{name}: DOM {:.2} µs, selected path {:.2} µs per page", micros[0], micros[1]);
        }
    }

    fn hit(title: &str) -> SearchHit {
        SearchHit {
            path: title.into(),
            title: title.into(),
            snippet: format!("<p>{title} <b>snippet</b></p>"),
            score: 1.0,
        }
    }

    fn native_page(hits: &[SearchHit], offset: u32, limit: u32) -> crate::archive::SearchResults {
        crate::archive::SearchResults {
            hits: hits.iter().skip(offset as usize).take(limit as usize).cloned().collect(),
            total: hits.len() as u64,
        }
    }

    #[test]
    fn snippets_are_fragment_text_and_respect_cancellation() {
        let cancel = CancellationToken::new();
        let mut snippet = hit("Fish");
        snippet.snippet = "<p>Fish &amp; chips&nbsp;<b>hot</b>\n<template>T</template><script>s</script></p><table><td>cell</td></table>&notin;".into();
        let value = row("book", 0, snippet.clone(), true, &cancel).unwrap();
        assert_eq!(value["snippet"], "Fish & chips\u{a0}hot\nTscell\u{2209}");
        assert_eq!(row("book", 0, snippet.clone(), false, &cancel).unwrap()["snippet"], "");
        cancel.cancel();
        assert_eq!(row("book", 0, snippet, true, &cancel).unwrap_err(), "Search cancelled");
    }

    #[test]
    fn combined_pages_deduplicate_indexes_but_keep_distinct_archives() {
        let streams: Vec<_> = sources(0, true).chain(sources(1, false)).collect();
        let fulltext = vec![hit("Shared"), hit("Body only")];
        let titles = [vec![hit("Shared"), hit("Title only")], vec![hit("Shared")]];
        let cancel = CancellationToken::new();
        let fetch = |source: Source, offset, limit, _: &CancellationToken| {
            let rows = if source.fulltext {
                assert_eq!(source.book, 0, "Title-only archive was sent to full-text search");
                &fulltext
            } else {
                &titles[source.book]
            };
            Ok(native_page(rows, offset, limit))
        };
        let all = collect_combined(&streams, 0, 20, &cancel, fetch).unwrap();
        let expected: Vec<_> = all.hits.iter().map(|(book, _, hit)| (*book, hit.path.clone())).collect();
        assert_eq!(expected, [
            (0, "Shared".into()),
            (1, "Shared".into()),
            (0, "Body only".into()),
            (0, "Title only".into()),
        ]);
        assert_eq!(all.total, 4);
        assert!(!all.has_more);
        assert!(all.errors.is_empty());
        let mut paged = Vec::new();
        for offset in 0..4 {
            let page = collect_combined(&streams, offset, 1, &cancel, fetch).unwrap();
            assert_eq!(page.has_more, offset < 3);
            paged.extend(page.hits.into_iter().map(|(book, _, hit)| (book, hit.path)));
        }
        assert_eq!(paged, expected);
    }

    #[test]
    fn combined_deduplication_precedes_native_refill_and_public_offset() {
        let streams: Vec<_> = sources(0, true).collect();
        let fulltext: Vec<_> = (0..PAGE + 5).map(|index| hit(&format!("Article {index:04}"))).collect();
        let mut titles = fulltext.clone();
        titles.push(hit("Title beyond the full-text index"));
        let cancel = CancellationToken::new();
        let mut refills = Vec::new();
        let page = collect_combined(&streams, PAGE, 5, &cancel, |source, offset, limit, _| {
            if offset > 0 { refills.push((source.fulltext, offset)); }
            Ok(native_page(if source.fulltext { &fulltext } else { &titles }, offset, limit))
        }).unwrap();
        assert_eq!(page.hits.len(), 5);
        assert_eq!(page.hits.first().unwrap().2.path, "Article 0200");
        assert_eq!(page.hits.last().unwrap().2.path, "Article 0204");
        assert!(page.has_more);
        assert_eq!(refills, [(true, PAGE), (false, PAGE)]);
        let last = collect_combined(&streams, PAGE + 5, 5, &cancel, |source, offset, limit, _| {
            Ok(native_page(if source.fulltext { &fulltext } else { &titles }, offset, limit))
        }).unwrap();
        assert_eq!(last.hits.len(), 1);
        assert_eq!(last.hits[0].2.path, "Title beyond the full-text index");
        assert_eq!(last.total, u64::from(PAGE) + 6);
        assert!(!last.has_more);
    }

    #[test]
    fn combined_title_only_archives_do_not_request_fulltext() {
        let streams: Vec<_> = sources(0, false).collect();
        let cancel = CancellationToken::new();
        let page = collect_combined(&streams, 0, 10, &cancel, |source, offset, limit, _| {
            assert!(!source.fulltext);
            Ok(native_page(&[hit("Title without a body index")], offset, limit))
        }).unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.hits[0].2.title, "Title without a body index");
        assert!(page.errors.is_empty());
        assert!(!page.has_more);
    }

    #[test]
    fn cancelling_combined_search_during_refill_retires_its_whole_page() {
        let streams: Vec<_> = sources(0, false).collect();
        let titles: Vec<_> = (0..PAGE + 1).map(|index| hit(&index.to_string())).collect();
        let cancel = CancellationToken::new();
        let mut fetched = 0;
        let result = collect_combined(&streams, PAGE, 5, &cancel, |_, offset, limit, _| {
            fetched += 1;
            if offset > 0 { cancel.cancel(); }
            Ok(native_page(&titles, offset, limit))
        });
        assert!(matches!(result, Err(error) if error == "Search cancelled"));
        assert_eq!(fetched, 2);
    }

    #[test]
    fn a_failed_fulltext_stream_does_not_hide_title_matches() {
        let streams: Vec<_> = sources(0, true).collect();
        let cancel = CancellationToken::new();
        let page = collect_combined(&streams, 0, 10, &cancel, |source, offset, limit, _| {
            if source.fulltext { Err("Index unavailable".into()) }
            else { Ok(native_page(&[hit("Still readable by title")], offset, limit)) }
        }).unwrap();
        assert_eq!(page.hits.len(), 1);
        assert_eq!(page.errors, [(0, "Index unavailable".into())]);
        assert!(!page.has_more);
    }

    #[test]
    fn capped_suggestion_archive_continues_at_the_next_archive() {
        let scope = vec!["large".to_string(), "next".to_string()];
        let cancel = CancellationToken::new();
        let mut cursor = (0, 0);
        let mut seen = Vec::new();
        let mut crossed_cap = false;
        loop {
            let page = collect_page(&scope, cursor, 15, &cancel, |book, offset, limit, _| {
                assert!(offset <= SUGGEST_OFFSET_MAX);
                let count = if book == 0 { 1200 } else { 3 };
                Ok((offset..count).take(limit as usize)
                    .map(|index| hit(&format!("{book}:{index}"))).collect())
            }).unwrap();
            seen.extend(page.hits.into_iter().map(|(_, hit)| hit.title));
            match page.next {
                Some(next) => {
                    if next.0 == 1 {
                        assert_eq!(next, (1, 0));
                        assert_eq!(seen.len(), 1005);
                        crossed_cap = true;
                    }
                    let token = suggestion_token("prefix", &scope, next.0, next.1);
                    cursor = suggestion_cursor(&token, "prefix", &scope).unwrap();
                }
                None => break,
            }
        }
        assert!(crossed_cap);
        assert_eq!(seen.len(), 1008);
        assert_eq!(&seen[1005..], ["1:0", "1:1", "1:2"]);
    }

    #[test]
    fn the_last_capped_archive_has_no_unusable_continuation() {
        let scope = vec!["large".to_string()];
        let cancel = CancellationToken::new();
        let page = collect_page(&scope, (0, 990), 15, &cancel, |_, offset, limit, _| {
            Ok((offset..offset + limit).map(|index| hit(&index.to_string())).collect())
        }).unwrap();
        assert_eq!(page.hits.len(), 15);
        assert_eq!(page.next, None);
    }

    #[test]
    fn snippets_disabled_preserves_hits_without_returning_fragment_text() {
        let cancel = CancellationToken::new();
        let result = row("book", 0, hit("Article"), false, &cancel).unwrap();
        assert_eq!(result["title"], "Article");
        assert_eq!(result["path"], "Article");
        assert_eq!(result["snippet"], "");
        let enabled = row("book", 0, hit("Article"), true, &cancel).unwrap();
        assert_eq!(enabled["snippet"], "Article snippet");
    }

    #[test]
    fn cancellation_between_snippet_rows_stops_the_remaining_page() {
        let cancel = CancellationToken::new();
        let mut fetched = 0;
        let page = std::iter::from_fn(|| {
            fetched += 1;
            match fetched {
                1 => Some(hit("First")),
                2 => {
                    cancel.cancel();
                    Some(hit("Retired"))
                }
                _ => panic!("Cancelled search consumed another result"),
            }
        });
        let mut rows = Vec::new();
        assert_eq!(
            append_rows(&mut rows, "book", 0, page, true, &cancel),
            Err("Search cancelled".into()),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["snippet"], "First snippet");

        let independent = CancellationToken::new();
        let mut other_rows = Vec::new();
        append_rows(&mut other_rows, "other-book", 0, [hit("Other")], true, &independent).unwrap();
        assert_eq!(other_rows[0]["snippet"], "Other snippet");
    }

    #[test]
    fn a_retired_search_never_publishes_an_empty_success_or_archive_error() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        for mode in [Mode::Combined, Mode::Title, Mode::Fulltext] {
            for archives in [
                Vec::new(),
                vec![("book".into(), Err("Archive unavailable".into()))],
            ] {
                assert_eq!(
                    run(archives, "query", mode, 0, 50, true, &cancel),
                    Err("Search cancelled".into()),
                );
            }
        }
    }

    #[test]
    fn paging_walks_archives_in_order_without_duplicates_or_gaps() {
        let scope = vec!["a".to_string(), "b".to_string()];
        let books = [
            vec![hit("A1"), hit("A2")],
            vec![hit("B1")],
        ];
        let cancel = CancellationToken::new();
        let mut cursor = (0usize, 0u32);
        let mut seen = Vec::new();
        loop {
            let page = collect_page(&scope, cursor, 1, &cancel, |book, offset, limit, _| {
                Ok(books[book]
                    .iter()
                    .skip(offset as usize)
                    .take(limit as usize)
                    .cloned()
                    .collect())
            })
            .unwrap();
            assert!(page.errors.is_empty());
            let empty = page.hits.is_empty();
            seen.extend(page.hits.into_iter().map(|(_, hit)| hit.title));
            match page.next {
                Some(next) => cursor = next,
                None => {
                    assert!(empty);
                    break;
                }
            }
        }
        assert_eq!(seen, ["A1", "A2", "B1"]);
    }

    #[test]
    fn a_cancelled_suggestion_page_stops_between_archives() {
        let scope = vec!["a".to_string(), "b".to_string()];
        let cancel = CancellationToken::new();
        let mut fetched = 0;
        let result = collect_page(&scope, (0, 0), 5, &cancel, |_, _, _, _| {
            fetched += 1;
            cancel.cancel();
            Ok(vec![hit("Late")])
        });
        assert_eq!(result.unwrap_err(), "Search cancelled");
        assert_eq!(fetched, 1);
    }

    #[test]
    fn continuation_tokens_bind_the_prefix_scope_and_cursor() {
        let scope = vec!["book".to_string()];
        let token = suggestion_token("Eiff", &scope, 0, 15);
        assert_eq!(suggestion_cursor(&token, "Eiff", &scope).unwrap(), (0, 15));
        assert!(suggestion_cursor(&token, "Eiffe", &scope).is_err());
        assert!(suggestion_cursor(&token, "Eiff", &["other".to_string()]).is_err());
        assert!(suggestion_cursor("0:15:deadbeef", "Eiff", &scope).is_err());
        assert!(suggestion_cursor("garbage", "Eiff", &scope).is_err());
        let beyond = suggestion_token("Eiff", &scope, 0, SUGGEST_OFFSET_MAX + 1);
        assert!(suggestion_cursor(&beyond, "Eiff", &scope).is_err());
        assert!(suggestion_cursor(&suggestion_token("Eiff", &scope, 2, 0), "Eiff", &scope).is_err());
    }

    #[test]
    fn a_failed_archive_is_reported_without_blocking_later_archives() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let title = archive.title_batch(0, 1).unwrap().titles.into_iter().next().unwrap();
        let prefix: String = title.chars().take(1).collect();
        let cancel = CancellationToken::new();
        let page = suggest(
            vec![
                ("broken".to_string(), Err("Archive unavailable".into())),
                ("book".to_string(), Ok(archive)),
            ],
            &prefix,
            None,
            5,
            &cancel,
        )
        .unwrap();
        assert_eq!(page["errors"][0]["bookId"], "broken");
        assert_eq!(page["errors"][0]["error"], "Archive unavailable");
        assert!(!page["suggestions"].as_array().unwrap().is_empty());
        assert_eq!(page["suggestions"][0]["bookId"], "book");
        assert!(page["suggestions"][0].get("snippet").is_none());
    }

    #[test]
    fn combined_fixture_search_includes_native_titles_and_deduplicates_paths() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let title = archive.title_batch(0, 1).unwrap().titles.into_iter().next().unwrap();
        let prefix: String = title.chars().take(2).collect();
        let cancel = CancellationToken::new();
        let titles = archive.search_cancellable(&prefix, false, 0, PAGE, false, &cancel).unwrap();
        assert!(!titles.hits.is_empty());
        let page = run(
            vec![
                ("broken".into(), Err("Archive unavailable".into())),
                ("book".into(), Ok(archive)),
            ],
            &prefix, Mode::Combined, 0, PAGE, false, &cancel,
        ).unwrap();
        let rows = page["hits"].as_array().unwrap();
        for hit in titles.hits {
            let path = crate::server::encode_path(&hit.path);
            assert!(rows.iter().any(|row| row["bookId"] == "book" && row["path"] == path));
        }
        let paths: HashSet<_> = rows.iter().map(|row| row["path"].as_str().unwrap()).collect();
        assert_eq!(paths.len(), rows.len());
        assert!(rows.iter().all(|row| row["snippet"] == ""));
        assert_eq!(page["errors"], json!([{"bookId":"broken","error":"Archive unavailable"}]));
    }

    #[test]
    fn fixture_suggestions_page_incrementally_to_the_same_list() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/small.zim");
        let archive = Archive::open(&fixture).unwrap();
        let title = archive.title_batch(0, 1).unwrap().titles.into_iter().next().unwrap();
        let prefix: String = title.chars().take(2).collect();
        let cancel = CancellationToken::new();
        let scope = || vec![("book".to_string(), Ok(archive.clone()))];
        let complete = suggest(scope(), &prefix, None, SUGGEST_PAGE_MAX, &cancel).unwrap();
        let expected: Vec<String> = complete["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["title"].as_str().unwrap().to_string())
            .collect();
        assert!(!expected.is_empty());
        let mut paged = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..SUGGEST_PAGE_MAX {
            let page = suggest(scope(), &prefix, token.as_deref(), 1, &cancel).unwrap();
            assert_eq!(page["errors"], json!([]));
            for row in page["suggestions"].as_array().unwrap() {
                assert_eq!(row["bookId"], "book");
                assert!(row["snippet"].is_null());
                paged.push(row["title"].as_str().unwrap().to_string());
            }
            match page["nextToken"].as_str() {
                Some(next) => token = Some(next.to_string()),
                None => break,
            }
        }
        assert_eq!(paged, expected);
    }

    #[test]
    fn a_cancelled_suggestion_never_opens_an_archive() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert_eq!(
            suggest(
                vec![("book".to_string(), Err("Archive unavailable".into()))],
                "Eiff",
                None,
                5,
                &cancel,
            ),
            Err("Search cancelled".into()),
        );
    }
}
