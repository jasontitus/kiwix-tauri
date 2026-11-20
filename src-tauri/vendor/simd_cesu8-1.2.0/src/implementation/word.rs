// Modified by Kiwix Tauri. Read words from checked slice chunks.

use core::mem;

use super::fallback;

#[must_use]
#[inline]
pub fn contains_null_or_utf8_4_byte_char_header(value: &[u8]) -> bool {
    test_word_any(
        value,
        fallback::contains_null_or_utf8_4_byte_char_header,
        word_contains_null_or_utf8_4_byte_char_header,
    )
}

#[must_use]
#[inline]
fn word_contains_null_or_utf8_4_byte_char_header(word: usize) -> bool {
    word_contains_null_byte(word) || word_contains_utf8_4_byte_char_header(word)
}

#[must_use]
#[inline]
pub fn contains_utf8_4_byte_char_header(value: &[u8]) -> bool {
    test_word_any(
        value,
        fallback::contains_utf8_4_byte_char_header,
        word_contains_utf8_4_byte_char_header,
    )
}

#[must_use]
#[inline]
fn word_contains_utf8_4_byte_char_header(word: usize) -> bool {
    const MASK: usize = usize_repeat_u8(0b1111_1000);
    const HEADER: usize = usize_repeat_u8(0b1111_0000);

    word_contains_null_byte((word & MASK) ^ HEADER)
}

#[must_use]
#[inline]
fn word_contains_null_byte(word: usize) -> bool {
    const LOW_MASK: usize = usize_repeat_u8(0x01);
    const HIGH_MASK: usize = usize_repeat_u8(0x80);

    (word.wrapping_sub(LOW_MASK) & !word & HIGH_MASK) != 0
}

#[must_use]
#[inline]
const fn usize_repeat_u8(byte: u8) -> usize {
    usize::from_ne_bytes([byte; USIZE_SIZE])
}

/// Tests complete native-width words and then the remaining bytes.
#[must_use]
#[inline]
fn test_word_any<F, T>(value: &[u8], fallback: F, test: T) -> bool
where
    F: Fn(&[u8]) -> bool,
    T: Fn(usize) -> bool,
{
    let mut words = value.chunks_exact(USIZE_SIZE);
    for chunk in words.by_ref() {
        let bytes = chunk.try_into().expect("native-width chunk");
        if test(usize::from_ne_bytes(bytes)) {
            return true;
        }
    }
    fallback(words.remainder())
}

const USIZE_SIZE: usize = mem::size_of::<usize>();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_scans_cover_unaligned_slices_and_incomplete_words() {
        let width = USIZE_SIZE;
        for byte in 0..=u8::MAX {
            for start in 0..width {
                for index in start..start + 2 * width + 1 {
                    let mut data = [b'A'; 4 * USIZE_SIZE];
                    data[index] = byte;
                    let slice = &data[start..start + 2 * width + 1];
                    assert_eq!(
                        contains_null_or_utf8_4_byte_char_header(slice),
                        fallback::contains_null_or_utf8_4_byte_char_header(slice),
                    );
                    assert_eq!(
                        contains_utf8_4_byte_char_header(slice),
                        fallback::contains_utf8_4_byte_char_header(slice),
                    );
                }
            }
        }
    }
}
