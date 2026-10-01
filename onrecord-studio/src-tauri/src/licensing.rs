use std::sync::{Arc, RwLock};

#[cfg(all(feature = "app-store", feature = "direct-license"))]
compile_error!(
    "The `app-store` and `direct-license` features are mutually exclusive: the Mac App Store \
     build is gated by StoreKit and must never require an account."
);

// The account gate is compiled for the direct build, and for tests so its logic is exercised
// without native code or a feature flag.
#[cfg(any(test, feature = "direct-license"))]
pub mod account;
#[cfg(any(test, feature = "direct-license"))]
pub mod client;
#[cfg(any(test, feature = "direct-license"))]
pub mod clock;
#[cfg(any(test, feature = "direct-license"))]
pub mod direct;
#[cfg(any(test, feature = "direct-license"))]
pub mod entitlement;

#[cfg(any(test, all(target_os = "macos", feature = "app-store")))]
use serde::Deserialize;

use crate::error::AppError;
#[cfg(any(test, not(feature = "direct-license")))]
use crate::types::LicenseState;
use crate::types::LicenseStatus;

pub const STATUS_COMMAND: &str = "get_license_status";
pub const REFRESH_COMMAND: &str = "refresh_license";
pub const ACCOUNT_STATUS_COMMAND: &str = "get_account_status";
pub const BEGIN_SIGN_IN_COMMAND: &str = "begin_sign_in";
pub const SIGN_OUT_COMMAND: &str = "sign_out";
pub const DELETE_ACCOUNT_COMMAND: &str = "delete_account";
pub const START_CHECKOUT_COMMAND: &str = "start_checkout";
pub const BILLING_PORTAL_COMMAND: &str = "start_billing_portal";
pub const ACTIVATE_DEVICE_COMMAND: &str = "activate_this_device";
#[cfg(any(test, all(target_os = "macos", feature = "app-store")))]
const EXPECTED_BUNDLE_IDENTIFIER: &str = "com.onrecord.studio";

pub struct LicenseGate {
    status: Arc<RwLock<LicenseStatus>>,
    #[cfg(feature = "direct-license")]
    direct: Arc<direct::DirectGate>,
}

/// Process-wide handle to the direct gate so deep paths (ingestion start, evidence reads) can
/// re-verify the entitlement themselves without plumbing the managed state through.
#[cfg(all(feature = "direct-license", not(test)))]
static DIRECT_GATE: std::sync::OnceLock<Arc<direct::DirectGate>> = std::sync::OnceLock::new();

/// Independent entitlement re-check for code that must not trust the dispatch-time verdict.
/// Direct builds re-verify the cached token's signature, device binding and expiry; other
/// builds have no per-call check beyond the command gate.
pub fn recheck() -> Result<(), AppError> {
    #[cfg(all(feature = "direct-license", not(test)))]
    {
        DIRECT_GATE
            .get()
            .ok_or_else(|| AppError::License("The account gate was not initialized.".to_string()))?
            .reverify(direct::unix_now())
    }
    #[cfg(not(all(feature = "direct-license", not(test))))]
    {
        Ok(())
    }
}

impl LicenseGate {
    #[cfg(not(feature = "direct-license"))]
    pub fn initialize() -> Self {
        Self {
            status: Arc::new(RwLock::new(platform_status(false))),
        }
    }

    #[cfg(feature = "direct-license")]
    pub fn initialize() -> Self {
        #[cfg(all(target_os = "macos", not(test)))]
        let account: Arc<dyn account::NativeAccount> = Arc::new(account::MacAccount);
        #[cfg(any(test, not(target_os = "macos")))]
        let account: Arc<dyn account::NativeAccount> = Arc::new(account::UnavailableAccount);
        let server = client::HttpAccountServer::new()
            .map(|server| Arc::new(server) as Arc<dyn client::AccountServer>)
            .unwrap_or_else(|error| Arc::new(UnreachableServer(error)));
        let direct = Arc::new(direct::DirectGate::new(
            account,
            server,
            entitlement::pinned_keys(),
        ));
        #[cfg(not(test))]
        let _ = DIRECT_GATE.set(direct.clone());
        let gate = Self {
            status: Arc::new(RwLock::new(direct.evaluate(direct::unix_now()))),
            direct,
        };
        gate.spawn_background_refresh();
        gate
    }

    /// Re-checks every 15 minutes and refreshes when the entitlement has under 24 hours left.
    #[cfg(feature = "direct-license")]
    fn spawn_background_refresh(&self) {
        let (direct, status) = (self.direct.clone(), self.status.clone());
        tauri::async_runtime::spawn(async move {
            loop {
                let now = direct::unix_now();
                direct.note_time(now);
                if direct.needs_refresh(now) {
                    let refreshed = direct.refresh(now).await;
                    *status
                        .write()
                        .expect("license status lock must not be poisoned") = refreshed;
                }
                tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
            }
        });
    }

    #[cfg(feature = "direct-license")]
    pub fn direct(&self) -> &Arc<direct::DirectGate> {
        &self.direct
    }

    pub fn store_status(&self, status: LicenseStatus) {
        *self
            .status
            .write()
            .expect("license status lock must not be poisoned") = status;
    }

