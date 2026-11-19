//! Architecture-independent search routines.

pub mod packedpair;
#[cfg(feature = "alloc")]
pub mod shiftor;
pub mod twoway;

/// Returns true if the needle is a prefix of the haystack.
#[inline]
pub fn is_prefix(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.starts_with(needle)
}

/// Returns true if the needle is a suffix of the haystack.
#[inline]
pub fn is_suffix(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.ends_with(needle)
}

/// Returns true if both slices contain the same bytes.
#[inline]
pub fn is_equal(x: &[u8], y: &[u8]) -> bool {
    x == y
}
