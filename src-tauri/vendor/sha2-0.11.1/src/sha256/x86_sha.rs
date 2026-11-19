//! SHA-256 `x86`/`x86_64` backend
#![forbid(unsafe_code)]

// Modified by Kiwix to use checked vector loads and stores.
#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
compile_error!("x86-sha backend can be used only on x86 and x86_64 target arches");

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

#[inline]
#[target_feature(enable = "sha")]
fn rounds4(r: usize, abef: &mut __m128i, cdgh: &mut __m128i, rest: __m128i) {
    use crate::consts::K32;
    let rk = _mm_set_epi32(
        K32[4 * r + 3] as i32,
        K32[4 * r + 2] as i32,
        K32[4 * r + 1] as i32,
        K32[4 * r] as i32,
    );
    let t1 = _mm_add_epi32(rest, rk);
    *cdgh = _mm_sha256rnds2_epu32(*cdgh, *abef, t1);
    let t2 = _mm_shuffle_epi32(t1, 0x0E);
    *abef = _mm_sha256rnds2_epu32(*abef, *cdgh, t2);
}

#[inline]
#[target_feature(enable = "sha,ssse3")]
fn schedule_rounds16(r: usize, abef: &mut __m128i, cdgh: &mut __m128i, w: &mut [__m128i; 4]) {
    for i in 0..4 {
        let w0 = w[i];
        let w1 = w[(i + 1) % 4];
        let w2 = w[(i + 2) % 4];
        let w3 = w[(i + 3) % 4];

        let t1 = _mm_sha256msg1_epu32(w0, w1);
        let t2 = _mm_alignr_epi8(w3, w2, 4);
        let t3 = _mm_add_epi32(t1, t2);

        w[i] = _mm_sha256msg2_epu32(t3, w3);

        rounds4(r + i, abef, cdgh, w[i]);
    }
}

#[inline]
#[target_feature(enable = "sse2")]
fn vector([a, b, c, d]: [u32; 4]) -> __m128i {
    _mm_setr_epi32(a as i32, b as i32, c as i32, d as i32)
}

#[inline]
#[target_feature(enable = "sse2")]
fn load_block(block: &[u8; 64]) -> [__m128i; 4] {
    let (lanes, _) = block.as_chunks::<4>();
    core::array::from_fn(|i| {
        vector(core::array::from_fn(|j| u32::from_be_bytes(lanes[4 * i + j])))
    })
}

#[inline]
#[target_feature(enable = "sse2")]
fn load_state(state: &[u32; 8]) -> [__m128i; 2] {
    let (chunks, _) = state.as_chunks::<4>();
    [vector(chunks[0]), vector(chunks[1])]
}

#[inline]
#[target_feature(enable = "sse4.1")]
fn store_state(state: &mut [u32; 8], [dcba, hgef]: [__m128i; 2]) {
    *state = [
        _mm_extract_epi32::<0>(dcba) as u32,
        _mm_extract_epi32::<1>(dcba) as u32,
        _mm_extract_epi32::<2>(dcba) as u32,
        _mm_extract_epi32::<3>(dcba) as u32,
        _mm_extract_epi32::<0>(hgef) as u32,
        _mm_extract_epi32::<1>(hgef) as u32,
        _mm_extract_epi32::<2>(hgef) as u32,
        _mm_extract_epi32::<3>(hgef) as u32,
    ];
}

#[target_feature(enable = "sha,sse4.1")]
pub(super) fn compress(state: &mut [u32; 8], blocks: &[[u8; 64]]) {
    let [dcba, hgfe] = load_state(state);

    let cdab = _mm_shuffle_epi32(dcba, 0xB1);
    let efgh = _mm_shuffle_epi32(hgfe, 0x1B);
    let mut abef = _mm_alignr_epi8(cdab, efgh, 8);
    let mut cdgh = _mm_blend_epi16(efgh, cdab, 0xF0);

    for block in blocks {
        let abef_save = abef;
        let cdgh_save = cdgh;

        let mut w = load_block(block);

        rounds4(0, &mut abef, &mut cdgh, w[0]);
        rounds4(1, &mut abef, &mut cdgh, w[1]);
        rounds4(2, &mut abef, &mut cdgh, w[2]);
        rounds4(3, &mut abef, &mut cdgh, w[3]);

        schedule_rounds16(4, &mut abef, &mut cdgh, &mut w);
        schedule_rounds16(8, &mut abef, &mut cdgh, &mut w);
        schedule_rounds16(12, &mut abef, &mut cdgh, &mut w);

        abef = _mm_add_epi32(abef, abef_save);
        cdgh = _mm_add_epi32(cdgh, cdgh_save);
    }

    let feba = _mm_shuffle_epi32(abef, 0x1B);
    let dchg = _mm_shuffle_epi32(cdgh, 0xB1);
    let dcba = _mm_blend_epi16(feba, dchg, 0xF0);
    let hgef = _mm_alignr_epi8(dchg, feba, 8);

    store_state(state, [dcba, hgef]);
}
