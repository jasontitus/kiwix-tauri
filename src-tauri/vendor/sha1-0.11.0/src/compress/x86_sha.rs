//! SHA-1 `x86`/`x86_64` backend
// Modified by Kiwix to load and store checked native words.

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
compile_error!("x86-sha backend can be used only on x86 and x86_64 target arches");

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

macro_rules! rounds4 {
    ($h0:ident, $h1:ident, $wk:expr, $i:expr) => {
        _mm_sha1rnds4_epu32($h0, _mm_sha1nexte_epu32($h1, $wk), $i)
    };
}

macro_rules! schedule {
    ($v0:expr, $v1:expr, $v2:expr, $v3:expr) => {
        _mm_sha1msg2_epu32(_mm_xor_si128(_mm_sha1msg1_epu32($v0, $v1), $v2), $v3)
    };
}

macro_rules! schedule_rounds4 {
    (
        $h0:ident, $h1:ident,
        $w0:expr, $w1:expr, $w2:expr, $w3:expr, $w4:expr,
        $i:expr
    ) => {
        $w4 = schedule!($w0, $w1, $w2, $w3);
        $h1 = rounds4!($h0, $h1, $w4, $i);
    };
}

#[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
#[allow(unused_unsafe)]
pub(crate) unsafe fn compress<'a>(state: &mut [u32; 5], blocks: impl Iterator<Item = &'a [u8; 64]>) {
    // SAFETY: the dispatch checks the target features. These intrinsics
    // operate on register values with constant lane indices.
    unsafe {
        let [a, b, c, d, e] = state.map(|word| word as i32);
        let mut state_abcd = _mm_set_epi32(a, b, c, d);
        let mut state_e = _mm_set_epi32(e, 0, 0, 0);

        for block in blocks {
            let [mut w0, mut w1, mut w2, mut w3] = [0, 16, 32, 48].map(|offset| {
                let [a, b, c, d] = core::array::from_fn(|lane| {
                    let word = offset + 4 * lane;
                    i32::from_be_bytes(block[word..word + 4].try_into().unwrap())
                });
                // SHA-1 expects the first message word in the highest lane.
                _mm_set_epi32(a, b, c, d)
            });
            #[allow(clippy::needless_late_init)]
            let mut w4;

            let mut h0 = state_abcd;
            let mut h1 = _mm_add_epi32(state_e, w0);

            // Rounds 0..20
            h1 = _mm_sha1rnds4_epu32(h0, h1, 0);
            h0 = rounds4!(h1, h0, w1, 0);
            h1 = rounds4!(h0, h1, w2, 0);
            h0 = rounds4!(h1, h0, w3, 0);
            schedule_rounds4!(h0, h1, w0, w1, w2, w3, w4, 0);

            // Rounds 20..40
            schedule_rounds4!(h1, h0, w1, w2, w3, w4, w0, 1);
            schedule_rounds4!(h0, h1, w2, w3, w4, w0, w1, 1);
            schedule_rounds4!(h1, h0, w3, w4, w0, w1, w2, 1);
            schedule_rounds4!(h0, h1, w4, w0, w1, w2, w3, 1);
            schedule_rounds4!(h1, h0, w0, w1, w2, w3, w4, 1);

            // Rounds 40..60
            schedule_rounds4!(h0, h1, w1, w2, w3, w4, w0, 2);
            schedule_rounds4!(h1, h0, w2, w3, w4, w0, w1, 2);
            schedule_rounds4!(h0, h1, w3, w4, w0, w1, w2, 2);
            schedule_rounds4!(h1, h0, w4, w0, w1, w2, w3, 2);
            schedule_rounds4!(h0, h1, w0, w1, w2, w3, w4, 2);

            // Rounds 60..80
            schedule_rounds4!(h1, h0, w1, w2, w3, w4, w0, 3);
            schedule_rounds4!(h0, h1, w2, w3, w4, w0, w1, 3);
            schedule_rounds4!(h1, h0, w3, w4, w0, w1, w2, 3);
            schedule_rounds4!(h0, h1, w4, w0, w1, w2, w3, 3);
            schedule_rounds4!(h1, h0, w0, w1, w2, w3, w4, 3);

            state_abcd = _mm_add_epi32(state_abcd, h0);
            state_e = _mm_sha1nexte_epu32(h1, state_e);
        }

        *state = [
            _mm_extract_epi32::<3>(state_abcd) as u32,
            _mm_extract_epi32::<2>(state_abcd) as u32,
            _mm_extract_epi32::<1>(state_abcd) as u32,
            _mm_extract_epi32::<0>(state_abcd) as u32,
            _mm_extract_epi32::<3>(state_e) as u32,
        ];
    }
}
