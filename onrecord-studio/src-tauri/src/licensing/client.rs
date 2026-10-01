//! HTTPS client for the entitlement and account-deletion endpoints. It carries the Supabase
//! access token as a Bearer credential and never logs it.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::account::AccessToken;

pub const ENTITLEMENT_URL: &str = "https://auth.writerslogic.com/functions/v1/onrecord-entitlement";
pub const DELETE_ACCOUNT_URL: &str =
    "https://auth.writerslogic.com/functions/v1/onrecord-delete-account";

pub const CHECKOUT_URL: &str = "https://auth.writerslogic.com/functions/v1/onrecord-checkout";
const CHECKOUT_HOST: &str = "checkout.stripe.com";
pub const MAX_CHECKOUT_QUANTITY: u32 = 999;
/// The shared `stripe-portal` function: it looks the customer up by the signed-in user, so it
/// needs no OnRecord-specific body beyond where the browser should return to.
pub const BILLING_PORTAL_URL: &str = "https://auth.writerslogic.com/functions/v1/stripe-portal";
const BILLING_PORTAL_HOST: &str = "billing.stripe.com";
const BILLING_PORTAL_RETURN_URL: &str = "https://writerslogic.com/onrecord";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESPONSE_BYTES: usize = 32 * 1024;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The server could not be reached: the offline path may still honor a stored token.
    #[error("The account server is unreachable: {0}")]
    Unreachable(String),
    /// The server answered and refused. Never falls back to a stored token.
    #[error("The account server refused the request: {0}")]
    Refused(String),
    /// The account is already active on the maximum number of Macs (HTTP 409 `device_limit`).
    #[error("This account is already active on the maximum number of Macs.")]
    DeviceLimit,
    #[error("The account server sent an unusable response: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntitlementRequest {
    pub challenge: String,
    pub device_signature: String,
    pub device_public_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoke_oldest_device: Option<bool>,
}

pub trait AccountServer: Send + Sync {
    /// Returns the compact JWS, unverified.
    fn fetch_entitlement<'a>(
        &'a self,
        access_token: &'a AccessToken,
        request: &'a EntitlementRequest,
    ) -> BoxFuture<'a, Result<String, FetchError>>;

    /// POST: the deployed function accepts no other method.
    fn delete_account<'a>(
        &'a self,
        access_token: &'a AccessToken,
    ) -> BoxFuture<'a, Result<(), FetchError>>;

    /// Returns a Stripe Checkout URL that `validate_checkout_url` has already accepted.
    fn start_checkout<'a>(
        &'a self,
        access_token: &'a AccessToken,
        quantity: Option<u32>,
    ) -> BoxFuture<'a, Result<String, FetchError>>;

    /// Returns a Stripe Customer Portal URL that `validate_billing_portal_url` has accepted. The
    /// portal is where a subscriber changes seats, payment method, or cancels; cancellation there
    /// stops renewal at the period end and the entitlement follows the subscription's status.
    fn start_billing_portal<'a>(
        &'a self,
        access_token: &'a AccessToken,
    ) -> BoxFuture<'a, Result<String, FetchError>>;
}

/// The only URL the app will hand to the system browser from a server reply.
pub fn validate_checkout_url(url: &str) -> Result<String, FetchError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| FetchError::Invalid("checkout URL is not a URL".to_string()))?;
    let acceptable = parsed.scheme() == "https"
        && parsed.host_str() == Some(CHECKOUT_HOST)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none();
    if acceptable {
        Ok(parsed.into())
    } else {
        Err(FetchError::Invalid(
            "checkout URL is not a Stripe Checkout address".to_string(),
        ))
    }
}

/// Same shape of check as the checkout URL, for the Customer Portal host.
pub fn validate_billing_portal_url(url: &str) -> Result<String, FetchError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| FetchError::Invalid("billing portal URL is not a URL".to_string()))?;
    let acceptable = parsed.scheme() == "https"
        && parsed.host_str() == Some(BILLING_PORTAL_HOST)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none();
    if acceptable {
        Ok(parsed.into())
    } else {
        Err(FetchError::Invalid(
            "billing portal URL is not a Stripe Customer Portal address".to_string(),
        ))
    }
}

