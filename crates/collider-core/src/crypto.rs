//! Clear-key AES-128-CBC segment decryption (RFC 8216 §4.3.2.4).

use aes::Aes128;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};

use crate::error::{Error, Result};

type Decryptor = cbc::Decryptor<Aes128>;

pub fn decrypt_aes128(data: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>> {
    Decryptor::new(key.into(), iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|e| Error::Decrypt(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cbc::cipher::BlockEncryptMut;

    #[test]
    fn roundtrip() {
        let key = [7u8; 16];
        let iv = crate::hls::iv_from_sequence(42);
        let plain = b"segment payload that is not block aligned".to_vec();
        let enc = cbc::Encryptor::<Aes128>::new(&key.into(), &iv.into())
            .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        assert_eq!(decrypt_aes128(&enc, &key, &iv).unwrap(), plain);
        assert!(decrypt_aes128(&enc, &[0u8; 16], &iv).is_err());
    }
}
