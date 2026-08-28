use std::sync::RwLock;

#[cfg(any(test, all(target_os = "macos", feature = "app-store")))]
use serde::Deserialize;

use crate::error::AppError;
use crate::types::{LicenseState, LicenseStatus};

pub const STATUS_COMMAND: &str = "get_license_status";
pub const REFRESH_COMMAND: &str = "refresh_license";
#[cfg(any(test, all(target_os = "macos", feature = "app-store")))]
const EXPECTED_BUNDLE_IDENTIFIER: &str = "com.onrecord.studio";

pub struct LicenseGate {
    status: RwLock<LicenseStatus>,
}

impl LicenseGate {
    pub fn initialize() -> Self {
        Self {
            status: RwLock::new(platform_status(false)),
        }
    }

    pub fn status(&self) -> LicenseStatus {
        self.status
            .read()
            .expect("license status lock must not be poisoned")
            .clone()
    }

    pub async fn refresh(&self) -> LicenseStatus {
        let result = tokio::task::spawn_blocking(|| platform_status(true))
            .await
            .unwrap_or_else(|error| {
                unavailable_status(format!("The StoreKit verification worker failed: {error}"))
            });
        *self
            .status
            .write()
            .expect("license status lock must not be poisoned") = result.clone();
        result
    }

    pub fn require_authorized(&self) -> Result<(), AppError> {
        let status = self.status();
        if status.authorized {
            Ok(())
        } else {
            Err(AppError::License(status.message))
        }
    }
}

pub fn command_is_unlicensed(command: &str) -> bool {
    matches!(command, STATUS_COMMAND | REFRESH_COMMAND)
}

#[cfg(not(all(target_os = "macos", feature = "app-store")))]
fn platform_status(_refresh: bool) -> LicenseStatus {
    LicenseStatus {
        enforced: false,
        authorized: true,
        state: LicenseState::Development,
        message: "Paid-license enforcement is enabled only in Mac App Store builds.".to_string(),
        bundle_id: Some("com.onrecord.studio".to_string()),
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        original_app_version: None,
        app_transaction_id: None,
        app_id: None,
        environment: Some("development".to_string()),
        original_purchase_date: None,
        signed_date: None,
        revocation_date: None,
    }
}

#[cfg(all(target_os = "macos", feature = "app-store"))]
fn platform_status(refresh: bool) -> LicenseStatus {
    use std::ffi::{c_char, CStr};

    unsafe extern "C" {
        fn onrecord_storekit_app_transaction(refresh: bool) -> *mut c_char;
        fn onrecord_storekit_free_string(value: *mut c_char);
    }

    let pointer = unsafe { onrecord_storekit_app_transaction(refresh) };
    if pointer.is_null() {
        return unavailable_status("StoreKit returned no transaction result.".to_string());
    }
    let json = unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned();
    unsafe { onrecord_storekit_free_string(pointer) };
    serde_json::from_str::<NativeLicenseEnvelope>(&json)
        .map(NativeLicenseEnvelope::into_status)
        .unwrap_or_else(|error| {
            unavailable_status(format!(
                "StoreKit returned an invalid transaction result: {error}"
            ))
        })
}

#[cfg(any(test, all(target_os = "macos", feature = "app-store")))]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NativeLicenseEnvelope {
    outcome: String,
    message: String,
    bundle_id: Option<String>,
    app_version: Option<String>,
    original_app_version: Option<String>,
    app_transaction_id: Option<String>,
    app_id: Option<u64>,
    environment: Option<String>,
    original_purchase_date: Option<String>,
    signed_date: Option<String>,
    revocation_date: Option<String>,
}

#[cfg(any(test, all(target_os = "macos", feature = "app-store")))]
impl NativeLicenseEnvelope {
    fn into_status(self) -> LicenseStatus {
        let mut state = match self.outcome.as_str() {
            "verified" => LicenseState::Verified,
            "revoked" => LicenseState::Revoked,
            "unverified" => LicenseState::Unverified,
            "bundle_mismatch" => LicenseState::BundleMismatch,
            _ => LicenseState::Unavailable,
        };
        if state == LicenseState::Verified
            && self.bundle_id.as_deref() != Some(EXPECTED_BUNDLE_IDENTIFIER)
        {
            state = LicenseState::BundleMismatch;
        }
        LicenseStatus {
            enforced: true,
            authorized: state == LicenseState::Verified,
            state,
            message: self.message,
            bundle_id: self.bundle_id,
            app_version: self.app_version,
            original_app_version: self.original_app_version,
            app_transaction_id: self.app_transaction_id,
            app_id: self.app_id,
            environment: self.environment,
            original_purchase_date: self.original_purchase_date,
            signed_date: self.signed_date,
            revocation_date: self.revocation_date,
        }
    }
}

fn unavailable_status(message: String) -> LicenseStatus {
    LicenseStatus {
        enforced: cfg!(feature = "app-store"),
        authorized: false,
        state: LicenseState::Unavailable,
        message,
        bundle_id: None,
        app_version: None,
        original_app_version: None,
        app_transaction_id: None,
        app_id: None,
        environment: None,
        original_purchase_date: None,
        signed_date: None,
        revocation_date: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_license_recovery_commands_bypass_the_gate() {
        assert!(command_is_unlicensed(STATUS_COMMAND));
        assert!(command_is_unlicensed(REFRESH_COMMAND));
        assert!(!command_is_unlicensed("list_saved_portals"));
        assert!(!command_is_unlicensed("export_legal_brief"));
    }

    #[test]
    fn native_outcomes_fail_closed_except_for_a_verified_transaction() {
        for (outcome, expected_state, authorized) in [
            ("verified", LicenseState::Verified, true),
            ("revoked", LicenseState::Revoked, false),
            ("unverified", LicenseState::Unverified, false),
            ("bundle_mismatch", LicenseState::BundleMismatch, false),
            ("future_value", LicenseState::Unavailable, false),
        ] {
            let status = NativeLicenseEnvelope {
                outcome: outcome.to_string(),
                message: "test".to_string(),
                bundle_id: (outcome == "verified").then(|| EXPECTED_BUNDLE_IDENTIFIER.to_string()),
                app_version: None,
                original_app_version: None,
                app_transaction_id: None,
                app_id: None,
                environment: None,
                original_purchase_date: None,
                signed_date: None,
                revocation_date: None,
            }
            .into_status();
            assert_eq!(status.state, expected_state);
            assert_eq!(status.authorized, authorized);
            assert!(status.enforced);
        }
    }

    #[test]
    fn a_verified_label_for_another_bundle_is_still_denied() {
        let status = NativeLicenseEnvelope {
            outcome: "verified".to_string(),
            message: "test".to_string(),
            bundle_id: Some("com.example.copy".to_string()),
            app_version: None,
            original_app_version: None,
            app_transaction_id: None,
            app_id: None,
            environment: None,
            original_purchase_date: None,
            signed_date: None,
            revocation_date: None,
        }
        .into_status();
        assert_eq!(status.state, LicenseState::BundleMismatch);
        assert!(!status.authorized);
    }
}