#[derive(Deserialize)]
struct EntitlementResponse {
    token: String,
}

#[derive(Serialize)]
struct BillingPortalRequest {
    #[serde(rename = "returnUrl")]
    return_url: &'static str,
}

#[derive(Deserialize)]
struct CheckoutResponse {
    url: String,
}

#[derive(Serialize)]
struct CheckoutRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    quantity: Option<u32>,
}

pub struct HttpAccountServer {
    client: reqwest::Client,
    entitlement_url: String,
    delete_url: String,
    checkout_url: String,
    portal_url: String,
}

impl HttpAccountServer {
    pub fn new() -> Result<Self, FetchError> {
        Self::with_urls(
            ENTITLEMENT_URL,
            DELETE_ACCOUNT_URL,
            CHECKOUT_URL,
            BILLING_PORTAL_URL,
        )
    }

    fn with_urls(
        entitlement_url: &str,
        delete_url: &str,
        checkout_url: &str,
        portal_url: &str,
    ) -> Result<Self, FetchError> {
        for url in [entitlement_url, delete_url, checkout_url, portal_url] {
            require_https(url)?;
        }
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| FetchError::Invalid(format!("HTTP client: {error}")))?;
        Ok(Self {
            client,
            entitlement_url: entitlement_url.to_string(),
            delete_url: delete_url.to_string(),
            checkout_url: checkout_url.to_string(),
            portal_url: portal_url.to_string(),
        })
    }
}

fn require_https(url: &str) -> Result<(), FetchError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| FetchError::Invalid(format!("endpoint URL: {error}")))?;
    if parsed.scheme() == "https" && parsed.host_str().is_some() {
        Ok(())
    } else {
        Err(FetchError::Invalid("endpoint must be https".to_string()))
    }
}

fn transport(error: reqwest::Error) -> FetchError {
    // reqwest errors may embed the URL but never the Authorization header.
    FetchError::Unreachable(error.without_url().to_string())
}

