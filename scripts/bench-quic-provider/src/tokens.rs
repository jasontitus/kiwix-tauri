#![forbid(unsafe_code)]

use std::sync::Arc;

use aes_gcm::{
    Aes256Gcm,
    aead::{AeadInOut, KeyInit, Nonce, Tag},
};
use hmac::{Hmac, Mac};
use quinn::crypto::{AeadKey, CryptoError, HandshakeTokenKey, HmacKey};
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

type Mac256 = Hmac<Sha256>;
const TAG_LEN: usize = 16;

pub fn endpoint() -> anyhow::Result<quinn::EndpointConfig> {
    let mut seed = Zeroizing::new([0u8; 64]);
    getrandom::fill(&mut *seed).map_err(|error| anyhow::anyhow!("reset entropy: {error}"))?;
    let key = <Mac256 as KeyInit>::new_from_slice(&*seed).expect("HMAC accepts any key length");
    Ok(quinn::EndpointConfig::new(Arc::new(ResetKey(key))))
}

pub fn server(crypto: Arc<dyn quinn::crypto::ServerConfig>) -> anyhow::Result<quinn::ServerConfig> {
    let mut seed = Zeroizing::new([0u8; 64]);
    getrandom::fill(&mut *seed).map_err(|error| anyhow::anyhow!("token entropy: {error}"))?;
    Ok(quinn::ServerConfig::new(
        crypto,
        Arc::new(TokenKey(hkdf::Hkdf::new(None, &*seed))),
    ))
}

struct ResetKey(Mac256);

impl HmacKey for ResetKey {
    fn sign(&self, data: &[u8], out: &mut [u8]) {
        let mut mac = self.0.clone();
        mac.update(data);
        out.copy_from_slice(&mac.finalize().into_bytes());
    }

    fn signature_len(&self) -> usize {
        32
    }

    fn verify(&self, data: &[u8], signature: &[u8]) -> Result<(), CryptoError> {
        let mut mac = self.0.clone();
        mac.update(data);
        mac.verify_slice(signature).map_err(|_| CryptoError)
    }
}

struct TokenKey(hkdf::Hkdf<Sha256>);

impl HandshakeTokenKey for TokenKey {
    fn aead_from_hkdf(&self, random_bytes: &[u8]) -> Box<dyn AeadKey> {
        let mut key = [0u8; 32];
        self.0
            .expand(random_bytes, &mut key)
            .expect("32 bytes fit HKDF-SHA-256");
        // Quinn supplies a fresh random input for each token. Its derived key
        // is used for that token only, just like Quinn's ring default.
        let cipher = Aes256Gcm::new(&key.into());
        key.zeroize();
        Box::new(TokenAead(cipher))
    }
}

struct TokenAead(Aes256Gcm);

impl AeadKey for TokenAead {
    fn seal(&self, data: &mut Vec<u8>, aad: &[u8]) -> Result<(), CryptoError> {
        let nonce = Nonce::<Aes256Gcm>::from([0; 12]);
        let tag = self
            .0
            .encrypt_inout_detached(&nonce, aad, data.as_mut_slice().into())
            .map_err(|_| CryptoError)?;
        data.extend_from_slice(&tag);
        Ok(())
    }

    fn open<'a>(&self, data: &'a mut [u8], aad: &[u8]) -> Result<&'a mut [u8], CryptoError> {
        let end = data.len().checked_sub(TAG_LEN).ok_or(CryptoError)?;
        let (body, tag) = data.split_at_mut(end);
        let tag = Tag::<Aes256Gcm>::try_from(&*tag).map_err(|_| CryptoError)?;
        let nonce = Nonce::<Aes256Gcm>::from([0; 12]);
        self.0
            .decrypt_inout_detached(&nonce, aad, body.into(), &tag)
            .map_err(|_| CryptoError)?;
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231_and_requires_the_complete_tag() {
        let key = ResetKey(<Mac256 as KeyInit>::new_from_slice(&[0xb; 20]).unwrap());
        let data = b"Hi There";
        let expected = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];
        let mut actual = [0; 32];
        key.sign(data, &mut actual);
        assert_eq!(actual, expected);
        assert!(key.verify(data, &actual).is_ok());
        assert!(key.verify(data, &actual[..31]).is_err());
        assert!(key.verify(b"Different message", &actual).is_err());
    }

    #[test]
    fn token_checks_aad_key_and_complete_tag() {
        let key = TokenKey(hkdf::Hkdf::new(None, &[4; 64]));
        let signer = key.aead_from_hkdf(&[9; 32]);
        let verifier = key.aead_from_hkdf(&[9; 32]);
        let wrong = key.aead_from_hkdf(&[8; 32]);
        let mut sealed = b"test token".to_vec();
        signer.seal(&mut sealed, b"route A").unwrap();
        assert!(verifier.open(&mut sealed.clone(), b"route B").is_err());
        assert!(wrong.open(&mut sealed.clone(), b"route A").is_err());
        assert!(verifier.open(&mut [0; TAG_LEN - 1], b"route A").is_err());
        assert_eq!(
            verifier.open(&mut sealed, b"route A").unwrap(),
            b"test token"
        );
    }
}
