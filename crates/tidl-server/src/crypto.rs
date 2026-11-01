use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("encryption failed: {0}")]
    #[allow(dead_code)]
    Encrypt(String),
    #[error("decryption failed: invalid data or wrong key")]
    Decrypt,
    #[error("invalid key length: expected 32 bytes")]
    InvalidKey,
}

/// Encrypt `plaintext` with AES-256-GCM using the given 32-byte key.
/// Returns `base64(12-byte nonce || ciphertext)` suitable for DB storage.
#[allow(dead_code)]
pub fn encrypt_pat(plaintext: &str, key: &[u8]) -> Result<String, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKey);
    }
    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(STANDARD.encode(&combined))
}

/// Decrypt the output of `encrypt_pat`. Key must be the same 32-byte key.
pub fn decrypt_pat(encoded: &str, key: &[u8]) -> Result<String, CryptoError> {
    if key.len() != 32 {
        return Err(CryptoError::InvalidKey);
    }
    let combined = STANDARD.decode(encoded).map_err(|_| CryptoError::Decrypt)?;
    if combined.len() < 12 {
        return Err(CryptoError::Decrypt);
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let key = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::Decrypt)?;
    String::from_utf8(plaintext).map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [42u8; 32];
        let pat = "ghp_test_personal_access_token";
        let enc = encrypt_pat(pat, &key).unwrap();
        let dec = decrypt_pat(&enc, &key).unwrap();
        assert_eq!(dec, pat);
    }

    #[test]
    fn wrong_key_fails() {
        let key = [1u8; 32];
        let enc = encrypt_pat("secret", &key).unwrap();
        let bad_key = [2u8; 32];
        assert!(decrypt_pat(&enc, &bad_key).is_err());
    }

    #[test]
    fn invalid_key_length() {
        assert!(encrypt_pat("x", &[1u8; 16]).is_err());
        assert!(decrypt_pat("abc", &[1u8; 16]).is_err());
    }
}
