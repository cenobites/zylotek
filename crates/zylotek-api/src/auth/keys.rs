use std::path::Path;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{
    SigningKey,
    pkcs8::{EncodePrivateKey, KeypairBytes, spki::der::pem::LineEnding},
};
use getrandom::{SysRng, rand_core::UnwrapErr};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header,
    jwk::{Jwk, JwkSet, PublicKeyUse, ThumbprintHash},
};
use serde::Serialize;

use crate::auth::{clients::write_private, error::AuthError};

/// RFC 9068 marks a JWT access token with this header `typ`, so other JWTs cannot pass as one.
pub(crate) const ACCESS_TOKEN_TYPE: &str = "at+jwt";

/// A new Ed25519 signing key as PKCS#8 PEM, in the plain 48-byte form that
/// `openssl genpkey -algorithm ed25519` writes (without the optional public-key half).
pub fn generate_signing_key_pem() -> Result<String, AuthError> {
    let mut rng = UnwrapErr(SysRng);
    let key = SigningKey::generate(&mut rng);
    let plain = KeypairBytes {
        secret_key: key.to_bytes(),
        public_key: None,
    };

    plain
        .to_pkcs8_pem(LineEnding::LF)
        .map(|pem| pem.to_string())
        .map_err(|error| AuthError::SigningKey(error.to_string()))
}

/// Generates a signing key and writes it to `path`, readable by the owner only. An existing
/// file is never replaced: losing a key would invalidate every token and client setup.
pub fn write_new_signing_key(path: &Path) -> Result<(), AuthError> {
    write_private(path, generate_signing_key_pem()?.as_bytes())
}

/// `byte_count` random bytes, base64url without padding.
pub(crate) fn random_string(byte_count: usize) -> Result<String, AuthError> {
    let mut bytes = vec![0u8; byte_count];
    getrandom::fill(&mut bytes).map_err(|_| AuthError::Random)?;

    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The key that signs access tokens, and the public JWKS that verifies them.
pub(crate) struct SigningKeys {
    encoding: EncodingKey,
    jwks: JwkSet,
    key_id: String,
}

impl SigningKeys {
    pub fn from_pem(pem: &str) -> Result<Self, AuthError> {
        let invalid = |_: jsonwebtoken::errors::Error| {
            AuthError::SigningKey(
                "expected an unencrypted Ed25519 private key in PKCS#8 PEM form, as written by \
                 `openssl genpkey -algorithm ed25519`"
                    .to_owned(),
            )
        };

        let encoding = EncodingKey::from_ed_pem(pem.as_bytes()).map_err(invalid)?;
        let mut jwk = Jwk::from_encoding_key(&encoding, Algorithm::EdDSA).map_err(invalid)?;
        let key_id = jwk.thumbprint(ThumbprintHash::SHA256).map_err(invalid)?;
        jwk.common.key_id = Some(key_id.clone());
        jwk.common.public_key_use = Some(PublicKeyUse::Signature);

        Ok(Self {
            encoding,
            jwks: JwkSet { keys: vec![jwk] },
            key_id,
        })
    }

    /// The public keys, as served at `/.well-known/jwks.json`.
    pub fn jwks(&self) -> &JwkSet {
        &self.jwks
    }

    pub fn sign<C: Serialize>(&self, claims: &C) -> Result<String, AuthError> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.typ = Some(ACCESS_TOKEN_TYPE.to_owned());
        header.kid = Some(self.key_id.clone());

        jsonwebtoken::encode(&header, claims, &self.encoding)
            .map_err(|error| AuthError::SigningKey(error.to_string()))
    }

    /// The verifying key a token's `kid` names, taken from the published JWKS.
    pub fn verifying_key(&self, key_id: &str) -> Option<DecodingKey> {
        let jwk = self.jwks.find(key_id)?;

        DecodingKey::from_jwk(jwk).ok()
    }
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve};
    use serde::Deserialize;

    use super::*;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Claims {
        sub: String,
        exp: u64,
    }

    fn keys() -> SigningKeys {
        SigningKeys::from_pem(&generate_signing_key_pem().unwrap()).unwrap()
    }

    #[test]
    fn a_generated_key_is_a_pem_private_key_and_every_one_differs() {
        let first = generate_signing_key_pem().unwrap();
        let second = generate_signing_key_pem().unwrap();

        assert!(first.starts_with("-----BEGIN PRIVATE KEY-----"));
        assert_ne!(first, second);
        assert!(SigningKeys::from_pem(&first).is_ok());
    }

    #[test]
    fn the_published_key_set_has_one_ed25519_signature_key_named_by_its_thumbprint() {
        let keys = keys();

        assert_eq!(keys.jwks().keys.len(), 1);
        let jwk = &keys.jwks().keys[0];

        assert!(matches!(
            &jwk.algorithm,
            AlgorithmParameters::OctetKeyPair(parameters) if parameters.curve == EllipticCurve::Ed25519
        ));
        assert_eq!(jwk.common.public_key_use, Some(PublicKeyUse::Signature));
        assert_eq!(
            jwk.common.key_id.as_deref(),
            Some(jwk.thumbprint(ThumbprintHash::SHA256).unwrap().as_str())
        );
        let json = serde_json::to_string(keys.jwks()).unwrap();
        assert!(
            !json.contains("\"d\""),
            "the private part is never published: {json}"
        );
    }

    #[test]
    fn a_signed_token_verifies_with_the_published_key_and_only_with_it() {
        let keys = keys();
        let claims = Claims {
            sub: "client".into(),
            exp: u64::MAX / 2,
        };
        let token = keys.sign(&claims).unwrap();
        let header = jsonwebtoken::decode_header(&token).unwrap();
        let key_id = header.kid.clone().unwrap();
        let mut validation = jsonwebtoken::Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();

        assert_eq!(header.typ.as_deref(), Some(ACCESS_TOKEN_TYPE));
        let verified = jsonwebtoken::decode::<Claims>(
            &token,
            &keys.verifying_key(&key_id).unwrap(),
            &validation,
        )
        .unwrap();
        assert_eq!(verified.claims, claims);
        assert!(keys.verifying_key("no-such-key").is_none());

        let other = self::keys();
        let other_id = other.jwks().keys[0].common.key_id.clone().unwrap();
        assert!(
            jsonwebtoken::decode::<Claims>(
                &token,
                &other.verifying_key(&other_id).unwrap(),
                &validation
            )
            .is_err()
        );
    }

    #[test]
    fn unusable_keys_are_rejected_with_a_reason() {
        for pem in [
            "",
            "not a pem",
            "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
        ] {
            let error = SigningKeys::from_pem(pem).err().expect("rejected");

            assert!(matches!(error, AuthError::SigningKey(_)), "{pem:?}");
            assert!(error.to_string().starts_with("invalid signing key"));
        }
    }

    #[test]
    fn random_strings_are_url_safe_unpadded_and_unique() {
        let first = random_string(32).unwrap();
        let second = random_string(32).unwrap();

        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        assert_eq!(random_string(0).unwrap(), "");
    }

    #[test]
    fn a_new_key_file_is_private_loadable_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key.pem");

        write_new_signing_key(&path).unwrap();

        let pem = std::fs::read_to_string(&path).unwrap();
        assert!(SigningKeys::from_pem(&pem).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let error = write_new_signing_key(&path).unwrap_err();
        assert!(matches!(error, AuthError::AlreadyExists(_)));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            pem,
            "the key is untouched"
        );
    }
}
