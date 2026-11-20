// Modified by Kiwix Tauri from the portable httparse token scanners.

#![forbid(unsafe_code)]

use crate::{is_header_name_token, is_header_value_token, is_uri_token, Bytes};

const WIDTH: usize = core::mem::size_of::<usize>();

#[inline]
pub(crate) fn uri(bytes: &mut Bytes<'_>) {
    scan_words::<0x21>(bytes, is_uri_token);
}

#[inline]
pub(crate) fn header_value(bytes: &mut Bytes<'_>) {
    scan_words::<0x20>(bytes, is_header_value_token);
}

#[inline]
pub(crate) fn header_name(bytes: &mut Bytes<'_>) {
    while let Some(byte) = bytes.peek() {
        if !is_header_name_token(byte) {
            break;
        }
        bytes.bump();
    }
}

#[inline]
fn scan_words<const MIN: u8>(bytes: &mut Bytes<'_>, accepts: impl Fn(u8) -> bool) {
    while let Some(word) = bytes.peek_n::<[u8; WIDTH]>(WIDTH) {
        let count = match_word::<MIN>(word);
        bytes.advance(count);
        if count < WIDTH {
            // The word check omits horizontal tabs. They are accepted
            // in header values and handled here by the scalar predicate.
            match bytes.peek() {
                Some(byte) if accepts(byte) => bytes.bump(),
                _ => return,
            }
        }
    }

    while let Some(byte) = bytes.peek() {
        if !accepts(byte) {
            break;
        }
        bytes.bump();
    }
}

#[inline]
fn match_word<const MIN: u8>(word: [u8; WIDTH]) -> usize {
    const ONES: usize = usize::from_le_bytes([1; WIDTH]);
    const HIGH_BITS: usize = usize::from_le_bytes([0x80; WIDTH]);
    const DEL: usize = usize::from_le_bytes([0x7f; WIDTH]);
    let lower_bound = usize::from_le_bytes([MIN; WIDTH]);

    // The first input byte is the least significant. A borrow can mark a
    // later byte, but it cannot precede the first invalid one.
    let word = usize::from_le_bytes(word);
    let below_min = word.wrapping_sub(lower_bound) & !word;
    let flipped_del = word ^ DEL;
    let equal_del = flipped_del.wrapping_sub(ONES) & !flipped_del;
    let invalid = (below_min | equal_del) & HIGH_BITS;
    (invalid.trailing_zeros() / 8) as usize
}

#[cfg(test)]
mod tests {
    use super::{header_value, uri, Bytes, WIDTH};

    #[test]
    fn word_scans_match_the_scalar_predicates_at_every_position() {
        for token_byte in 0..=255u8 {
            for offset in 0..WIDTH * 3 {
                let mut input = [b'x'; WIDTH * 3];
                input[offset] = token_byte;
                // Include bytes that can be spuriously marked by a borrow
                // from an earlier lane. The first invalid position still
                // has to agree with the scalar scan.
                if offset + 1 < input.len() {
                    input[offset + 1] = if offset % 2 == 0 { 0x20 } else { 0x7e };
                }
                for (scan, accepts) in [
                    (
                        uri as fn(&mut Bytes<'_>),
                        crate::is_uri_token as fn(u8) -> bool,
                    ),
                    (header_value, crate::is_header_value_token),
                ] {
                    let mut bytes = Bytes::new(&input);
                    let expected = input
                        .iter()
                        .position(|&byte| !accepts(byte))
                        .unwrap_or(input.len());
                    scan(&mut bytes);
                    assert_eq!(bytes.pos(), expected, "byte {token_byte}, offset {offset}");
                }
            }
        }
    }
}
