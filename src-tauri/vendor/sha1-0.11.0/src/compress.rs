// Modified by Kiwix to dispatch once for borrowed complete blocks.
use crate::block_api::Sha1Core;
use digest::block_api::Block;

cfg_if::cfg_if! {
    if #[cfg(sha1_backend = "soft")] {
        mod soft;
        pub(crate) use soft::compress;
    } else if #[cfg(sha1_backend = "aarch64-sha2")] {
        mod aarch64_sha2;

        #[cfg(not(target_feature = "sha2"))]
        compile_error!("aarch64-sha2 backend requires sha2 target feature");

        pub(crate) fn compress<'a>(state: &mut [u32; 5], blocks: impl Iterator<Item = &'a [u8; 64]>) {
            // SAFETY: we checked above that the required target features are enabled
            unsafe { aarch64_sha2::compress(state, blocks) }
        }
    } else if #[cfg(sha1_backend = "x86-sha")] {
        mod x86_sha;

        #[cfg(not(all(
            target_feature = "sha",
            target_feature = "sse2",
            target_feature = "ssse3",
            target_feature = "sse4.1",
        )))]
        compile_error!("x86-sha backend requires sha, sse2, ssse3, sse4.1 target features");

        pub(crate) fn compress<'a>(state: &mut [u32; 5], blocks: impl Iterator<Item = &'a [u8; 64]>) {
            // SAFETY: we checked above that the required target features are enabled
            unsafe { x86_sha::compress(state, blocks) }
        }
    } else if #[cfg(target_arch = "loongarch64")] {
        mod loongarch64_asm;
    } else {
        mod soft;

        cfg_if::cfg_if! {
            if #[cfg(any(target_arch = "x86", target_arch = "x86_64"))] {
                mod x86_sha;
                cpufeatures::new!(shani_cpuid, "sha", "sse2", "ssse3", "sse4.1");
            } else if #[cfg(target_arch = "aarch64")] {
                mod aarch64_sha2;
                cpufeatures::new!(sha2_hwcap, "sha2");
            }
        }

        pub(crate) fn compress<'a>(state: &mut [u32; 5], blocks: impl Iterator<Item = &'a [u8; 64]>) {
            cfg_if::cfg_if! {
                if #[cfg(any(target_arch = "x86", target_arch = "x86_64"))] {
                    if shani_cpuid::get() {
                        // SAFETY: we checked that required target features are available
                        return unsafe { x86_sha::compress(state, blocks) };
                    }
                } else if #[cfg(target_arch = "aarch64")] {
                    if sha2_hwcap::get() {
                        // SAFETY: we checked that `sha2` target feature is available
                        return unsafe { aarch64_sha2::compress(state, blocks) };
                    }
                }
            }

            soft::compress(state, blocks);
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(all(target_arch = "loongarch64", not(any(
        sha1_backend = "soft",
        sha1_backend = "aarch64-sha2",
        sha1_backend = "x86-sha",
    ))))] {
        pub(crate) use loongarch64_asm::{compress_bytes, compress_wrapped};
    } else {
        #[inline]
        pub(crate) fn compress_bytes(state: &mut [u32; 5], bytes: &[u8]) {
            let blocks = bytes.chunks_exact(64);
            assert!(blocks.remainder().is_empty());
            compress(state, blocks.map(|block| block.try_into().unwrap()));
        }

        #[inline]
        pub(crate) fn compress_wrapped(state: &mut [u32; 5], blocks: &[Block<Sha1Core>]) {
            compress(state, blocks.iter().map(|block| &block.0));
        }
    }
}
