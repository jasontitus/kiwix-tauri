// SPDX-License-Identifier: GPL-3.0-or-later

use checked::digest::block_buffer::{
    array::typenum::{U1, U128, U16, U254, U255, U3, U32, U63, U64},
    BlockBuffer, BlockSizes, BufferKind, Eager, Lazy,
};

fn check_batches<BS: BlockSizes, K: BufferKind>(lazy: bool) {
    let size = BS::USIZE;
    let data: Vec<u8> = (0..3 * size + 2).map(|i| (i * 17 + 113) as u8).collect();
    for pos in 0..=size {
        let Ok(original) = BlockBuffer::<BS, K>::try_new(&data[..pos]) else {
            continue;
        };
        for length in [0, 1, size - pos, size - pos + 1, size, 2 * size, 3 * size] {
            let input = &data[1..1 + length];
            let joined = [&data[..pos], input].concat();
            let available = joined.len().saturating_sub(usize::from(lazy));
            let end = available / size * size;
            let first = if pos != 0 && end != 0 { size } else { 0 };
            let mut expected = Vec::new();
            if first != 0 {
                expected.push(&joined[..first]);
            }
            if first != end {
                expected.push(&joined[first..end]);
            }

            let mut buffer = original.clone();
            let mut callbacks = 0;
            buffer.digest_block_bytes(input, |bytes| {
                assert_eq!(bytes, expected[callbacks]);
                if first == 0 || callbacks != 0 {
                    let start = if first == 0 { 0 } else { size - pos };
                    assert_eq!(bytes.as_ptr(), input[start..].as_ptr());
                }
                callbacks += 1;
            });
            assert_eq!(callbacks, expected.len());
            assert_eq!(buffer.get_data(), &joined[end..]);
        }
    }
}

#[test]
fn native_batches_match_a_separate_model_and_borrow_bulk_input() {
    macro_rules! check {
        ($($size:ty),+) => { $(
            check_batches::<$size, Eager>(false);
            check_batches::<$size, Lazy>(true);
        )+ };
    }
    check!(U1, U3, U16, U32, U63, U64, U128, U254, U255);
}

#[test]
fn native_batch_panic_keeps_the_buffer_reusable() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    for stop in [1, 2] {
        let mut buffer = BlockBuffer::<U3, Lazy>::new(b"abc");
        let mut calls = 0;
        let input = b"123456789";
        let failed = catch_unwind(AssertUnwindSafe(|| {
            buffer.digest_block_bytes(input, |_| {
                calls += 1;
                assert_ne!(calls, stop);
            });
        }));
        assert!(failed.is_err());
        assert_eq!(calls, stop);
        assert!(buffer.get_data().is_empty());

        let mut output = Vec::new();
        buffer.digest_block_bytes(input, |bytes| output.extend_from_slice(bytes));
        assert_eq!(output, b"123456");
        assert_eq!(buffer.get_data(), b"789");
    }
}

fn verify_sha2<D: checked::Digest, R: reference::Digest>(data: &[u8], width: usize) {
    let mut new = D::new();
    let mut old = R::new();
    for segment in data.chunks(width) {
        new.update(segment);
        old.update(segment);
    }
    assert_eq!(new.finalize()[..], old.finalize()[..]);
}

#[test]
fn all_sha2_digests_match_the_reference_across_partitions() {
    let data: Vec<u8> = (0..8193).map(|i| (i * 113 + 1) as u8).collect();
    for len in [
        0,
        1,
        54,
        55,
        56,
        63,
        64,
        65,
        111,
        112,
        127,
        128,
        129,
        1024,
        data.len(),
    ] {
        for width in [1, 3, 32, 63, 64, 65, 128, 129, 4096] {
            let bytes = &data[..len];
            verify_sha2::<checked::Sha224, reference::Sha224>(bytes, width);
            verify_sha2::<checked::Sha256, reference::Sha256>(bytes, width);
            verify_sha2::<checked::Sha384, reference::Sha384>(bytes, width);
            verify_sha2::<checked::Sha512, reference::Sha512>(bytes, width);
            verify_sha2::<checked::Sha512_224, reference::Sha512_224>(bytes, width);
            verify_sha2::<checked::Sha512_256, reference::Sha512_256>(bytes, width);
        }
    }
}

