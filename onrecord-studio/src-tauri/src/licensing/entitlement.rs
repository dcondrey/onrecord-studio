//! Verification of the server-signed entitlement (compact JWS, EdDSA). See
//! `docs/accounts-and-licensing.md`. Every function here is pure: time, the challenge and the
//! device thumbprint are inputs, so each denial is testable without a network or native code.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::DateTime;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;

use crate::types::LicensePlan;

pub const ISSUER: &str = "auth.writerslogic.com/onrecord";
pub const AUDIENCE: &str = "com.onrecord.studio";
pub const MAX_LIFETIME_SECONDS: i64 = 7 * 24 * 3600;
pub const MAX_IAT_SKEW_SECONDS: i64 = 5 * 60;
const MAX_TOKEN_BYTES: usize = 8 * 1024;

/// Pinned entitlement public keys as `(kid, base64url of the 32-byte Ed25519 key)`. The release
/// process fills this constant; rotation adds a second entry. Empty means no token can verify,
/// so an unfilled build denies everything. Tests pass their own keys to `verify` as an argument;
/// there is deliberately no runtime or environment route to substitute a key.
pub const ENTITLEMENT_PUBLIC_KEYS: &[(&str, &str)] =
    &[("or-2026-1", "PhILOdFMgDbDJVKVr7pN6tVYChP7w8r-qS3JMilBs28")];

#[derive(Clone)]
pub struct PinnedKey {
    kid: String,
    key: VerifyingKey,
}

impl PinnedKey {
    pub fn new(kid: &str, key: VerifyingKey) -> Self {
        Self {
            kid: kid.to_string(),
            key,
        }
    }
}

