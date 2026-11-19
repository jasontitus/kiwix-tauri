use core::iter::{FusedIterator, Rev};

/// Search for the first occurrence of a byte in a slice.
///
/// This returns the index corresponding to the first occurrence of `needle` in
/// `haystack`, or `None` if one is not found. If an index is returned, it is
/// guaranteed to be less than `haystack.len()`.
///
/// # Example
///
/// This shows how to find the first position of a byte in a byte string.
///
/// ```
/// use memchr::memchr;
///
/// let haystack = b"the quick brown fox";
/// assert_eq!(memchr(b'k', haystack), Some(8));
/// ```
#[inline]
pub fn memchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    Bytes::new([needle]).find(haystack)
}

/// Search for the last occurrence of a byte in a slice.
///
/// This returns the index corresponding to the last occurrence of `needle` in
/// `haystack`, or `None` if one is not found. If an index is returned, it is
/// guaranteed to be less than `haystack.len()`.
///
/// # Example
///
/// This shows how to find the last position of a byte in a byte string.
///
/// ```
/// use memchr::memrchr;
///
/// let haystack = b"the quick brown fox";
/// assert_eq!(memrchr(b'o', haystack), Some(17));
/// ```
#[inline]
pub fn memrchr(needle: u8, haystack: &[u8]) -> Option<usize> {
    Bytes::new([needle]).rfind(haystack)
}

/// Search for the first occurrence of two possible bytes in a haystack.
///
/// This returns the index corresponding to the first occurrence of one of the
/// needle bytes in `haystack`, or `None` if one is not found. If an index is
/// returned, it is guaranteed to be less than `haystack.len()`.
///
/// # Example
///
/// This shows how to find the first position of one of two possible bytes in a
/// haystack.
///
/// ```
/// use memchr::memchr2;
///
/// let haystack = b"the quick brown fox";
/// assert_eq!(memchr2(b'k', b'q', haystack), Some(4));
/// ```
#[inline]
pub fn memchr2(needle1: u8, needle2: u8, haystack: &[u8]) -> Option<usize> {
    Bytes::new([needle1, needle2]).find(haystack)
}

/// Search for the last occurrence of two possible bytes in a haystack.
///
/// This returns the index corresponding to the last occurrence of one of the
/// needle bytes in `haystack`, or `None` if one is not found. If an index is
/// returned, it is guaranteed to be less than `haystack.len()`.
///
/// # Example
///
/// This shows how to find the last position of one of two possible bytes in a
/// haystack.
///
/// ```
/// use memchr::memrchr2;
///
/// let haystack = b"the quick brown fox";
/// assert_eq!(memrchr2(b'k', b'o', haystack), Some(17));
/// ```
#[inline]
pub fn memrchr2(needle1: u8, needle2: u8, haystack: &[u8]) -> Option<usize> {
    Bytes::new([needle1, needle2]).rfind(haystack)
}

/// Search for the first occurrence of three possible bytes in a haystack.
///
/// This returns the index corresponding to the first occurrence of one of the
/// needle bytes in `haystack`, or `None` if one is not found. If an index is
/// returned, it is guaranteed to be less than `haystack.len()`.
///
/// # Example
///
/// This shows how to find the first position of one of three possible bytes in
/// a haystack.
///
/// ```
/// use memchr::memchr3;
///
/// let haystack = b"the quick brown fox";
/// assert_eq!(memchr3(b'k', b'q', b'u', haystack), Some(4));
/// ```
#[inline]
pub fn memchr3(
    needle1: u8,
    needle2: u8,
    needle3: u8,
    haystack: &[u8],
) -> Option<usize> {
    Bytes::new([needle1, needle2, needle3]).find(haystack)
}

/// Search for the last occurrence of three possible bytes in a haystack.
///
/// This returns the index corresponding to the last occurrence of one of the
/// needle bytes in `haystack`, or `None` if one is not found. If an index is
/// returned, it is guaranteed to be less than `haystack.len()`.
///
/// # Example
///
/// This shows how to find the last position of one of three possible bytes in
/// a haystack.
///
/// ```
/// use memchr::memrchr3;
///
/// let haystack = b"the quick brown fox";
/// assert_eq!(memrchr3(b'k', b'o', b'n', haystack), Some(17));
/// ```
#[inline]
pub fn memrchr3(
    needle1: u8,
    needle2: u8,
    needle3: u8,
    haystack: &[u8],
) -> Option<usize> {
    Bytes::new([needle1, needle2, needle3]).rfind(haystack)
}

/// Returns an iterator over all occurrences of the needle in a haystack.
///
/// The iterator returned implements `DoubleEndedIterator`. This means it
/// can also be used to find occurrences in reverse order.
#[inline]
pub fn memchr_iter<'h>(needle: u8, haystack: &'h [u8]) -> Memchr<'h> {
    Memchr::new(needle, haystack)
}

