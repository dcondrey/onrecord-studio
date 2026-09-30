use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LicenseState {
    Development,
    Verified,
    Revoked,
    Unverified,
    Unavailable,
    BundleMismatch,
    SignInRequired,
    Trial,
    TrialExpired,
    Licensed,
    ClockRolledBack,
    EntitlementExpired,
    DeviceLimitReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LicensePlan {
    Trial,
    Licensed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    pub enforced: bool,
    pub authorized: bool,
    pub state: LicenseState,
    pub message: String,
    pub bundle_id: Option<String>,
    pub app_version: Option<String>,
    pub original_app_version: Option<String>,
    pub app_transaction_id: Option<String>,
    pub app_id: Option<u64>,
    pub environment: Option<String>,
    pub original_purchase_date: Option<String>,
    pub signed_date: Option<String>,
    pub revocation_date: Option<String>,
    /// Present only when a verified entitlement named it.
    pub plan: Option<LicensePlan>,
    /// RFC 3339 UTC. Present exactly when the state is `Trial` or `TrialExpired`.
    pub trial_ends_at: Option<String>,
    pub days_left: Option<i64>,
    pub account_email: Option<String>,
    pub sign_in_provider: Option<String>,
    pub entitlement_expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutSession {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    /// False in builds that have no account system (App Store, development).
    pub available: bool,
    pub signed_in: bool,
    pub email: Option<String>,
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignInOutcome {
    SignedIn,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignInReport {
    pub outcome: SignInOutcome,
    pub message: Option<String>,
    pub license: LicenseStatus,
}

/// Each part is reported separately: local clearing succeeds offline, server deletion may not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAccountReport {
    pub remote_deleted: bool,
    pub signed_out: bool,
    pub local_cleared: bool,
    pub not_done: Vec<String>,
    pub license: LicenseStatus,
}
