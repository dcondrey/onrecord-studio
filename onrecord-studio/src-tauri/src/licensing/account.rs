//! The native account boundary: sign-in session, Secure Enclave device key, integrity check and
//! a small Keychain-backed string store. The macOS implementation binds the C ABI that
//! `native/AppAccount.swift` exports; everything else is testable through `TestAccount`.

use std::fmt;

use serde::Deserialize;

use crate::error::AppError;

pub const PROVIDERS: &[&str] = &["email", "google", "apple", "linkedin", "github"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSession {
    pub user_id: String,
    pub email: Option<String>,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignInResult {
    SignedIn(AccountSession),
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionResult {
    SignedIn(AccountSession),
    SignedOut,
    Failed(String),
}

/// A bearer credential. Never printed: `Debug` is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken(String);

impl AccessToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AccessToken(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSignature {
    pub public_key_b64url: String,
    pub signature_b64url: String,
    pub thumbprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityResult {
    Ok,
    Tampered(String),
    Unknown(String),
}

pub trait NativeAccount: Send + Sync {
    fn sign_in(&self, provider: &str) -> Result<SignInResult, AppError>;
    fn session(&self) -> Result<SessionResult, AppError>;
    fn access_token(&self) -> Result<AccessToken, AppError>;
    fn sign_out(&self) -> Result<(), AppError>;
    fn device_sign(&self, challenge_b64url: &str) -> Result<DeviceSignature, AppError>;
    fn integrity_check(&self) -> IntegrityResult;
    /// Keychain string store. An empty stored value reads back as absent.
    fn secure_get(&self, key: &str) -> Result<Option<String>, AppError>;
    fn secure_set(&self, key: &str, value: &str) -> Result<(), AppError>;
}

pub fn validate_provider(provider: &str) -> Result<&str, AppError> {
    PROVIDERS
        .iter()
        .find(|candidate| **candidate == provider)
        .copied()
        .ok_or_else(|| AppError::License(format!("Unsupported sign-in provider '{provider}'.")))
}

fn native_error(context: &str, message: Option<String>) -> AppError {
    AppError::License(format!(
        "{context}: {}",
        message.unwrap_or_else(|| "no detail".to_string())
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionEnvelope {
    outcome: String,
    message: Option<String>,
    user_id: Option<String>,
    email: Option<String>,
    provider: Option<String>,
}

impl SessionEnvelope {
    fn session(self) -> Result<AccountSession, String> {
        match self.user_id.filter(|id| !id.is_empty()) {
            Some(user_id) => Ok(AccountSession {
                user_id,
                email: self.email.filter(|value| !value.is_empty()),
                provider: self.provider.filter(|value| !value.is_empty()),
            }),
            None => Err("The account service reported a session without a user id.".to_string()),
        }
    }
}

pub(crate) fn parse_sign_in(json: &str) -> SignInResult {
    match serde_json::from_str::<SessionEnvelope>(json) {
        Err(error) => SignInResult::Failed(format!("Invalid sign-in result: {error}")),
        Ok(envelope) => match envelope.outcome.as_str() {
            "signed_in" => envelope
                .session()
                .map_or_else(SignInResult::Failed, SignInResult::SignedIn),
            "cancelled" => SignInResult::Cancelled,
            _ => SignInResult::Failed(
                envelope
                    .message
                    .unwrap_or_else(|| "Sign-in failed.".to_string()),
            ),
        },
    }
}

pub(crate) fn parse_session(json: &str) -> SessionResult {
    match serde_json::from_str::<SessionEnvelope>(json) {
        Err(error) => SessionResult::Failed(format!("Invalid session result: {error}")),
        Ok(envelope) => match envelope.outcome.as_str() {
            "signed_in" => envelope
                .session()
                .map_or_else(SessionResult::Failed, SessionResult::SignedIn),
            "signed_out" => SessionResult::SignedOut,
            _ => SessionResult::Failed(
                envelope
                    .message
                    .unwrap_or_else(|| "The session could not be read.".to_string()),
            ),
        },
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenEnvelope {
    outcome: String,
    message: Option<String>,
    access_token: Option<String>,
}

pub(crate) fn parse_access_token(json: &str) -> Result<AccessToken, AppError> {
    let envelope: TokenEnvelope = serde_json::from_str(json)
        .map_err(|error| AppError::License(format!("Invalid access-token result: {error}")))?;
    match (envelope.outcome.as_str(), envelope.access_token) {
        ("ok" | "signed_in", Some(token)) if !token.is_empty() => Ok(AccessToken::new(token)),
        _ => Err(native_error("No access token", envelope.message)),
    }
}

#[cfg(any(test, all(target_os = "macos", feature = "direct-license")))]
#[derive(Deserialize)]
struct OutcomeEnvelope {
    outcome: String,
    message: Option<String>,
}

#[cfg(any(test, all(target_os = "macos", feature = "direct-license")))]
pub(crate) fn parse_outcome(json: &str, context: &str) -> Result<(), AppError> {
    let envelope: OutcomeEnvelope = serde_json::from_str(json)
        .map_err(|error| AppError::License(format!("Invalid {context} result: {error}")))?;
    if matches!(envelope.outcome.as_str(), "ok" | "signed_out") {
        Ok(())
    } else {
        Err(native_error(context, envelope.message))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceEnvelope {
    outcome: String,
    message: Option<String>,
    device_public_key_b64_url: Option<String>,
    signature_b64_url: Option<String>,
    thumbprint: Option<String>,
}

pub(crate) fn parse_device_signature(json: &str) -> Result<DeviceSignature, AppError> {
    let envelope: DeviceEnvelope = serde_json::from_str(json)
        .map_err(|error| AppError::License(format!("Invalid device-signature result: {error}")))?;
    let present = |value: Option<String>| value.filter(|item| !item.is_empty());
    match (
        envelope.outcome.as_str(),
        present(envelope.device_public_key_b64_url),
        present(envelope.signature_b64_url),
        present(envelope.thumbprint),
    ) {
        ("ok", Some(public_key_b64url), Some(signature_b64url), Some(thumbprint)) => {
            Ok(DeviceSignature {
                public_key_b64url,
                signature_b64url,
                thumbprint,
            })
        }
        _ => Err(native_error("Device signing failed", envelope.message)),
    }
}

#[derive(Deserialize)]
struct IntegrityEnvelope {
    outcome: String,
    detail: Option<String>,
}

pub(crate) fn parse_integrity(json: &str) -> IntegrityResult {
    match serde_json::from_str::<IntegrityEnvelope>(json) {
        Err(error) => IntegrityResult::Unknown(format!("Invalid integrity result: {error}")),
        Ok(envelope) => {
            let detail = envelope.detail.unwrap_or_default();
            match envelope.outcome.as_str() {
                "ok" => IntegrityResult::Ok,
                "tampered" => IntegrityResult::Tampered(detail),
                _ => IntegrityResult::Unknown(detail),
            }
        }
    }
}

#[derive(Deserialize)]
struct StoreEnvelope {
    outcome: String,
    message: Option<String>,
    value: Option<String>,
}

pub(crate) fn parse_secure_get(json: &str) -> Result<Option<String>, AppError> {
    let envelope: StoreEnvelope = serde_json::from_str(json)
        .map_err(|error| AppError::License(format!("Invalid secure-store result: {error}")))?;
    match envelope.outcome.as_str() {
        "ok" => Ok(envelope.value.filter(|value| !value.is_empty())),
        "missing" => Ok(None),
        _ => Err(native_error("Secure store read failed", envelope.message)),
    }
}

#[cfg(all(target_os = "macos", feature = "direct-license", not(test)))]
pub use macos::MacAccount;

#[cfg(all(target_os = "macos", feature = "direct-license", not(test)))]
mod macos {
    use std::ffi::{c_char, CStr, CString};

    use super::*;

    // ABI owned by native/AppAccount.swift. Each function returns a malloc'd JSON string that
    // must be released with `onrecord_account_free_string`. The Keychain, through the
    // secure-store pair, is the only place `max_seen_time` and the cached entitlement may live.
    unsafe extern "C" {
        fn onrecord_account_sign_in(provider: *const c_char) -> *mut c_char;
        fn onrecord_account_session() -> *mut c_char;
        fn onrecord_account_access_token() -> *mut c_char;
        fn onrecord_account_sign_out() -> *mut c_char;
        fn onrecord_account_device_sign(challenge_b64url: *const c_char) -> *mut c_char;
        fn onrecord_account_integrity_check() -> *mut c_char;
        fn onrecord_account_secure_get(key: *const c_char) -> *mut c_char;
        fn onrecord_account_secure_set(key: *const c_char, value: *const c_char) -> *mut c_char;
        fn onrecord_account_free_string(value: *mut c_char);
    }

    fn take(pointer: *mut c_char) -> Result<String, AppError> {
        if pointer.is_null() {
            return Err(AppError::License(
                "The native account service returned no result.".to_string(),
            ));
        }
        // SAFETY: the pointer is a NUL-terminated string allocated by the Swift side, is read
        // once here, and is released exactly once through its matching free function.
        let text = unsafe { CStr::from_ptr(pointer) }
            .to_string_lossy()
            .into_owned();
        unsafe { onrecord_account_free_string(pointer) };
        Ok(text)
    }

    fn c_string(value: &str) -> Result<CString, AppError> {
        CString::new(value)
            .map_err(|_| AppError::License("Account argument contained a NUL byte.".to_string()))
    }

    pub struct MacAccount;

    impl NativeAccount for MacAccount {
        fn sign_in(&self, provider: &str) -> Result<SignInResult, AppError> {
            let provider = c_string(validate_provider(provider)?)?;
            // SAFETY: `provider` outlives the call and is NUL-terminated.
            Ok(parse_sign_in(&take(unsafe {
                onrecord_account_sign_in(provider.as_ptr())
            })?))
        }
        fn session(&self) -> Result<SessionResult, AppError> {
            Ok(parse_session(&take(unsafe { onrecord_account_session() })?))
        }
        fn access_token(&self) -> Result<AccessToken, AppError> {
            parse_access_token(&take(unsafe { onrecord_account_access_token() })?)
        }
        fn sign_out(&self) -> Result<(), AppError> {
            parse_outcome(&take(unsafe { onrecord_account_sign_out() })?, "Sign-out")
        }
        fn device_sign(&self, challenge_b64url: &str) -> Result<DeviceSignature, AppError> {
            let challenge = c_string(challenge_b64url)?;
            // SAFETY: `challenge` outlives the call and is NUL-terminated.
            parse_device_signature(&take(unsafe {
                onrecord_account_device_sign(challenge.as_ptr())
            })?)
        }
        fn integrity_check(&self) -> IntegrityResult {
            match take(unsafe { onrecord_account_integrity_check() }) {
                Ok(json) => parse_integrity(&json),
                Err(error) => IntegrityResult::Unknown(error.to_string()),
            }
        }
        fn secure_get(&self, key: &str) -> Result<Option<String>, AppError> {
            let key = c_string(key)?;
            // SAFETY: `key` outlives the call and is NUL-terminated.
            parse_secure_get(&take(unsafe { onrecord_account_secure_get(key.as_ptr()) })?)
        }
        fn secure_set(&self, key: &str, value: &str) -> Result<(), AppError> {
            let (key, value) = (c_string(key)?, c_string(value)?);
            // SAFETY: both strings outlive the call and are NUL-terminated.
            parse_outcome(
                &take(unsafe { onrecord_account_secure_set(key.as_ptr(), value.as_ptr()) })?,
                "Secure store write",
            )
        }
    }
}

/// Direct builds outside macOS have no account service: every operation fails closed.
#[cfg(all(feature = "direct-license", any(test, not(target_os = "macos"))))]
pub struct UnavailableAccount;

#[cfg(all(feature = "direct-license", any(test, not(target_os = "macos"))))]
impl NativeAccount for UnavailableAccount {
    fn sign_in(&self, _: &str) -> Result<SignInResult, AppError> {
        Err(unavailable())
    }
    fn session(&self) -> Result<SessionResult, AppError> {
        Err(unavailable())
    }
    fn access_token(&self) -> Result<AccessToken, AppError> {
        Err(unavailable())
    }
    fn sign_out(&self) -> Result<(), AppError> {
        Err(unavailable())
    }
    fn device_sign(&self, _: &str) -> Result<DeviceSignature, AppError> {
        Err(unavailable())
    }
    fn integrity_check(&self) -> IntegrityResult {
        IntegrityResult::Unknown("no native account service on this platform".to_string())
    }
    fn secure_get(&self, _: &str) -> Result<Option<String>, AppError> {
        Err(unavailable())
    }
    fn secure_set(&self, _: &str, _: &str) -> Result<(), AppError> {
        Err(unavailable())
    }
}

#[cfg(all(feature = "direct-license", any(test, not(target_os = "macos"))))]
fn unavailable() -> AppError {
    AppError::License("Accounts are supported only on macOS.".to_string())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    pub struct AccountState {
        pub session: SessionResult,
        pub sign_in: SignInResult,
        pub thumbprint: String,
        pub integrity: IntegrityResult,
        pub fail_device_sign: bool,
        pub fail_secure_set: bool,
        pub store: HashMap<String, String>,
        pub signed_out: u32,
    }

    /// A signed-in test account with a fixed device thumbprint.
    #[derive(Clone)]
    pub struct TestAccount(pub Arc<Mutex<AccountState>>);

    impl TestAccount {
        pub fn signed_in(thumbprint: &str) -> Self {
            let session = AccountSession {
                user_id: "user-1".to_string(),
                email: Some("reporter@example.org".to_string()),
                provider: Some("github".to_string()),
            };
            Self(Arc::new(Mutex::new(AccountState {
                session: SessionResult::SignedIn(session.clone()),
                sign_in: SignInResult::SignedIn(session),
                thumbprint: thumbprint.to_string(),
                integrity: IntegrityResult::Ok,
                fail_device_sign: false,
                fail_secure_set: false,
                store: HashMap::new(),
                signed_out: 0,
            })))
        }
        pub fn state(&self) -> std::sync::MutexGuard<'_, AccountState> {
            self.0.lock().unwrap()
        }
    }

    impl NativeAccount for TestAccount {
        fn sign_in(&self, provider: &str) -> Result<SignInResult, AppError> {
            validate_provider(provider)?;
            let mut state = self.state();
            if let SignInResult::SignedIn(session) = &state.sign_in {
                state.session = SessionResult::SignedIn(session.clone());
            }
            Ok(state.sign_in.clone())
        }
        fn session(&self) -> Result<SessionResult, AppError> {
            Ok(self.state().session.clone())
        }
        fn access_token(&self) -> Result<AccessToken, AppError> {
            match self.state().session {
                SessionResult::SignedIn(_) => Ok(AccessToken::new("supabase-access".to_string())),
                _ => Err(AppError::License("Not signed in.".to_string())),
            }
        }
        fn sign_out(&self) -> Result<(), AppError> {
            let mut state = self.state();
            state.session = SessionResult::SignedOut;
            state.store.clear();
            state.signed_out += 1;
            Ok(())
        }
        fn device_sign(&self, challenge: &str) -> Result<DeviceSignature, AppError> {
            let state = self.state();
            if state.fail_device_sign {
                return Err(AppError::License("Device key unavailable.".to_string()));
            }
            Ok(DeviceSignature {
                public_key_b64url: "device-public".to_string(),
                signature_b64url: format!("sig:{challenge}"),
                thumbprint: state.thumbprint.clone(),
            })
        }
        fn integrity_check(&self) -> IntegrityResult {
            self.state().integrity.clone()
        }
        fn secure_get(&self, key: &str) -> Result<Option<String>, AppError> {
            Ok(self
                .state()
                .store
                .get(key)
                .filter(|v| !v.is_empty())
                .cloned())
        }
        fn secure_set(&self, key: &str, value: &str) -> Result<(), AppError> {
            let mut state = self.state();
            if state.fail_secure_set {
                return Err(AppError::License("Keychain write refused.".to_string()));
            }
            state.store.insert(key.to_string(), value.to_string());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_json_fails_closed_on_every_unexpected_shape() {
        assert!(matches!(parse_sign_in("nope"), SignInResult::Failed(_)));
        assert!(matches!(
            parse_sign_in(r#"{"outcome":"signed_in","message":"x"}"#),
            SignInResult::Failed(_)
        ));
        assert_eq!(
            parse_sign_in(r#"{"outcome":"cancelled","message":""}"#),
            SignInResult::Cancelled
        );
        assert!(matches!(
            parse_sign_in(r#"{"outcome":"weird","message":"m"}"#),
            SignInResult::Failed(message) if message == "m"
        ));
        assert_eq!(
            parse_session(r#"{"outcome":"signed_out"}"#),
            SessionResult::SignedOut
        );
        assert!(matches!(
            parse_session(
                r#"{"outcome":"signed_in","message":"","userId":"u","email":"a@b.c","provider":"apple"}"#
            ),
            SessionResult::SignedIn(session) if session.user_id == "u"
        ));
        assert!(parse_access_token(r#"{"outcome":"ok","accessToken":""}"#).is_err());
        assert!(parse_access_token(r#"{"outcome":"failed","accessToken":"t"}"#).is_err());
        assert!(parse_device_signature(r#"{"outcome":"ok","thumbprint":"t"}"#).is_err());
        assert!(matches!(
            parse_integrity(r#"{"outcome":"???","detail":"d"}"#),
            IntegrityResult::Unknown(_)
        ));
        assert_eq!(parse_secure_get(r#"{"outcome":"missing"}"#).unwrap(), None);
        assert!(parse_secure_get(r#"{"outcome":"failed"}"#).is_err());
        assert!(parse_outcome(r#"{"outcome":"signed_out"}"#, "x").is_ok());
        assert!(parse_outcome(r#"{"outcome":"failed","message":"m"}"#, "x").is_err());
    }

    /// Exact success literals as `native/AppAccount.swift` emits them (sorted keys), one per
    /// call. If Swift changes a literal, the source scan below fails first.
    #[test]
    fn swift_success_literals_are_accepted() {
        assert!(matches!(
            parse_sign_in(
                r#"{"email":"a@b.c","message":"Signed in.","outcome":"signed_in","provider":"github","userId":"u"}"#
            ),
            SignInResult::SignedIn(_)
        ));
        assert_eq!(
            parse_sign_in(r#"{"message":"Sign-in was cancelled.","outcome":"cancelled"}"#),
            SignInResult::Cancelled
        );
        assert!(matches!(
            parse_session(r#"{"message":"Signed in.","outcome":"signed_in","userId":"u"}"#),
            SessionResult::SignedIn(_)
        ));
        assert_eq!(
            parse_session(r#"{"outcome":"signed_out"}"#),
            SessionResult::SignedOut
        );
        assert!(parse_access_token(
            r#"{"accessToken":"t","expiresAt":"2030-01-01T00:00:00Z","outcome":"signed_in"}"#
        )
        .is_ok());
        assert!(parse_outcome(r#"{"message":"Signed out.","outcome":"signed_out"}"#, "s").is_ok());
        let device = parse_device_signature(
            r#"{"devicePublicKeyB64Url":"k","keyStorage":"secureEnclave","outcome":"ok","signatureB64Url":"s","thumbprint":"t"}"#,
        )
        .unwrap();
        assert_eq!(device.thumbprint, "t");
        assert_eq!(
            parse_integrity(r#"{"detail":"d","outcome":"ok"}"#),
            IntegrityResult::Ok
        );
        assert!(matches!(
            parse_integrity(r#"{"detail":"d","outcome":"tampered"}"#),
            IntegrityResult::Tampered(_)
        ));
        assert!(matches!(
            parse_integrity(r#"{"detail":"d","outcome":"unknown"}"#),
            IntegrityResult::Unknown(_)
        ));
        assert_eq!(
            parse_secure_get(r#"{"outcome":"ok","value":"v"}"#).unwrap(),
            Some("v".to_string())
        );
        assert_eq!(parse_secure_get(r#"{"outcome":"missing"}"#).unwrap(), None);
        assert!(parse_outcome(r#"{"outcome":"ok"}"#, "set").is_ok());
    }

    /// Every outcome word the Swift bridge can emit must be one the parsers know, so a rename
    /// on either side cannot silently turn every call into a failure.
    #[test]
    fn every_swift_outcome_literal_is_known_to_the_parsers() {
        const KNOWN: &[&str] = &[
            "ok",
            "missing",
            "failed",
            "cancelled",
            "signed_in",
            "signed_out",
            "tampered",
            "unknown",
        ];
        let source = include_str!("../../native/AppAccount.swift");
        let mut found = 0;
        for line in source.lines() {
            for marker in ["\"outcome\": \"", "outcome(\""] {
                let mut rest = line;
                while let Some(at) = rest.find(marker) {
                    rest = &rest[at + marker.len()..];
                    let word = rest.split('"').next().unwrap();
                    assert!(
                        KNOWN.contains(&word),
                        "Swift emits unknown outcome '{word}'"
                    );
                    found += 1;
                }
            }
        }
        assert!(found >= 10, "the scan found only {found} literals");
    }

    #[test]
    fn access_token_debug_never_prints_the_secret() {
        let text = format!("{:?}", AccessToken::new("s3cret".to_string()));
        assert!(!text.contains("s3cret"));
    }

    #[test]
    fn unknown_providers_are_rejected() {
        assert!(validate_provider("github").is_ok());
        assert!(validate_provider("evil\0").is_err());
    }
}
