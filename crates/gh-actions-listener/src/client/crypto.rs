//! The two pieces of cryptography the protocol needs.

use aes::cipher::{BlockDecryptMut, KeyIvInit};
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Oaep, RsaPrivateKey};

use base64::Engine as _;

use crate::client::types::PublicKey;
use crate::error::Error;

/// AES-256-CBC, which is what `Aes.Create()` on the other side defaults to.
type Decryptor = cbc::Decryptor<aes::Aes256>;

/// The digest depends on a flag the session carries, so both have to be supported.
pub fn unwrap_session_key(
    key: &RsaPrivateKey,
    wrapped: &[u8],
    fips: bool,
) -> Result<Vec<u8>, Error> {
    let padding = if fips {
        Oaep::new::<sha2::Sha256>()
    } else {
        Oaep::new::<sha1::Sha1>()
    };

    key.decrypt(padding, wrapped)
        .map_err(|err| Error::Crypto(format!("cannot unwrap the session key: {err}")))
}

pub fn decrypt_message(key: &[u8], iv: &[u8], body: &[u8]) -> Result<String, Error> {
    let decryptor = Decryptor::new_from_slices(key, iv)
        .map_err(|err| Error::Crypto(format!("bad key or iv: {err}")))?;
    let plain = decryptor
        .decrypt_padded_vec_mut::<aes::cipher::block_padding::Pkcs7>(body)
        .map_err(|err| Error::Crypto(format!("cannot decrypt the message: {err}")))?;

    String::from_utf8(plain).map_err(|err| Error::Crypto(format!("message is not utf8: {err}")))
}

pub fn public_key(pem: &str) -> Result<PublicKey, Error> {
    use rsa::traits::PublicKeyParts as _;

    let key = read_key(pem)?;
    let encode = |number: &rsa::BigUint| {
        base64::engine::general_purpose::STANDARD.encode(number.to_bytes_be())
    };

    Ok(PublicKey {
        exponent: encode(key.e()),
        modulus: encode(key.n()),
    })
}

/// Either spelling: registering here writes PKCS#1, and a key from elsewhere may be PKCS#8.
pub fn read_key(pem: &str) -> Result<RsaPrivateKey, Error> {
    RsaPrivateKey::from_pkcs1_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs8_pem(pem))
        .map_err(|err| Error::Crypto(format!("cannot read the runner key: {err}")))
}

#[cfg(test)]
mod tests {
    use aes::cipher::{BlockEncryptMut, block_padding::Pkcs7};

    use super::*;

    type Encryptor = cbc::Encryptor<aes::Aes256>;

    #[test]
    fn a_message_survives_the_round_trip() {
        let key = [7u8; 32];
        let iv = [3u8; 16];
        let body = Encryptor::new_from_slices(&key, &iv)
            .expect("cipher")
            .encrypt_padded_vec_mut::<Pkcs7>(b"{\"messageType\":\"PipelineAgentJobRequest\"}");

        let plain = decrypt_message(&key, &iv, &body).expect("decrypts");
        assert!(plain.contains("PipelineAgentJobRequest"));
    }

    #[test]
    fn a_session_key_unwraps_with_the_runner_key() {
        use rsa::Oaep;

        let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("a key");
        let session = [9u8; 32];
        let wrapped = key
            .to_public_key()
            .encrypt(&mut rand::thread_rng(), Oaep::new::<sha1::Sha1>(), &session)
            .expect("wraps");

        assert_eq!(
            unwrap_session_key(&key, &wrapped, false).expect("unwraps"),
            session
        );
    }

    #[test]
    fn a_key_is_readable_in_either_spelling() {
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::pkcs8::EncodePrivateKey;

        let key = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).expect("a key");
        let pkcs1 = key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).expect("pkcs1");
        let pkcs8 = key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).expect("pkcs8");

        assert_eq!(read_key(&pkcs1).expect("reads pkcs1"), key);
        assert_eq!(read_key(&pkcs8).expect("reads pkcs8"), key);
    }
}