async fn bounded_body(mut response: reqwest::Response) -> Result<Vec<u8>, FetchError> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(FetchError::Invalid("response too large".to_string()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn conflict_error(body: &[u8]) -> FetchError {
    if String::from_utf8_lossy(body).contains("device_limit") {
        FetchError::DeviceLimit
    } else {
        FetchError::Refused("server status 409 Conflict".to_string())
    }
}

fn status_error(status: reqwest::StatusCode) -> FetchError {
    if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        FetchError::Unreachable(format!("server status {status}"))
    } else {
        FetchError::Refused(format!("server status {status}"))
    }
}

impl AccountServer for HttpAccountServer {
    fn fetch_entitlement<'a>(
        &'a self,
        access_token: &'a AccessToken,
        request: &'a EntitlementRequest,
    ) -> BoxFuture<'a, Result<String, FetchError>> {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.entitlement_url)
                .bearer_auth(access_token.expose())
                .json(request)
                .send()
                .await
                .map_err(transport)?;
            let status = response.status();
            if status == reqwest::StatusCode::CONFLICT {
                let body = bounded_body(response).await.unwrap_or_default();
                return Err(conflict_error(&body));
            }
            if !status.is_success() {
                return Err(status_error(status));
            }
            let body = bounded_body(response).await?;
            serde_json::from_slice::<EntitlementResponse>(&body)
                .map(|parsed| parsed.token)
                .map_err(|_| FetchError::Invalid("missing token".to_string()))
        })
    }

    fn delete_account<'a>(
        &'a self,
        access_token: &'a AccessToken,
    ) -> BoxFuture<'a, Result<(), FetchError>> {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.delete_url)
                .bearer_auth(access_token.expose())
                .send()
                .await
                .map_err(transport)?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(status_error(response.status()))
            }
        })
    }

    fn start_checkout<'a>(
        &'a self,
        access_token: &'a AccessToken,
        quantity: Option<u32>,
    ) -> BoxFuture<'a, Result<String, FetchError>> {
        Box::pin(async move {
            if quantity.is_some_and(|value| !(1..=MAX_CHECKOUT_QUANTITY).contains(&value)) {
                return Err(FetchError::Refused(format!(
                    "quantity must be between 1 and {MAX_CHECKOUT_QUANTITY}"
                )));
            }
            let response = self
                .client
                .post(&self.checkout_url)
                .bearer_auth(access_token.expose())
                .json(&CheckoutRequest { quantity })
                .send()
                .await
                .map_err(transport)?;
            if !response.status().is_success() {
                return Err(status_error(response.status()));
            }
            let body = bounded_body(response).await?;
            let parsed: CheckoutResponse = serde_json::from_slice(&body)
                .map_err(|_| FetchError::Invalid("missing checkout url".to_string()))?;
            validate_checkout_url(&parsed.url)
        })
    }

    fn start_billing_portal<'a>(
        &'a self,
        access_token: &'a AccessToken,
    ) -> BoxFuture<'a, Result<String, FetchError>> {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.portal_url)
                .bearer_auth(access_token.expose())
                .json(&BillingPortalRequest {
                    return_url: BILLING_PORTAL_RETURN_URL,
                })
                .send()
                .await
                .map_err(transport)?;
            if !response.status().is_success() {
                return Err(status_error(response.status()));
            }
            let body = bounded_body(response).await?;
            let parsed: CheckoutResponse = serde_json::from_slice(&body)
                .map_err(|_| FetchError::Invalid("missing billing portal url".to_string()))?;
            validate_billing_portal_url(&parsed.url)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_endpoints_are_accepted() {
        assert!(HttpAccountServer::new().is_ok());
        for bad in ["http://auth.example/x", "ftp://a/b", "not a url"] {
            let good = [
                ENTITLEMENT_URL,
                DELETE_ACCOUNT_URL,
                CHECKOUT_URL,
                BILLING_PORTAL_URL,
            ];
            for slot in 0..good.len() {
                let mut urls = good;
                urls[slot] = bad;
                assert!(HttpAccountServer::with_urls(urls[0], urls[1], urls[2], urls[3]).is_err());
            }
        }
    }

    #[test]
    fn only_the_stripe_portal_host_is_handed_to_the_browser() {
        assert!(validate_billing_portal_url("https://billing.stripe.com/p/session/x").is_ok());
        for bad in [
            "http://billing.stripe.com/p/x",
            "https://checkout.stripe.com/p/x",
            "https://user@billing.stripe.com/p/x",
            "https://billing.stripe.com:8443/p/x",
        ] {
            assert!(validate_billing_portal_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_device_limit_conflict_is_typed_and_other_conflicts_are_refusals() {
        assert_eq!(
            conflict_error(br#"{"error":"device_limit"}"#),
            FetchError::DeviceLimit
        );
        assert!(matches!(conflict_error(b"{}"), FetchError::Refused(_)));
    }

    #[test]
    fn revoke_flag_is_serialized_only_when_confirmed() {
        let request = |flag| {
            serde_json::to_value(EntitlementRequest {
                challenge: "c".into(),
                device_signature: "s".into(),
                device_public_key: "k".into(),
                revoke_oldest_device: flag,
            })
            .unwrap()
        };
        assert!(request(None).get("revokeOldestDevice").is_none());
        assert_eq!(request(Some(true))["revokeOldestDevice"], true);
    }

    #[test]
    fn only_stripe_checkout_https_urls_are_returned() {
        assert!(validate_checkout_url("https://checkout.stripe.com/c/pay/cs_test_1#frag").is_ok());
        for bad in [
            "http://checkout.stripe.com/c/pay/x",
            "https://checkout.stripe.com.evil.example/x",
            "https://evil.example/checkout.stripe.com",
            "https://user@checkout.stripe.com/x",
            "https://checkout.stripe.com:8443/x",
            "https://stripe.com/x",
            "javascript:alert(1)",
            "",
        ] {
            assert!(validate_checkout_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn server_errors_are_offline_and_client_errors_are_refusals() {
        use reqwest::StatusCode;
        assert!(matches!(
            status_error(StatusCode::BAD_GATEWAY),
            FetchError::Unreachable(_)
        ));
        assert!(matches!(
            status_error(StatusCode::FORBIDDEN),
            FetchError::Refused(_)
        ));
    }
}
