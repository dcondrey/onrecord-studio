//! Account sign-in state for the direct build. These commands bypass the license gate: they are
//! how a locked-out user signs in, signs out, or leaves. Builds without accounts report that
//! plainly instead of granting anything.

use crate::error::AppError;
use crate::licensing::LicenseGate;
use crate::types::{
    AccountStatus, CheckoutSession, DeleteAccountReport, LicenseStatus, SignInReport,
};
use tauri::State;

#[cfg(not(feature = "direct-license"))]
fn not_in_this_build() -> AppError {
    AppError::License("Accounts are not part of this build.".to_string())
}

#[tauri::command]
pub async fn get_account_status(
    license: State<'_, LicenseGate>,
) -> Result<AccountStatus, AppError> {
    #[cfg(feature = "direct-license")]
    {
        use crate::licensing::account::SessionResult;
        let direct = license.direct().clone();
        let session = tokio::task::spawn_blocking(move || direct.session())
            .await
            .map_err(|error| AppError::Internal(format!("Account worker failed: {error}")))??;
        Ok(match session {
            SessionResult::SignedIn(session) => AccountStatus {
                available: true,
                signed_in: true,
                email: session.email,
                provider: session.provider,
            },
            SessionResult::SignedOut => AccountStatus {
                available: true,
                signed_in: false,
                email: None,
                provider: None,
            },
            SessionResult::Failed(message) => return Err(AppError::License(message)),
        })
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = license;
        Ok(AccountStatus {
            available: false,
            signed_in: false,
            email: None,
            provider: None,
        })
    }
}

#[tauri::command]
pub async fn begin_sign_in(
    provider: String,
    license: State<'_, LicenseGate>,
) -> Result<SignInReport, AppError> {
    #[cfg(feature = "direct-license")]
    {
        use crate::licensing::account::SignInResult;
        use crate::types::SignInOutcome;
        let direct = license.direct().clone();
        let result = tokio::task::spawn_blocking(move || direct.begin_sign_in(&provider))
            .await
            .map_err(|error| AppError::Internal(format!("Account worker failed: {error}")))??;
        let (outcome, message) = match result {
            SignInResult::SignedIn(_) => (SignInOutcome::SignedIn, None),
            SignInResult::Cancelled => (SignInOutcome::Cancelled, None),
            SignInResult::Failed(message) => (SignInOutcome::Failed, Some(message)),
        };
        let status = if outcome == SignInOutcome::SignedIn {
            license.refresh().await
        } else {
            license.status()
        };
        Ok(SignInReport {
            outcome,
            message,
            license: status,
        })
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = (provider, license);
        Err(not_in_this_build())
    }
}

#[tauri::command]
pub async fn sign_out(license: State<'_, LicenseGate>) -> Result<LicenseStatus, AppError> {
    #[cfg(feature = "direct-license")]
    {
        let direct = license.direct().clone();
        tokio::task::spawn_blocking(move || direct.sign_out())
            .await
            .map_err(|error| AppError::Internal(format!("Account worker failed: {error}")))??;
        Ok(license.reevaluate().await)
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = license;
        Err(not_in_this_build())
    }
}

#[tauri::command]
pub async fn delete_account(
    license: State<'_, LicenseGate>,
) -> Result<DeleteAccountReport, AppError> {
    #[cfg(feature = "direct-license")]
    {
        let report = license.direct().delete_account().await;
        Ok(DeleteAccountReport {
            remote_deleted: report.remote_deleted,
            signed_out: report.signed_out,
            local_cleared: report.local_cleared,
            not_done: report.not_done,
            license: license.reevaluate().await,
        })
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = license;
        Err(not_in_this_build())
    }
}

#[tauri::command]
pub async fn start_checkout(
    quantity: Option<u32>,
    license: State<'_, LicenseGate>,
) -> Result<CheckoutSession, AppError> {
    if quantity.is_some_and(|value| !(1..=999).contains(&value)) {
        return Err(AppError::License(
            "Quantity must be between 1 and 999.".to_string(),
        ));
    }
    #[cfg(feature = "direct-license")]
    {
        let url = license.direct().clone().start_checkout(quantity).await?;
        Ok(CheckoutSession { url })
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = license;
        Err(not_in_this_build())
    }
}

/// Opens Stripe's Customer Portal for the signed-in subscriber: payment method, seat count, and
/// cancellation. A cancellation there ends renewal at the current period's end; the license stays
/// active until then and the next entitlement refresh after that denies. Nothing is refunded by it.
#[tauri::command]
pub async fn start_billing_portal(
    license: State<'_, LicenseGate>,
) -> Result<CheckoutSession, AppError> {
    #[cfg(feature = "direct-license")]
    {
        let url = license.direct().clone().start_billing_portal().await?;
        Ok(CheckoutSession { url })
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = license;
        Err(not_in_this_build())
    }
}

/// Re-runs the entitlement flow for this Mac. `confirm_revoke_oldest` must come from an
/// explicit user confirmation: it lets the server revoke the account's oldest device.
#[tauri::command]
pub async fn activate_this_device(
    confirm_revoke_oldest: bool,
    license: State<'_, LicenseGate>,
) -> Result<LicenseStatus, AppError> {
    #[cfg(feature = "direct-license")]
    {
        Ok(license.activate_device(confirm_revoke_oldest).await)
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = (confirm_revoke_oldest, license);
        Err(not_in_this_build())
    }
}
