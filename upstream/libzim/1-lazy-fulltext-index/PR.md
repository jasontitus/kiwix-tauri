# Keep the lazily loaded fulltext Xapian database

Refs #1127 (item 7)

## Problem

If an archive is opened with `zim::OpenConfig().preloadXapianDb(false)`, full-text search never works on it:

- `Archive::hasFulltextIndex()` returns `true`;
- every `Searcher::search()` on that archive throws `std::runtime_error("Cannot create Search without FT Xapian index")`. This applies to every `Searcher` and lasts as long as the archive does.

When a `Searcher` covers several archives, `InternalDataBase` (`src/search.cpp`) skips any archive whose `getXapianDb()` returns null. A lazily opened archive is therefore left out of the results without any error. I found this by reading the code and did not test it.

Archives opened with the default configuration (`preloadXapianDb(true)`) are not affected. The bug has been present since `preloadXapianDb()` was added in 9.4.0 (#958).

#1127 describes the symptom as "always returns empty results". With a single archive, what I observed is the exception above.

## Root cause

`FileImpl::getXapianDb()` (`src/fileimpl.cpp`) performs the lazy, double-checked initialisation. It calls `loadXapianDb()` but does not assign the result, then sets `m_xapianDbCreated` to `true`. `mp_xapianDb` therefore stays null permanently. The preload path in the `FileImpl` constructor assigns the result correctly: `mp_xapianDb = loadXapianDb();`.

## Fix

Assign the result of `loadXapianDb()` to `mp_xapianDb` on the lazy path. This is a one-line change.

## Title index / suggestions

The suggestion path does not have this bug. `SuggestionSearcher` opens the title index itself in `SuggestionDataBase::initXapianDb()` (`src/suggestion.cpp`) and does not go through `getXapianDb()`.

I checked this with an ad hoc assertion that is not part of this PR. I ran it without the fix on an archive opened with `preloadXapianDb(false)`:

- `hasTitleIndex()` returned `true`;
- `SuggestionSearcher::suggest()` returned the expected entry;
- in the same test, `Searcher::search()` still threw.

## Tests

`test/search.cpp`, `Search.fulltextIndexNotPreloaded`:

1. Creates an indexed archive with the writer.
2. Opens it with `preloadXapianDb(false)`.
3. Checks `hasFulltextIndex()`.
4. Searches twice, with two different `Searcher` objects (the second must reuse the database loaded by the first), and expects exactly one result with the right path.

How to run:

```bash
meson setup build -Dtest_data_dir=<zim-testing-suite dir>
ninja -C build
build/test/search --gtest_filter=Search.fulltextIndexNotPreloaded
meson test -C build
```

The test creates its own archive and does not need the test data.

## Evidence

Environment: Linux x86_64 (Ubuntu 24.04), GCC 13.3, Xapian 1.4.22, zstd 1.5.5, ICU 74, zim-testing-suite 0.10.0. Base: `ad888b4`.

New test without the fix (only `src/fileimpl.cpp` reverted):

```
[ RUN      ] Search.fulltextIndexNotPreloaded
unknown file: Failure
C++ exception with description "Cannot create Search without FT Xapian index" thrown in the test body.
[  FAILED  ] Search.fulltextIndexNotPreloaded (158 ms)
```

The `ASSERT_TRUE(archive.hasFulltextIndex())` before the search passes even without the fix.

With the fix:

```
[  PASSED  ] 1 test.
```

Full suite with the fix: `meson test`: 31/31 OK.

## Risk / compatibility

- The lazy path now behaves like the preload path. No API or ABI change.
- Thread safety is unchanged:
  - The assignment happens under `m_xapianDbCreationMutex`, before the release-store of `m_xapianDbCreated`.
  - Readers do an acquire-load of `m_xapianDbCreated` before they read `mp_xapianDb`.

## Not verified

- Only tested on Linux x86_64.
- The multi-archive `Searcher` case was checked by reading the code only.