/// Parses the compiled-in key constant. A malformed entry is skipped, which can only reduce
/// what verifies.
pub fn pinned_keys() -> Vec<PinnedKey> {
    ENTITLEMENT_PUBLIC_KEYS
        .iter()
        .filter_map(|(kid, encoded)| {
            let bytes: [u8; 32] = URL_SAFE_NO_PAD.decode(encoded).ok()?.try_into().ok()?;
            Some(PinnedKey::new(kid, VerifyingKey::from_bytes(&bytes).ok()?))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonceCheck<'a> {
    /// A fresh token must echo the challenge issued for this request.
    Challenge(&'a str),
    /// A token read back from storage: its challenge is gone by design. Replay protection
    /// applied when it was acquired; signature, device, and time are still enforced.
    Stored,
}

pub struct VerifyContext<'a> {
    pub now: i64,
    pub nonce: NonceCheck<'a>,
    pub device_thumbprint: &'a str,
}

/// A trial cannot exist without its end: the end lives inside the variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    Trial { ends_at: i64 },
    Licensed,
}

impl Grant {
    pub fn plan(self) -> LicensePlan {
        match self {
            Self::Trial { .. } => LicensePlan::Trial,
            Self::Licensed => LicensePlan::Licensed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEntitlement {
    pub subject: String,
    pub grant: Grant,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Denial {
    #[error("The entitlement is malformed: {0}")]
    Malformed(&'static str),
    #[error("The entitlement is signed with an unknown key.")]
    UnknownKey,
    #[error("The entitlement signature is invalid.")]
    BadSignature,
    #[error("The entitlement has the wrong {0}.")]
    WrongClaim(&'static str),
    #[error("The entitlement was issued in the future; the system clock is behind the server.")]
    IssuedInFuture,
    #[error("The entitlement lifetime exceeds the permitted maximum.")]
    LifetimeTooLong,
    #[error("The entitlement has expired.")]
    Expired { expires_at: i64 },
    #[error("The trial has ended.")]
    TrialExpired { ends_at: i64 },
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    kid: String,
}

#[derive(Deserialize)]
struct Claims {
    iss: Option<String>,
    aud: Option<String>,
    sub: Option<String>,
    plan: Option<String>,
    trial_ends_at: Option<String>,
    trial_started_at: Option<String>,
    dev: Option<String>,
    nonce: Option<String>,
    iat: Option<i64>,
    exp: Option<i64>,
}

fn decode_segment(segment: &str, what: &'static str) -> Result<Vec<u8>, Denial> {
    URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|_| Denial::Malformed(what))
}

fn rfc3339_seconds(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp())
}

pub fn verify(
    token: &str,
    keys: &[PinnedKey],
    context: &VerifyContext<'_>,
) -> Result<VerifiedEntitlement, Denial> {
    if token.len() > MAX_TOKEN_BYTES {
        return Err(Denial::Malformed("token too large"));
    }
    let mut parts = token.split('.');
    let (Some(header_b64), Some(payload_b64), Some(signature_b64), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Denial::Malformed("not a compact JWS"));
    };

    let header: Header = serde_json::from_slice(&decode_segment(header_b64, "header encoding")?)
        .map_err(|_| Denial::Malformed("header"))?;
    if header.alg != "EdDSA" {
        return Err(Denial::Malformed("algorithm"));
    }
    let pinned = keys
        .iter()
        .find(|candidate| candidate.kid == header.kid)
        .ok_or(Denial::UnknownKey)?;
    let signature = Signature::from_slice(&decode_segment(signature_b64, "signature encoding")?)
        .map_err(|_| Denial::BadSignature)?;
    let signed_len = header_b64.len() + 1 + payload_b64.len();
    pinned
        .key
        .verify_strict(&token.as_bytes()[..signed_len], &signature)
        .map_err(|_| Denial::BadSignature)?;

    let claims: Claims = serde_json::from_slice(&decode_segment(payload_b64, "payload encoding")?)
        .map_err(|_| Denial::Malformed("payload"))?;

    if claims.iss.as_deref() != Some(ISSUER) {
        return Err(Denial::WrongClaim("issuer"));
    }
    if claims.aud.as_deref() != Some(AUDIENCE) {
        return Err(Denial::WrongClaim("audience"));
    }
    let subject = claims
        .sub
        .filter(|value| !value.is_empty())
        .ok_or(Denial::Malformed("missing subject"))?;
    if let NonceCheck::Challenge(expected) = context.nonce {
        if expected.is_empty() || claims.nonce.as_deref() != Some(expected) {
            return Err(Denial::WrongClaim("nonce"));
        }
    }
    if context.device_thumbprint.is_empty()
        || claims.dev.as_deref() != Some(context.device_thumbprint)
    {
        return Err(Denial::WrongClaim("device"));
    }
    let issued_at = claims.iat.ok_or(Denial::Malformed("missing iat"))?;
    let expires_at = claims.exp.ok_or(Denial::Malformed("missing exp"))?;
    if issued_at > context.now.saturating_add(MAX_IAT_SKEW_SECONDS) {
        return Err(Denial::IssuedInFuture);
    }
    if expires_at <= issued_at || expires_at - issued_at > MAX_LIFETIME_SECONDS {
        return Err(Denial::LifetimeTooLong);
    }
    if context.now >= expires_at {
        return Err(Denial::Expired { expires_at });
    }

    let grant = match claims.plan.as_deref() {
        Some("licensed") => Grant::Licensed,
        Some("trial") => {
            let ends_at = claims
                .trial_ends_at
                .as_deref()
                .and_then(rfc3339_seconds)
                .ok_or(Denial::Malformed("trial without a valid end"))?;
            if let Some(started) = claims.trial_started_at.as_deref() {
                if rfc3339_seconds(started).is_none_or(|started| started > ends_at) {
                    return Err(Denial::Malformed("trial start"));
                }
            }
            if context.now >= ends_at {
                return Err(Denial::TrialExpired { ends_at });
            }
            Grant::Trial { ends_at }
        }
        _ => return Err(Denial::WrongClaim("plan")),
    };

    Ok(VerifiedEntitlement {
        subject,
        grant,
        issued_at,
        expires_at,
    })
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::{json, Value};

    pub const NOW: i64 = 1_800_000_000;

    pub struct TestSigner {
        pub signing: SigningKey,
        pub kid: &'static str,
    }

    impl TestSigner {
        pub fn generate(kid: &'static str) -> Self {
            let mut seed = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut seed);
            Self {
                signing: SigningKey::from_bytes(&seed),
                kid,
            }
        }

        pub fn pinned(&self) -> PinnedKey {
            PinnedKey::new(self.kid, self.signing.verifying_key())
        }

        pub fn sign(&self, claims: &Value) -> String {
            self.sign_with_header(&json!({"alg": "EdDSA", "kid": self.kid}), claims)
        }

        pub fn sign_with_header(&self, header: &Value, claims: &Value) -> String {
            let signed = format!(
                "{}.{}",
                URL_SAFE_NO_PAD.encode(header.to_string()),
                URL_SAFE_NO_PAD.encode(claims.to_string())
            );
            let signature = self.signing.sign(signed.as_bytes());
            format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
        }
    }

    pub fn licensed_claims(nonce: &str, thumbprint: &str) -> Value {
        json!({
            "iss": ISSUER, "aud": AUDIENCE, "sub": "user-1", "plan": "licensed",
            "dev": thumbprint, "nonce": nonce,
            "iat": NOW - 60, "exp": NOW - 60 + 3 * 24 * 3600,
        })
    }

    pub fn trial_claims(nonce: &str, thumbprint: &str, ends_at: i64) -> Value {
        let mut claims = licensed_claims(nonce, thumbprint);
        claims["plan"] = json!("trial");
        claims["trial_started_at"] = json!(rfc3339(ends_at - 14 * 24 * 3600));
        claims["trial_ends_at"] = json!(rfc3339(ends_at));
        claims
    }

    pub fn rfc3339(seconds: i64) -> String {
        DateTime::from_timestamp(seconds, 0).unwrap().to_rfc3339()
    }
}

#[cfg(test)]
mod pinned_key_tests {
    #[test]
    fn every_pinned_entitlement_key_parses() {
        // A malformed pin is skipped silently, which would deny every user, so count them.
        assert_eq!(
            super::pinned_keys().len(),
            super::ENTITLEMENT_PUBLIC_KEYS.len()
        );
        assert!(!super::ENTITLEMENT_PUBLIC_KEYS.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use serde_json::json;

    const NONCE: &str = "challenge-1";
    const THUMB: &str = "thumb-1";

    fn ctx(now: i64) -> VerifyContext<'static> {
        VerifyContext {
            now,
            nonce: NonceCheck::Challenge(NONCE),
            device_thumbprint: THUMB,
        }
    }

    fn denial(signer: &TestSigner, claims: serde_json::Value) -> Denial {
        verify(&signer.sign(&claims), &[signer.pinned()], &ctx(NOW)).unwrap_err()
    }

    #[test]
    fn a_well_formed_licensed_token_verifies() {
        let signer = TestSigner::generate("k1");
        let token = signer.sign(&licensed_claims(NONCE, THUMB));
        let verified = verify(&token, &[signer.pinned()], &ctx(NOW)).unwrap();
        assert_eq!(verified.grant, Grant::Licensed);
    }

    #[test]
    fn claim_violations_are_denied() {
        let signer = TestSigner::generate("k1");
        let base = licensed_claims(NONCE, THUMB);
        let mut cases = Vec::new();
        for (field, value, expected) in [
            ("iss", json!("evil"), Denial::WrongClaim("issuer")),
            ("aud", json!("com.other"), Denial::WrongClaim("audience")),
            ("nonce", json!("other"), Denial::WrongClaim("nonce")),
            ("dev", json!("other-mac"), Denial::WrongClaim("device")),
            ("plan", json!("enterprise"), Denial::WrongClaim("plan")),
            ("iat", json!(NOW + 301), Denial::IssuedInFuture),
        ] {
            let mut claims = base.clone();
            claims[field] = value;
            cases.push((field, claims, expected));
        }
        for (field, claims, expected) in cases {
            assert_eq!(denial(&signer, claims), expected, "{field}");
        }
    }

    #[test]
    fn iat_skew_boundary_is_five_minutes() {
        let signer = TestSigner::generate("k1");
        for (iat, ok) in [(NOW + 299, true), (NOW + 300, true), (NOW + 301, false)] {
            let mut claims = licensed_claims(NONCE, THUMB);
            claims["iat"] = json!(iat);
            claims["exp"] = json!(iat + 3600);
            let result = verify(&signer.sign(&claims), &[signer.pinned()], &ctx(NOW));
            assert_eq!(result.is_ok(), ok, "iat offset {}", iat - NOW);
        }
    }

    #[test]
    fn missing_subject_is_denied() {
        let signer = TestSigner::generate("k1");
        let mut claims = licensed_claims(NONCE, THUMB);
        claims.as_object_mut().unwrap().remove("sub");
        assert!(matches!(denial(&signer, claims), Denial::Malformed(_)));
    }

    #[test]
    fn expiry_and_lifetime_bounds() {
        let signer = TestSigner::generate("k1");
        let seven_days = MAX_LIFETIME_SECONDS;
        for (iat, exp, expected) in [
            (NOW - 60, NOW, Some(Denial::Expired { expires_at: NOW })),
            (NOW - 60, NOW + 1, None),
            (NOW, NOW + seven_days, None),
            (NOW, NOW + seven_days + 1, Some(Denial::LifetimeTooLong)),
        ] {
            let mut claims = licensed_claims(NONCE, THUMB);
            claims["iat"] = json!(iat);
            claims["exp"] = json!(exp);
            let result = verify(&signer.sign(&claims), &[signer.pinned()], &ctx(NOW));
            assert_eq!(result.err(), expected, "iat {iat} exp {exp}");
        }
    }

    #[test]
    fn trial_requires_an_unpassed_end() {
        let signer = TestSigner::generate("k1");
        let mut no_end = trial_claims(NONCE, THUMB, NOW + 100);
        no_end.as_object_mut().unwrap().remove("trial_ends_at");
        assert!(matches!(denial(&signer, no_end), Denial::Malformed(_)));

        let mut garbage_end = trial_claims(NONCE, THUMB, NOW + 100);
        garbage_end["trial_ends_at"] = json!("tomorrow");
        assert!(matches!(denial(&signer, garbage_end), Denial::Malformed(_)));

        assert_eq!(
            denial(&signer, trial_claims(NONCE, THUMB, NOW)),
            Denial::TrialExpired { ends_at: NOW }
        );
        let token = signer.sign(&trial_claims(NONCE, THUMB, NOW + 1));
        let verified = verify(&token, &[signer.pinned()], &ctx(NOW)).unwrap();
        assert_eq!(verified.grant, Grant::Trial { ends_at: NOW + 1 });
    }

    #[test]
    fn signature_and_key_failures_are_denied() {
        let signer = TestSigner::generate("k1");
        let other = TestSigner::generate("k1");
        let token = signer.sign(&licensed_claims(NONCE, THUMB));
        // Same kid, different key.
        assert_eq!(
            verify(&token, &[other.pinned()], &ctx(NOW)).unwrap_err(),
            Denial::BadSignature
        );
        // Unknown kid.
        let renamed = TestSigner {
            signing: signer.signing.clone(),
            kid: "k2",
        };
        assert_eq!(
            verify(
                &renamed.sign(&licensed_claims(NONCE, THUMB)),
                &[signer.pinned()],
                &ctx(NOW)
            )
            .unwrap_err(),
            Denial::UnknownKey
        );
        // Payload tampered after signing.
        let mut parts: Vec<String> = token.split('.').map(str::to_string).collect();
        let mut claims = licensed_claims(NONCE, THUMB);
        claims["sub"] = json!("someone-else");
        parts[1] = URL_SAFE_NO_PAD.encode(claims.to_string());
        assert_eq!(
            verify(&parts.join("."), &[signer.pinned()], &ctx(NOW)).unwrap_err(),
            Denial::BadSignature
        );
    }

    #[test]
    fn only_eddsa_and_compact_form_are_accepted() {
        let signer = TestSigner::generate("k1");
        let claims = licensed_claims(NONCE, THUMB);
        for header in [
            json!({"alg": "none", "kid": "k1"}),
            json!({"alg": "HS256", "kid": "k1"}),
        ] {
            let token = signer.sign_with_header(&header, &claims);
            assert!(matches!(
                verify(&token, &[signer.pinned()], &ctx(NOW)),
                Err(Denial::Malformed(_))
            ));
        }
        let token = signer.sign(&claims);
        for bad in [format!("{token}.extra"), "a.b".to_string(), String::new()] {
            assert!(verify(&bad, &[signer.pinned()], &ctx(NOW)).is_err());
        }
    }

    #[test]
    fn stored_tokens_skip_only_the_nonce() {
        let signer = TestSigner::generate("k1");
        let token = signer.sign(&licensed_claims("old-challenge", THUMB));
        let stored = VerifyContext {
            now: NOW,
            nonce: NonceCheck::Stored,
            device_thumbprint: THUMB,
        };
        assert!(verify(&token, &[signer.pinned()], &stored).is_ok());
        let wrong_device = VerifyContext {
            device_thumbprint: "other",
            ..stored
        };
        assert!(verify(&token, &[signer.pinned()], &wrong_device).is_err());
    }
}
