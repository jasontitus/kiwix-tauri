// SPDX-License-Identifier: GPL-3.0-or-later
#![forbid(unsafe_code)]

use checked::Digest as CheckedDigest;
use reference::Digest as ReferenceDigest;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

fn file_digest(path: &Path) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut before = reference::Sha256::new();
    let mut after = checked::Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        before.update(&buffer[..count]);
        after.update(&buffer[..count]);
    }
    let digest: [u8; 32] = after.finalize().into();
    assert_eq!(
        <[u8; 32]>::from(before.finalize()),
        digest,
        "{}",
        path.display()
    );
    println!("{bytes} {} {}", hex::encode(digest), path.display());
    Ok(())
}

fn main() -> io::Result<()> {
    for filename in std::env::args_os().skip(1) {
        file_digest(Path::new(&filename))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(len: usize) -> Vec<u8> {
        let mut state = 0xf85cbf3109341a37u64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    fn compare(before: reference::Sha256, after: checked::Sha256) {
        assert_eq!(
            <[u8; 32]>::from(before.finalize()),
            <[u8; 32]>::from(after.finalize())
        );
    }

    #[test]
    fn padding_stream_partitions_and_forks_match() {
        let data = bytes(65536 + 64);
        for length in (0..144).chain([255, 256, 4095, 4096, 4097, 65535, 65536]) {
            for offset in 0..32 {
                let input = &data[offset..offset + length];
                let expected: [u8; 32] = reference::Sha256::digest(input).into();
                assert_eq!(<[u8; 32]>::from(checked::Sha256::digest(input)), expected);
                for width in [1, 55, 64, 127, 4096] {
                    let mut before = reference::Sha256::new();
                    let mut after = checked::Sha256::new();
                    for (index, chunk) in input.chunks(width).enumerate() {
                        before.update(chunk);
                        after.update(chunk);
                        after.update([]);
                        if index < 4 {
                            compare(before.clone(), after.clone());
                            let mut old_fork = before.clone();
                            let mut new_fork = after.clone();
                            old_fork.update(b"alternate suffix");
                            new_fork.update(b"alternate suffix");
                            compare(old_fork, new_fork);
                        }
                    }
                    assert_eq!(<[u8; 32]>::from(after.finalize()), expected);
                }
            }
        }
    }

    #[test]
    fn compresses_full_blocks_from_arbitrary_states() {
        use reference::digest::generic_array::GenericArray;
        let input = bytes(19 * 64 + 32);
        for offset in 0..16 {
            let (blocks, _) = input[offset..].as_chunks::<64>();
            let reference_blocks: Vec<_> = blocks
                .iter()
                .map(|block| GenericArray::clone_from_slice(block))
                .collect();
            let init = core::array::from_fn(|index| {
                let position = 4 * index + offset;
                u32::from_le_bytes(input[position..position + 4].try_into().unwrap())
            });
            for count in 0..=blocks.len() {
                let mut before = init;
                let mut after = init;
                reference::compress256(&mut before, &reference_blocks[..count]);
                checked::block_api::compress256(&mut after, &blocks[..count]);
                assert_eq!(after, before, "offset {offset}, blocks {count}");
            }
        }
    }

    #[test]
    #[ignore = "streams 4 GiB without allocating the whole input"]
    fn checks_digest_across_the_32_bit_byte_boundary() {
        let block: Vec<_> = (0..65536).map(|i| (i * 17 + (i >> 9)) as u8).collect();
        let mut state = checked::Sha256::new();
        for _ in 0..65535 {
            state.update(&block);
        }
        state.update(&block[..65535]);
        assert_eq!(
            hex::encode(state.clone().finalize()),
            "77c71012f90e900c88b08fe928d126afbfd24f54d7bf3b2616e45cb536ff91a6"
        );
        state.update(&block[65535..]);
        assert_eq!(
            hex::encode(state.clone().finalize()),
            "e38fca173d291aad85290cfeb19a9ccfc2d29e96ded293935950bf2ad1662b30"
        );
        state.update(&block[..79]);
        assert_eq!(
            hex::encode(state.finalize()),
            "942b8db518c5e85f34c20b84dc5ada52c674950a5a5868e379543c9b1e432be0"
        );
    }
}