    pub fn status(&self) -> LicenseStatus {
        let cached = self
            .status
            .read()
            .expect("license status lock must not be poisoned")
            .clone();
        // A cached grant is re-derived once its token stops verifying, so the UI never shows a
        // stale "authorized" past an expiry.
        #[cfg(feature = "direct-license")]
        if cached.authorized && self.direct.reverify(direct::unix_now()).is_err() {
            let current = self.direct.evaluate(direct::unix_now());
            self.store_status(current.clone());
            return current;
        }
        cached
    }

    /// Re-derives the status from local state only (no network) and stores it.
    #[cfg(feature = "direct-license")]
    pub async fn reevaluate(&self) -> LicenseStatus {
        let direct = self.direct.clone();
        let status = tokio::task::spawn_blocking(move || direct.evaluate(direct::unix_now()))
            .await
            .unwrap_or_else(|error| direct::worker_failure(&error.to_string()));
        self.store_status(status.clone());
        status
    }

    #[cfg(feature = "direct-license")]
    pub async fn refresh(&self) -> LicenseStatus {
        self.activate_device(false).await
    }

    #[cfg(feature = "direct-license")]
    pub async fn activate_device(&self, confirm_revoke_oldest: bool) -> LicenseStatus {
        let result = self
            .direct
            .refresh_with(direct::unix_now(), confirm_revoke_oldest)
            .await;
        self.store_status(result.clone());
        result
    }

    #[cfg(not(feature = "direct-license"))]
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
        if !status.authorized {
            return Err(AppError::License(status.message));
        }
        self.reverify()
    }

    /// Verdict re-derived from the entitlement itself, not from the cached status.
    pub fn reverify(&self) -> Result<(), AppError> {
        #[cfg(feature = "direct-license")]
        {
            self.direct.reverify(direct::unix_now())
        }
        #[cfg(not(feature = "direct-license"))]
        {
            Ok(())
        }
    }
}

/// Stands in when the HTTP client cannot be built; every request fails as unreachable, so
/// nothing is granted beyond a still-valid stored entitlement.
#[cfg(feature = "direct-license")]
struct UnreachableServer(client::FetchError);

#[cfg(feature = "direct-license")]
impl client::AccountServer for UnreachableServer {
    fn fetch_entitlement<'a>(
        &'a self,
        _: &'a account::AccessToken,
        _: &'a client::EntitlementRequest,
    ) -> client::BoxFuture<'a, Result<String, client::FetchError>> {
        let error = client::FetchError::Unreachable(self.0.to_string());
        Box::pin(async move { Err(error) })
    }

    fn delete_account<'a>(
        &'a self,
        _: &'a account::AccessToken,
    ) -> client::BoxFuture<'a, Result<(), client::FetchError>> {
        let error = client::FetchError::Unreachable(self.0.to_string());
        Box::pin(async move { Err(error) })
    }

    fn start_checkout<'a>(
        &'a self,
        _: &'a account::AccessToken,
        _: Option<u32>,
    ) -> client::BoxFuture<'a, Result<String, client::FetchError>> {
        let error = client::FetchError::Unreachable(self.0.to_string());
        Box::pin(async move { Err(error) })
    }

    fn start_billing_portal<'a>(
        &'a self,
        _: &'a account::AccessToken,
    ) -> client::BoxFuture<'a, Result<String, client::FetchError>> {
        let error = client::FetchError::Unreachable(self.0.to_string());
        Box::pin(async move { Err(error) })
    }
}

pub fn command_is_unlicensed(command: &str) -> bool {
    matches!(
        command,
        STATUS_COMMAND
            | REFRESH_COMMAND
            | ACCOUNT_STATUS_COMMAND
            | BEGIN_SIGN_IN_COMMAND
            | SIGN_OUT_COMMAND
            | DELETE_ACCOUNT_COMMAND
            | START_CHECKOUT_COMMAND
            | BILLING_PORTAL_COMMAND
            | ACTIVATE_DEVICE_COMMAND
    )
}

#[cfg(not(any(
    feature = "direct-license",
    all(target_os = "macos", feature = "app-store")
)))]
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
        plan: None,
        trial_ends_at: None,
        days_left: None,
        account_email: None,
        sign_in_provider: None,
        entitlement_expires_at: None,
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
            plan: None,
            trial_ends_at: None,
            days_left: None,
            account_email: None,
            sign_in_provider: None,
            entitlement_expires_at: None,
        }
    }
}

#[cfg(not(feature = "direct-license"))]
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
        plan: None,
        trial_ends_at: None,
        days_left: None,
        account_email: None,
        sign_in_provider: None,
        entitlement_expires_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_license_recovery_commands_bypass_the_gate() {
        assert!(command_is_unlicensed(STATUS_COMMAND));
        assert!(command_is_unlicensed(REFRESH_COMMAND));
        for command in [
            "get_account_status",
            "begin_sign_in",
            "sign_out",
            "delete_account",
            "start_checkout",
            "activate_this_device",
        ] {
            assert!(command_is_unlicensed(command), "{command}");
        }
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