/// Returns an iterator over all occurrences of the needle in a haystack, in
/// reverse.
#[inline]
pub fn memrchr_iter(needle: u8, haystack: &[u8]) -> Rev<Memchr<'_>> {
    Memchr::new(needle, haystack).rev()
}

/// Returns an iterator over all occurrences of the needles in a haystack.
///
/// The iterator returned implements `DoubleEndedIterator`. This means it
/// can also be used to find occurrences in reverse order.
#[inline]
pub fn memchr2_iter<'h>(
    needle1: u8,
    needle2: u8,
    haystack: &'h [u8],
) -> Memchr2<'h> {
    Memchr2::new(needle1, needle2, haystack)
}

/// Returns an iterator over all occurrences of the needles in a haystack, in
/// reverse.
#[inline]
pub fn memrchr2_iter(
    needle1: u8,
    needle2: u8,
    haystack: &[u8],
) -> Rev<Memchr2<'_>> {
    Memchr2::new(needle1, needle2, haystack).rev()
}

/// Returns an iterator over all occurrences of the needles in a haystack.
///
/// The iterator returned implements `DoubleEndedIterator`. This means it
/// can also be used to find occurrences in reverse order.
#[inline]
pub fn memchr3_iter<'h>(
    needle1: u8,
    needle2: u8,
    needle3: u8,
    haystack: &'h [u8],
) -> Memchr3<'h> {
    Memchr3::new(needle1, needle2, needle3, haystack)
}

/// Returns an iterator over all occurrences of the needles in a haystack, in
/// reverse.
#[inline]
pub fn memrchr3_iter(
    needle1: u8,
    needle2: u8,
    needle3: u8,
    haystack: &[u8],
) -> Rev<Memchr3<'_>> {
    Memchr3::new(needle1, needle2, needle3, haystack).rev()
}

pub(crate) const WORD_BYTES: usize = core::mem::size_of::<usize>();

const ONES: usize = usize::MAX / 255;
const LOW_BITS: usize = 0x7f * ONES;
const HIGH_BITS: usize = 0x80 * ONES;

#[inline]
pub(crate) fn splat(needle: u8) -> usize {
    usize::from(needle) * ONES
}

#[inline]
pub(crate) fn read_word(chunk: &[u8]) -> usize {
    // Callers take exact word-sized chunks. Explicit little-endian order makes
    // the low bit of the mask correspond to the first byte on every target.
    usize::from_le_bytes(chunk.try_into().expect("complete byte-search word"))
}

#[inline]
pub(crate) fn equal_mask(word: usize, needle: usize) -> usize {
    // Set the high bit of each matching byte independently. Masking off the
    // high bits before adding 0x7f prevents carries between byte lanes.
    let diff = word ^ needle;
    !((diff & LOW_BITS).wrapping_add(LOW_BITS) | diff | LOW_BITS) & HIGH_BITS
}

#[inline]
pub(crate) fn first_lane(mask: usize) -> usize {
    mask.trailing_zeros() as usize / 8
}

#[derive(Clone, Debug)]
struct Bytes<const N: usize> {
    needles: [u8; N],
    words: [usize; N],
}

impl<const N: usize> Bytes<N> {
    #[inline]
    fn new(needles: [u8; N]) -> Self {
        Self { needles, words: needles.map(splat) }
    }

    #[inline]
    fn mask(&self, chunk: &[u8]) -> usize {
        let word = read_word(chunk);
        self.words
            .iter()
            .fold(0, |mask, &needle| mask | equal_mask(word, needle))
    }

    #[inline]
    fn find(&self, haystack: &[u8]) -> Option<usize> {
        let mut chunks = haystack.chunks_exact(WORD_BYTES);
        for (i, chunk) in chunks.by_ref().enumerate() {
            let mask = self.mask(chunk);
            if mask != 0 {
                return Some(i * WORD_BYTES + first_lane(mask));
            }
        }
        let tail = chunks.remainder();
        tail.iter()
            .position(|byte| self.needles.contains(byte))
            .map(|i| haystack.len() - tail.len() + i)
    }

    #[inline]
    fn rfind(&self, haystack: &[u8]) -> Option<usize> {
        let chunks = haystack.chunks_exact(WORD_BYTES);
        let tail = chunks.remainder();
        if let Some(i) =
            tail.iter().rposition(|byte| self.needles.contains(byte))
        {
            return Some(haystack.len() - tail.len() + i);
        }
        for (i, chunk) in chunks.enumerate().rev() {
            let mask = self.mask(chunk);
            if mask != 0 {
                let lane = WORD_BYTES - 1 - mask.leading_zeros() as usize / 8;
                return Some(i * WORD_BYTES + lane);
            }
        }
        None
    }

