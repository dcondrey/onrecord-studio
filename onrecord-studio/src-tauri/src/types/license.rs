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
}
