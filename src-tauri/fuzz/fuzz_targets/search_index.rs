#![no_main]
use libfuzzer_sys::fuzz_target;
use zimfile_xapian::{Index, ReaderLimits};

// The limits `IndexSlot::open` in src-tauri/src/archive_search.rs gives the
// reader; keep the two in step.
const MAX_QUERY: usize = 64 * 1024;

// The whole input is one embedded Glass index, as the app reads it out of an
// archive's `X/fulltext/xapian` or `X/title/xapian` entry. The queries name
// articles of the seed fixture so a seed starts with real postings.
fuzz_target!(|data: &[u8]| {
    let limits = ReaderLimits { max_query_bytes: MAX_QUERY, ..Default::default() };
    let Ok(index) = Index::open_with_limits(Box::new(data.to_vec()), limits) else { return; };
    let _ = index.metadata("kind");
    let _ = index.metadata("language");
    let _ = index.search_page("venus earth sky", 0, 10, "eng", true, &|| false);
    let _ = index.suggest_page("ve", 0, 10, "eng", &|| false);
});