#[test]
fn core_proxy_and_object_update_keep_the_buffer_and_reset() {
    use checked::{
        digest::{block_api::CoreProxy, DynDigest},
        Digest, Sha256,
    };
    let mut hasher = Sha256::new();
    Digest::update(&mut hasher, [1; 81]);
    let (core, buffer) = hasher.decompose();
    assert_eq!(buffer.get_data(), &[1; 17]);
    let mut combined = Sha256::compose(core, buffer);
    Digest::update(&mut combined, [2; 196]);

    let expected =
        <reference::Sha256 as reference::Digest>::digest([[1; 81].as_slice(), &[2; 196]].concat());
    assert_eq!(Digest::finalize_reset(&mut combined)[..], expected[..]);
    assert_eq!(combined.finalize(), Sha256::digest(b""));

    let mut boxed: Box<dyn DynDigest> = Box::new(Sha256::new());
    boxed.update(&[1; 81]);
    boxed.update(&[2; 196]);
    assert_eq!(boxed.finalize_reset().as_ref(), &expected[..]);
    assert_eq!(boxed.finalize().as_ref(), &Sha256::digest(b"")[..]);
}

// Straight scalar rounds from the SHA-1 definition, independent of the backend.
fn sha1_reference_compress(state: &mut [u32; 5], blocks: &[[u8; 64]]) {
    for block in blocks {
        let mut words = [0u32; 80];
        for (word, bytes) in words.iter_mut().zip(block.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().unwrap());
        }
        for i in 16..80 {
            words[i] = (words[i - 3] ^ words[i - 8] ^ words[i - 14] ^ words[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = *state;
        for (round, word) in words.into_iter().enumerate() {
            let (f, k) = match round {
                0..=19 => ((b & c) | (!b & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            [a, b, c, d, e] = [next, a, b.rotate_left(30), c, d];
        }
        for (old, new) in state.iter_mut().zip([a, b, c, d, e]) {
            *old = old.wrapping_add(new);
        }
    }
}

#[test]
fn sha1_compression_matches_scalar_rounds_from_arbitrary_states() {
    let data: Vec<u8> = (0..127 * 64 + 15)
        .map(|i| (179 * i + i / 257) as u8)
        .collect();
    for offset in [0, 1, 15] {
        for count in [0, 1, 2, 127] {
            let (blocks, tail) = data[offset..offset + 64 * count].as_chunks::<64>();
            assert!(tail.is_empty());
            let mut actual = [0x713f8901, 0x5b7eafb9, 0x123fed43, 0xdeadbeef, 0xaceb9444];
            let mut expected = actual;
            sha1::block_api::compress(&mut actual, blocks);
            sha1_reference_compress(&mut expected, blocks);
            assert_eq!(actual, expected, "offset={offset}, blocks={count}");
        }
    }
}

#[test]
fn sha1_matches_a_separate_oracle_across_chunk_boundaries() {
    use sha1::{Digest, Sha1};
    let sample: Vec<u8> = (0..8193).map(|i| (i * 113 + 1) as u8).collect();
    for (len, expected) in [
        (0, "da39a3ee5e6b4b0d3255bfef95601890afd80709"),
        (1, "bf8b4530d8d246dd74ac53a13471bba17941dff7"),
        (3, "2607d89005cb104fa8f9fc5f246da0f7814331cc"),
        (55, "d29ec9d580c7277f520c69b2c02ca3fca680aa49"),
        (56, "9af50db8560e0c797d0eb98b86f619d0099bd4f3"),
        (63, "c4f4d4c9929d6132eff6140b1cbec26e95dc2f0b"),
        (64, "715ff5e6561231b13972a16eef4a213fcef7fecc"),
        (65, "c8ec8c5282947e9bde41d5bdb58952816b6aadf3"),
        (119, "dc103a2c4e881577cfa5cb811458c31c58e1a9c1"),
        (120, "fca7052165e8c9a7c9a5570069e89fe310261136"),
        (127, "2de933b81d6eda6106894f43543159b5e203b58a"),
        (128, "03c43fa43fb141ce30b24062f39cbee0c24c8b75"),
        (129, "b0430dec077eb3eebb5e31ccc46d012bad17f1db"),
        (4095, "f6615b08908ec189061cb5d19ba91442371a210b"),
        (4096, "7e5f33606ed68eefa76c9dfda69dce11b5b96d60"),
        (4097, "83ce72003e1f8eca1dced4d3fe7ff35eb094f341"),
        (8193, "06f1724005a8ec8258672a3ceb69bc71d6f12e50"),
    ] {
        for width in [1, 3, 55, 63, 64, 65, 128, 129, 4096] {
            let mut digest = Sha1::new();
            for chunk in sample[..len].chunks(width) {
                digest.update(chunk);
            }
            assert_eq!(hex::encode(digest.finalize_reset()), expected);
            assert_eq!(
                hex::encode(digest.finalize()),
                "da39a3ee5e6b4b0d3255bfef95601890afd80709"
            );
        }
    }
}
