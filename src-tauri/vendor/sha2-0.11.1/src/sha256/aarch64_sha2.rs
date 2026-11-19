//! SHA-256 `aarch64` backend.
//!
//! Implementation adapted from mbedtls.
#![forbid(unsafe_code)]

// Modified by Kiwix to use checked vector loads and stores.
use core::arch::aarch64::*;

#[cfg(not(target_arch = "aarch64"))]
compile_error!("aarch64-sha2 backend can be used only on aarch64 target arches");

#[target_feature(enable = "sha2")]
pub(super) fn compress(state: &mut [u32; 8], blocks: &[[u8; 64]]) {
    // Load state into vectors.
    let [mut abcd, mut efgh] = load_state(state);

    // Iterate through the message blocks.
    for block in blocks {
        // Keep original state values.
        let abcd_orig = abcd;
        let efgh_orig = efgh;

        // Load the message block into vectors, assuming little endianness.
        let [mut s0, mut s1, mut s2, mut s3] = load_block(block);

        // Rounds 0 to 3
        let mut tmp = vaddq_u32(s0, rk(0));
        let mut abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        // Rounds 4 to 7
        tmp = vaddq_u32(s1, rk(4));
        abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        // Rounds 8 to 11
        tmp = vaddq_u32(s2, rk(8));
        abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        // Rounds 12 to 15
        tmp = vaddq_u32(s3, rk(12));
        abcd_prev = abcd;
        abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
        efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

        for t in (16..64).step_by(16) {
            // Rounds t to t + 3
            s0 = vsha256su1q_u32(vsha256su0q_u32(s0, s1), s2, s3);
            tmp = vaddq_u32(s0, rk(t));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

            // Rounds t + 4 to t + 7
            s1 = vsha256su1q_u32(vsha256su0q_u32(s1, s2), s3, s0);
            tmp = vaddq_u32(s1, rk(t + 4));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

            // Rounds t + 8 to t + 11
            s2 = vsha256su1q_u32(vsha256su0q_u32(s2, s3), s0, s1);
            tmp = vaddq_u32(s2, rk(t + 8));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);

            // Rounds t + 12 to t + 15
            s3 = vsha256su1q_u32(vsha256su0q_u32(s3, s0), s1, s2);
            tmp = vaddq_u32(s3, rk(t + 12));
            abcd_prev = abcd;
            abcd = vsha256hq_u32(abcd_prev, efgh, tmp);
            efgh = vsha256h2q_u32(efgh, abcd_prev, tmp);
        }

        // Add the block-specific state to the original state.
        abcd = vaddq_u32(abcd, abcd_orig);
        efgh = vaddq_u32(efgh, efgh_orig);
    }

    // Store vectors into state.
    store_state(state, [abcd, efgh]);
}

#[inline]
#[target_feature(enable = "neon")]
fn vector([a, b, c, d]: [u32; 4]) -> uint32x4_t {
    let result = vdupq_n_u32(a);
    let result = vsetq_lane_u32::<1>(b, result);
    let result = vsetq_lane_u32::<2>(c, result);
    vsetq_lane_u32::<3>(d, result)
}

#[inline]
#[target_feature(enable = "neon")]
fn rk(i: usize) -> uint32x4_t {
    let chunk = crate::consts::K32[i..][..4].try_into().unwrap();
    vector(chunk)
}

#[inline]
#[target_feature(enable = "neon")]
fn load_block(block: &[u8; 64]) -> [uint32x4_t; 4] {
    let (lanes, _) = block.as_chunks::<4>();
    core::array::from_fn(|i| {
        vector(core::array::from_fn(|j| u32::from_be_bytes(lanes[4 * i + j])))
    })
}

#[inline]
#[target_feature(enable = "neon")]
fn load_state(state: &[u32; 8]) -> [uint32x4_t; 2] {
    let (chunks, _) = state.as_chunks::<4>();
    [vector(chunks[0]), vector(chunks[1])]
}

#[inline]
#[target_feature(enable = "neon")]
fn store_state(state_dst: &mut [u32; 8], [abcd, efgh]: [uint32x4_t; 2]) {
    *state_dst = [
        vgetq_lane_u32::<0>(abcd),
        vgetq_lane_u32::<1>(abcd),
        vgetq_lane_u32::<2>(abcd),
        vgetq_lane_u32::<3>(abcd),
        vgetq_lane_u32::<0>(efgh),
        vgetq_lane_u32::<1>(efgh),
        vgetq_lane_u32::<2>(efgh),
        vgetq_lane_u32::<3>(efgh),
    ];
}
