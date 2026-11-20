// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]
//! Capability-scoped LAN article search. No app IPC, profile data or sensors.
use super::*;
use crate::search::{self, Mode};

type SharedArchive = (String, Result<Archive, String>);

#[derive(Clone)]
pub(super) enum Scope { Selected(Index), Library(LibraryIndex) }
impl Scope {
    fn token(&self) -> &str { match self { Self::Selected(s) => &s.token, Self::Library(s) => &s.token } }
    fn cancellation(&self) -> &SharingCancellation { match self { Self::Selected(s) => &s.cancellation, Self::Library(s) => &s.cancellation } }
    fn books(&self) -> Result<Vec<(String, String)>, String> {
        Ok(match self {
            Self::Selected(s) => s.entries.iter().map(|b| (b.id.clone(), b.title.clone())).collect(),
            Self::Library(s) => available_books(s)?.into_iter().map(|b| (b.metadata.id, b.metadata.title)).collect(),
        })
    }
    fn archives(&self, selected: &str) -> Result<Vec<SharedArchive>, String> {
        search::check_cancel(&self.cancellation().0.search_cancel)?;
        match self {
            Self::Selected(s) => {
                if !selected.is_empty() && !s.entries.iter().any(|b| b.id == selected) { return Err("Archive is not shared".into()); }
                Ok(s.entries.iter().filter(|b| selected.is_empty() || selected == b.id).map(|b| {
                    (b.id.clone(), b.archive.upgrade().map(|a| (*a).clone()).ok_or_else(|| "Archive is no longer shared".into()))
                }).collect())
            }
            Self::Library(s) => {
                let mut archives = Vec::new();
                if !selected.is_empty() {
                    // A selected archive is resolved once from the same live
                    // record the availability list would have produced.
                    search::check_cancel(&s.cancellation.0.search_cancel)?;
                    let book = available_book(s, selected)?;
                    let id = book.metadata.id.clone();
                    let archive = library_archive_record(s, &book).map(|a| (*a).clone()).map_err(|e| e.message);
                    archives.push((id, archive));
                    return Ok(archives);
                }
                for book in available_books(s)? {
                    search::check_cancel(&s.cancellation.0.search_cancel)?;
                    let id = book.metadata.id.clone();
                    let archive = library_archive_record(s, &book).map(|a| (*a).clone()).map_err(|e| e.message);
                    archives.push((id, archive));
                }
                Ok(archives)
            }
        }
    }
    fn href(&self, host: IpAddr, id: &str, path: &str) -> Option<String> {
        let path = format!("/{path}"); // run/suggest already encode the archive path exactly once.
        let query = url::form_urlencoded::Serializer::new(String::new()).append_pair("path", &path).finish();
        Some(match self {
            Self::Selected(s) => access_url(host, s.entries.iter().find(|b| b.id == id)?.port, &format!("/.kiwix-access/{}?{query}", s.token)),
            Self::Library(s) => format!("/{}/open/{}?{query}", s.token, query_value(id)),
        })
    }
}
struct Query { q: String, book: String, mode: Mode, offset: u32, token: Option<String> }
fn parse(uri: &Uri, suggest: bool) -> Result<Query, String> {
    let raw = uri.query().unwrap_or("");
    if raw.len() > 8192 { return Err("Search URL is too long".into()); }
    let mut values = HashMap::new();
    for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
        if !matches!(key.as_ref(), "q" | "book" | "mode" | "offset" | "token") || values.insert(key.into_owned(), value.into_owned()).is_some() {
            return Err("Unknown or repeated search parameter".into());
        }
    }
    let q = values.remove("q").unwrap_or_default();
    if q.len() > QUERY_BYTES || q.chars().count() > QUERY_CHARS || q.chars().any(char::is_control) { return Err("Search must be at most 256 characters and 1024 bytes, without control characters".into()); }
    let book = values.remove("book").unwrap_or_default();
    if book.len() > 256 { return Err("Archive identifier is too long".into()); }
    let mode = Mode::parse(values.get("mode").map(String::as_str))?;
    let offset = values.get("offset").map(|v| v.parse::<u32>()).transpose().map_err(|_| "Invalid search offset")?.unwrap_or(0);
    let token = values.remove("token");
    if offset > 10000 || token.as_ref().is_some_and(|t| t.len() > 4096) || (suggest && offset != 0) || (!suggest && token.is_some()) { return Err("Invalid search continuation".into()); }
    Ok(Query { q, book, mode, offset, token })
}