    #[inline]
    fn count(&self, haystack: &[u8]) -> usize {
        let chunks = haystack.chunks_exact(WORD_BYTES);
        let tail = chunks.remainder();
        chunks
            .map(|chunk| self.mask(chunk).count_ones() as usize)
            .sum::<usize>()
            + tail.iter().filter(|byte| self.needles.contains(byte)).count()
    }
}

macro_rules! define_iter {
    ($name:ident, $len:expr, $($needle:ident),+) => {
        /// An iterator over byte offsets matching any of the supplied needles.
        #[derive(Clone, Debug)]
        pub struct $name<'h> {
            bytes: Bytes<$len>,
            remaining: &'h [u8],
            start: usize,
        }

        impl<'h> $name<'h> {
            /// Search the haystack from both ends for the supplied byte needles.
            #[inline]
            pub fn new($($needle: u8,)+ haystack: &'h [u8]) -> Self {
                Self { bytes: Bytes::new([$($needle),+]), remaining: haystack, start: 0 }
            }
        }

        impl Iterator for $name<'_> {
            type Item = usize;

            #[inline]
            fn next(&mut self) -> Option<usize> {
                let i = match self.bytes.find(self.remaining) {
                    Some(i) => i,
                    None => {
                        self.remaining = &[];
                        return None;
                    }
                };
                let found = self.start + i;
                self.remaining = &self.remaining[i + 1..];
                self.start = found + 1;
                Some(found)
            }

            #[inline]
            fn size_hint(&self) -> (usize, Option<usize>) {
                (0, Some(self.remaining.len()))
            }

            #[inline]
            fn count(self) -> usize {
                self.bytes.count(self.remaining)
            }
        }

        impl DoubleEndedIterator for $name<'_> {
            #[inline]
            fn next_back(&mut self) -> Option<usize> {
                let i = match self.bytes.rfind(self.remaining) {
                    Some(i) => i,
                    None => {
                        self.remaining = &[];
                        return None;
                    }
                };
                self.remaining = &self.remaining[..i];
                Some(self.start + i)
            }
        }

        impl FusedIterator for $name<'_> {}
    };
}

define_iter!(Memchr, 1, needle);
define_iter!(Memchr2, 2, needle1, needle2);
define_iter!(Memchr3, 3, needle1, needle2, needle3);

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn mixed_direction_and_count_at_every_word_boundary() {
        let pattern = [0, 1, 0, 0xff, 0x7f, 0x80, 2, 0xff, 1, 2, 0];
        let input: Vec<_> = pattern.into_iter().cycle().take(79).collect();
        for start in 0..2 * WORD_BYTES {
            for end in start..=input.len() {
                let haystack = &input[start..end];
                for needles in [[0, 0, 0xff], [1, 2, 0x80], [3, 19, 0x7f]] {
                    let [a, b, c] = needles;
                    let expected: Vec<_> = haystack
                        .iter()
                        .enumerate()
                        .filter_map(|(i, byte)| {
                            needles.contains(byte).then(|| i)
                        })
                        .collect();
                    let mut actual = memchr3_iter(a, b, c, haystack);
                    assert_eq!(
                        memchr3(a, b, c, haystack),
                        expected.first().copied()
                    );
                    assert_eq!(
                        memrchr3(a, b, c, haystack),
                        expected.last().copied()
                    );
                    assert_eq!(actual.clone().collect::<Vec<_>>(), expected);
                    assert_eq!(actual.clone().count(), expected.len());
                    let mut remaining = expected.into_iter();
                    while !remaining.as_slice().is_empty() {
                        assert_eq!(actual.next(), remaining.next());
                        assert_eq!(actual.next_back(), remaining.next_back());
                        assert_eq!(actual.clone().count(), remaining.len());
                    }
                    assert_eq!(actual.next_back(), None);
                    assert_eq!(actual.next(), None);
                }
            }
        }
    }

    #[test]
    fn single_and_duplicate_needles_agree_with_scalar_count() {
        let haystack = [0, 1, 0, 1, 0, 1, 1, 0xff, 1, 0, 2, 1, 1, 0, 0, 1, 0];
        for byte in 0..=u8::MAX {
            let expected = haystack.iter().filter(|&&b| b == byte).count();
            assert_eq!(memchr_iter(byte, &haystack).count(), expected);
            assert_eq!(memchr2_iter(byte, byte, &haystack).count(), expected);
            assert_eq!(memrchr_iter(byte, &haystack).count(), expected);
        }
    }
}
