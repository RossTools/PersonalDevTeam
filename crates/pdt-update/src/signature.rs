//! The release signature (PDT-531): `SHA256SUMS.sig` is an ECDSA P-256 / SHA-256 signature, DER-encoded as
//! `openssl dgst -sha256 -sign` writes it, over [`signed_message`]: the release's tag, a newline, then SHA256SUMS
//! byte for byte. The tag inside it stops an old release's signed SHA256SUMS from vouching for another release. The
//! key is pinned as base64 of its SubjectPublicKeyInfo DER (`pdt_core::brand::RELEASE_SIGNING_KEY`); the install
//! scripts check the same message with openssl and .NET.

use base64::Engine;
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;

/// What the release workflow signs: `<tag>\n` followed by SHA256SUMS.
pub fn signed_message(tag: &str, sums: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(tag.len() + 1 + sums.len());
    m.extend_from_slice(tag.as_bytes());
    m.push(b'\n');
    m.extend_from_slice(sums);
    m
}

/// Checks `sig` (DER) over the signed message of `tag` and `sums` with `key` (base64 SPKI DER). The error says why.
pub fn verify_sums(key: &str, tag: &str, sums: &[u8], sig: &[u8]) -> Result<(), String> {
    let der = base64::engine::general_purpose::STANDARD
        .decode(key.trim())
        .map_err(|e| format!("the pinned signing key is not base64: {e}"))?;
    let key =
        VerifyingKey::from_public_key_der(&der).map_err(|e| format!("the pinned signing key is unusable: {e}"))?;
    let sig = Signature::from_der(sig).map_err(|_| "the signature is malformed".to_string())?;
    key.verify(&signed_message(tag, sums), &sig).map_err(|_| "the signature does not match".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::SigningKey;
    use p256::ecdsa::signature::Signer;
    use p256::pkcs8::EncodePublicKey;

    fn pair() -> (SigningKey, String) {
        let sk = SigningKey::from_slice(&[7u8; 32]).unwrap();
        let der = sk.verifying_key().to_public_key_der().unwrap();
        (sk, base64::engine::general_purpose::STANDARD.encode(der.as_bytes()))
    }

    fn sign(sk: &SigningKey, tag: &str, sums: &[u8]) -> Vec<u8> {
        let sig: Signature = sk.sign(&signed_message(tag, sums));
        sig.to_der().as_bytes().to_vec()
    }

    #[test]
    fn a_good_signature_verifies() {
        let (sk, key) = pair();
        let sig = sign(&sk, "v1.2.3", b"abc  x\n");
        assert_eq!(verify_sums(&key, "v1.2.3", b"abc  x\n", &sig), Ok(()));
    }

    /// Made by scripts/release-sign.sh (openssl) with a throwaway key: what the release workflow publishes verifies.
    #[test]
    fn an_openssl_signature_verifies() {
        let key = "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEZv3zoT15o6bXkEaJR39Kgk3t5Gawa5svOK89OHGRwZTgRkkH3/Y9igSbiKXQn6aeXX6m6nSzQ2Ifrvt3n/zJYg==";
        let sig = hex::decode(
            "3046022100c6b05e5948e87080e70255fe5f4d4eb4de68528119586cd435ff15febf755827022100b3b3170fa6016bd29c5e89960c4b\
             e6373a775dd6ad1093159a1287142083726e",
        )
        .unwrap();
        assert_eq!(verify_sums(key, "v1.2.3", b"abc  a.tar.gz\n", &sig), Ok(()));
        assert!(verify_sums(key, "v1.2.4", b"abc  a.tar.gz\n", &sig).is_err());
    }

    #[test]
    fn changed_sums_another_tag_or_another_key_fail() {
        let (sk, key) = pair();
        let sig = sign(&sk, "v1.2.3", b"abc  x\n");
        assert!(verify_sums(&key, "v1.2.3", b"abd  x\n", &sig).is_err(), "changed SHA256SUMS");
        assert!(verify_sums(&key, "v9.9.9", b"abc  x\n", &sig).is_err(), "replayed on another release");
        let other = SigningKey::from_slice(&[9u8; 32]).unwrap();
        assert!(verify_sums(&key, "v1.2.3", b"abc  x\n", &sign(&other, "v1.2.3", b"abc  x\n")).is_err());
    }

    #[test]
    fn junk_keys_and_signatures_are_errors() {
        let (sk, key) = pair();
        let sig = sign(&sk, "t", b"s");
        assert!(verify_sums("not base64!", "t", b"s", &sig).unwrap_err().contains("not base64"));
        assert!(verify_sums("AAAA", "t", b"s", &sig).unwrap_err().contains("unusable"));
        assert!(verify_sums(&key, "t", b"s", b"junk").unwrap_err().contains("malformed"));
    }
}
