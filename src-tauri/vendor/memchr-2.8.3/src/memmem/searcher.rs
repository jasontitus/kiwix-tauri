use crate::{
    arch::all::{packedpair, twoway},
    memchr, memrchr,
};

#[derive(Clone, Debug)]
pub(crate) enum Searcher {
    Empty,
    Byte(u8),
    TwoWay(twoway::Finder, Option<packedpair::Finder>),
}

impl Searcher {
    pub(crate) fn new<R: packedpair::HeuristicFrequencyRank>(
        config: PrefilterConfig,
        ranker: R,
        needle: &[u8],
    ) -> Self {
        match needle {
            [] => Self::Empty,
            &[byte] => Self::Byte(byte),
            _ => {
                let prefilter = if config.is_none() {
                    None
                } else {
                    packedpair::Pair::with_ranker(needle, ranker).and_then(
                        |pair| packedpair::Finder::with_pair(needle, pair),
                    )
                };
                Self::TwoWay(twoway::Finder::new(needle), prefilter)
            }
        }
    }

    #[inline]
    pub(crate) fn find(
        &self,
        state: &mut PrefilterState,
        haystack: &[u8],
        needle: &[u8],
    ) -> Option<usize> {
        match *self {
            Self::Empty => Some(0),
            Self::Byte(byte) => memchr(byte, haystack),
            Self::TwoWay(finder, prefilter) => {
                if haystack.len() < 64 {
                    return short_find(haystack, needle);
                }
                let pre = prefilter.as_ref().map(|strategy| Pre {
                    prestate: state,
                    prestrat: strategy,
                });
                finder.find_with_prefilter(pre, haystack, needle)
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) enum SearcherRev {
    Empty,
    Byte(u8),
    TwoWay(twoway::FinderRev),
}

impl SearcherRev {
    pub(crate) fn new(needle: &[u8]) -> Self {
        match needle {
            [] => Self::Empty,
            &[byte] => Self::Byte(byte),
            _ => Self::TwoWay(twoway::FinderRev::new(needle)),
        }
    }

    #[inline]
    pub(crate) fn rfind(
        &self,
        haystack: &[u8],
        needle: &[u8],
    ) -> Option<usize> {
        match *self {
            Self::Empty => Some(haystack.len()),
            Self::Byte(byte) => memrchr(byte, haystack),
            Self::TwoWay(_) if haystack.len() < 64 => {
                short_rfind(haystack, needle)
            }
            Self::TwoWay(finder) => finder.rfind(haystack, needle),
        }
    }
}

// Both callers bound the haystack length, so even repeated matches of a
// long needle prefix cannot make this scan superlinear.
#[inline]
pub(crate) fn short_find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    match needle {
        [] => Some(0),
        &[byte] => memchr(byte, haystack),
        _ => {
            haystack.windows(needle.len()).position(|window| window == needle)
        }
    }
}

#[inline]
pub(crate) fn short_rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    match needle {
        [] => Some(haystack.len()),
        &[byte] => memrchr(byte, haystack),
        _ => {
            haystack.windows(needle.len()).rposition(|window| window == needle)
        }
    }
}

/// Controls the use of a forward substring search prefilter.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum PrefilterConfig {
    /// Never use a prefilter in substring search.
    None,
    /// Automatically detect whether a heuristic prefilter should be used. If
    /// it is used, then heuristics will be used to dynamically disable the
    /// prefilter if it is believed to not be carrying its weight.
    Auto,
}

impl Default for PrefilterConfig {
    fn default() -> PrefilterConfig {
        PrefilterConfig::Auto
    }
}

impl PrefilterConfig {
    /// Returns true when this prefilter is set to the `None` variant.
    fn is_none(&self) -> bool {
        matches!(*self, PrefilterConfig::None)
    }
}

/// Per-search history for deciding when to bypass the prefilter.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PrefilterState {
    /// The number of skips that has been executed. This is always 1 greater
    /// than the actual number of skips. The special sentinel value of 0
    /// indicates that the prefilter is inert. This is useful to avoid
    /// additional checks to determine whether the prefilter is still
    /// "effective." Once a prefilter becomes inert, it should no longer be
    /// used (according to our heuristics).
    skips: u32,
    /// The total number of bytes that have been skipped.
    skipped: u32,
}

impl PrefilterState {
    /// The minimum number of skip attempts to try before considering whether
    /// a prefilter is effective or not.
    const MIN_SKIPS: u32 = 50;

    /// The minimum amount of bytes that skipping must average.
    ///
    /// This value was chosen based on varying it and checking
    /// the microbenchmarks. In particular, this can impact the
    /// pathological/repeated-{huge,small} benchmarks quite a bit if it's set
    /// too low.
    const MIN_SKIP_BYTES: u32 = 8;

    /// Create a fresh prefilter state.
    #[inline]
    pub(crate) fn new() -> PrefilterState {
        PrefilterState { skips: 1, skipped: 0 }
    }

    /// Update this state with the number of bytes skipped on the last
    /// invocation of the prefilter.
    #[inline]
    fn update(&mut self, skipped: usize) {
        self.skips = self.skips.saturating_add(1);
        // We need to do this dance since it's technically possible for
        // `skipped` to overflow a `u32`. (And we use a `u32` to reduce the
        // size of a prefilter state.)
        self.skipped = match u32::try_from(skipped) {
            Err(_) => u32::MAX,
            Ok(skipped) => self.skipped.saturating_add(skipped),
        };
    }

    /// Return true if and only if this state indicates that a prefilter is
    /// still effective.
    #[inline]
    fn is_effective(&mut self) -> bool {
        if self.is_inert() {
            return false;
        }
        if self.skips() < PrefilterState::MIN_SKIPS {
            return true;
        }
        if self.skipped / self.skips() >= PrefilterState::MIN_SKIP_BYTES {
            return true;
        }

        // We're inert.
        self.skips = 0;
        false
    }

    /// Returns true if the prefilter this state represents should no longer
    /// be used.
    #[inline]
    fn is_inert(&self) -> bool {
        self.skips == 0
    }

    /// Returns the total number of times the prefilter has been used.
    #[inline]
    fn skips(&self) -> u32 {
        // Remember, `0` is a sentinel value indicating inertness, so we
        // always need to subtract `1` to get our actual number of skips.
        self.skips.saturating_sub(1)
    }
}

/// A combination of prefilter effectiveness state and the prefilter itself.
#[derive(Debug)]
pub(crate) struct Pre<'a> {
    /// State that tracks the effectiveness of a prefilter.
    prestate: &'a mut PrefilterState,
    /// The actual prefilter.
    prestrat: &'a packedpair::Finder,
}

impl<'a> Pre<'a> {
    /// Return the next candidate and account for the bytes skipped.
    #[inline]
    pub(crate) fn find(&mut self, haystack: &[u8]) -> Option<usize> {
        let result = self.prestrat.find_prefilter(haystack);
        self.prestate.update(result.unwrap_or(haystack.len()));
        result
    }

    /// Return true if and only if this prefilter should be used.
    #[inline]
    pub(crate) fn is_effective(&mut self) -> bool {
        self.prestate.is_effective()
    }
}
