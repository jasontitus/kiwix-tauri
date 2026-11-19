// Modified by Kiwix. The librqbit 9.1 `sha1-ring` feature selects RustCrypto.
#![forbid(unsafe_code)]

pub trait ISha1 {
    fn new() -> Self;
    fn update(&mut self, buf: &[u8]);
    fn finish(self) -> [u8; 20];
}

/// SHA-256 hash trait for BEP 52 (BitTorrent v2) support.
pub trait ISha256 {
    fn new() -> Self;
    fn update(&mut self, buf: &[u8]);
    fn finish(self) -> [u8; 32];

    fn finish_id32(self) -> [u8; 32]
    where
        Self: Sized,
    {
        self.finish()
    }
}

assert_cfg::exactly_one! {
    feature = "sha1-crypto-hash",
    feature = "sha1-ring",
}

#[cfg(feature = "sha1-crypto-hash")]
mod crypto_hash_impl {
    use super::{ISha1, ISha256};

    pub struct Sha1CryptoHash {
        inner: crypto_hash::Hasher,
    }

    impl ISha1 for Sha1CryptoHash {
        fn new() -> Self {
            Self {
                inner: crypto_hash::Hasher::new(crypto_hash::Algorithm::SHA1),
            }
        }

        fn update(&mut self, buf: &[u8]) {
            use std::io::Write;
            self.inner.write_all(buf).unwrap();
        }

        fn finish(mut self) -> [u8; 20] {
            let result = self.inner.finish();
            debug_assert_eq!(result.len(), 20);
            let mut result_arr = [0u8; 20];
            result_arr.copy_from_slice(&result);
            result_arr
        }
    }

    pub struct Sha256CryptoHash {
        inner: crypto_hash::Hasher,
    }

    impl ISha256 for Sha256CryptoHash {
        fn new() -> Self {
            Self {
                inner: crypto_hash::Hasher::new(crypto_hash::Algorithm::SHA256),
            }
        }

        fn update(&mut self, buf: &[u8]) {
            use std::io::Write;
            self.inner.write_all(buf).unwrap();
        }

        fn finish(mut self) -> [u8; 32] {
            let result = self.inner.finish();
            debug_assert_eq!(result.len(), 32);
            let mut result_arr = [0u8; 32];
            result_arr.copy_from_slice(&result);
            result_arr
        }
    }
}

#[cfg(feature = "sha1-ring")]
mod rustcrypto_impl {
    use super::{ISha1, ISha256};
    use sha2::Digest;

    pub struct Sha1RustCrypto {
        inner: sha1::Sha1,
    }

    impl ISha1 for Sha1RustCrypto {
        fn new() -> Self {
            Self {
                inner: sha1::Sha1::new(),
            }
        }

        fn update(&mut self, buf: &[u8]) {
            self.inner.update(buf);
        }

        fn finish(self) -> [u8; 20] {
            self.inner.finalize().into()
        }
    }

    pub struct Sha256RustCrypto {
        inner: sha2::Sha256,
    }

    impl ISha256 for Sha256RustCrypto {
        fn new() -> Self {
            Self {
                inner: sha2::Sha256::new(),
            }
        }

        fn update(&mut self, buf: &[u8]) {
            self.inner.update(buf);
        }

        fn finish(self) -> [u8; 32] {
            self.inner.finalize().into()
        }
    }
}

#[cfg(feature = "sha1-crypto-hash")]
pub type Sha1 = crypto_hash_impl::Sha1CryptoHash;

#[cfg(feature = "sha1-ring")]
pub type Sha1 = rustcrypto_impl::Sha1RustCrypto;

#[cfg(feature = "sha1-crypto-hash")]
pub type Sha256 = crypto_hash_impl::Sha256CryptoHash;

#[cfg(feature = "sha1-ring")]
pub type Sha256 = rustcrypto_impl::Sha256RustCrypto;

#[cfg(test)]
mod tests {
    use super::{ISha1, ISha256, Sha1, Sha256};

    #[test]
    fn sha1_matches_known_vector_across_updates() {
        let mut state = Sha1::new();
        state.update(b"a");
        state.update(b"");
        state.update(b"bc");
        assert_eq!(
            state.finish(),
            [
                0xa9, 0x99, 0x3e, 0x36, 0x47, 0x06, 0x81, 0x6a, 0xba, 0x3e, 0x25, 0x71, 0x78, 0x50,
                0xc2, 0x6c, 0x9c, 0xd0, 0xd8, 0x9d,
            ]
        );
    }

    fn assert_sha256_impl<T: ISha256>() {}

    #[test]
    fn test_sha256_known_vector_empty() {
        assert_sha256_impl::<Sha256>();
        let mut h = Sha256::new();
        h.update(b"");
        let got = h.finish();
        let expected: [u8; 32] = [
            0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
            0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
            0x78, 0x52, 0xb8, 0x55,
        ];
        assert_eq!(got, expected);
    }
}