pub(super) async fn request(scope: Scope, rest: &str, host: IpAddr, uri: &Uri, headers: &HeaderMap) -> Response {
    if rest == "reader.js" {
        return Response::builder().header("Content-Type", "text/javascript; charset=utf-8").header("Cache-Control", "no-store").header("X-Content-Type-Options", "nosniff").body(Body::from(include_str!("sharing_reader.js"))).unwrap();
    }
    if rest == "reader" { return reader_page(&scope, headers); }
    let suggest = rest == "suggest";
    let query = match parse(uri, suggest) { Ok(q) => q, Err(e) => return (StatusCode::BAD_REQUEST, e).into_response() };
    let cancellation = scope.cancellation().clone();
    let permit = match cancellation.0.searches.clone().try_acquire_owned() { Ok(p) => p, Err(_) => return (StatusCode::TOO_MANY_REQUESTS, "Search is busy; try again shortly").into_response() };
    let worker_scope = scope.clone();
    let stopped = cancellation.clone();
    let result = cancellation.0.workers.run(move || {
        let _permit = permit;
        stopped.native(|| {
            let archives = worker_scope.archives(&query.book)?;
            if suggest { search::suggest(archives, query.q.trim(), query.token.as_deref(), 20, &stopped.0.search_cancel) }
            else { search::run(archives, query.q.trim(), query.mode, query.offset, 20, true, &stopped.0.search_cancel) }
        }).map_err(|e| e.to_string())?
    }).await;
    match result {
        Ok(mut value) => {
            // Only supply links to this capability's archives, even if native
            // output is malformed. Never accept archive-supplied URL schemes.
            for key in ["hits", "suggestions"] {
                if let Some(rows) = value.get_mut(key).and_then(Value::as_array_mut) {
                    for row in rows {
                        if let (Some(id), Some(path)) = (row["bookId"].as_str(), row["path"].as_str()) {
                            if let Some(href) = scope.href(host, id, path) { row["href"] = json!(href); }
                        }
                    }
                }
            }
            if !suggest && value["hits"].as_array().is_some_and(Vec::is_empty) {
                let store = match &scope { Scope::Selected(s) => &s.store, Scope::Library(s) => &s.store };
                if let Ok(locale) = request_locale(store, headers) {
                    value["message"] = json!(locale.text("No matching articles.", &[]));
                }
            }
            Response::builder().header("Content-Type", "application/json; charset=utf-8").header("Cache-Control", "no-store").header("Referrer-Policy", "no-referrer").header("X-Content-Type-Options", "nosniff").body(Body::from(value.to_string())).unwrap()
        }
        Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
    }
}
fn reader_page(scope: &Scope, headers: &HeaderMap) -> Response {
    let store = match scope { Scope::Selected(s) => &s.store, Scope::Library(s) => &s.store };
    let locale = match request_locale(store, headers) { Ok(l) => l, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response() };
    let t = |text: &str| escape(&locale.text(text, &[]));
    let books = match scope.books() { Ok(b) => b, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response() };
    let mut body = format!("<!doctype html><html lang=\"{}\" dir=\"{}\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>:root{{color-scheme:light;--ink:#183b35;--paper:#f6f2e8}}body{{max-width:900px;margin:auto;padding:2rem 1.5rem;background:var(--paper);color:var(--ink);font:18px/1.6 Georgia,serif}}h1{{border-top:8px solid var(--ink);padding-top:1rem;font-size:3rem;line-height:1.1}}a{{color:var(--ink)}}form{{display:flex;gap:1rem;flex-wrap:wrap;border-block:1px solid #c3cdc1;padding:1rem 0}}label{{display:flex;flex-direction:column}}input,select,button{{font:inherit;padding:.5rem;max-width:100%}}button{{cursor:pointer}}li{{padding:.5rem 0}}small{{display:block}}.note{{color:#4d625c}}[hidden]{{display:none}}</style><a href=\"/{}/\">{}</a><h1>{}</h1><form id=\"search\" role=\"search\"><label>{}<input id=\"q\" name=\"q\" type=\"search\" maxlength=\"256\" autocomplete=\"off\" aria-controls=\"suggestions\" required></label><label>{}<select id=\"book\"><option value=\"\">{}</option>", escape(locale.lang()), escape(locale.dir()), t("Search articles"), scope.token(), t("Browse the collection"), t("Search articles"), t("Search"), t("Archive"), t("All shared items"));
    for (id, title) in books { body.push_str(&format!("<option value=\"{}\">{}</option>", escape(&id), escape(&title))); }
    body.push_str(&format!("</select></label><label>{}<select id=\"mode\"><option value=\"combined\">{}</option><option value=\"title\">{}</option><option value=\"fulltext\">{}</option></select></label><button>{}</button></form><ul id=\"suggestions\" aria-label=\"{}\"></ul><p id=\"status\" role=\"status\" aria-live=\"polite\"></p><ol id=\"results\"></ol><p id=\"limit\" class=\"note\" role=\"status\" hidden>{}</p><button id=\"more\" hidden>{}</button><script src=\"/{}/reader.js\" defer></script></html>", t("Search mode"), t("Titles and full text"), t("Titles"), t("Full text"), t("Search"), t("Title suggestions"), t("Refine your search to see more results."), t("More results"), scope.token()));
    let mut response = page(body);
    response.headers_mut().insert("Content-Security-Policy", axum::http::HeaderValue::from_static("default-src 'none'; script-src 'self'; connect-src 'self'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'; worker-src 'none'"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queries_reject_scope_and_continuation_ambiguity() {
        for query in ["q=x&q=y", "book=x&book=y", "offset=10001", "q=%00", "mode=unknown", "token=x"] {
            assert!(parse(&format!("/token/search?{query}").parse().unwrap(), false).is_err(), "{query}");
        }
        assert!(parse(&"/token/suggest?q=a&offset=1".parse().unwrap(), true).is_err());
        assert!(parse(&"/token/search?q=a%26b&mode=combined&offset=20".parse().unwrap(), false).is_ok());
    }
}
