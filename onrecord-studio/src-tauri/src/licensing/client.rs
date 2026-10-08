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
/// OnRecord's own portal function: it looks the customer up by the signed-in user and binds the
/// session to the OnRecord portal configuration (cancel at period end, seat quantity, payment
/// method). The return URL is fixed server-side; the body is empty.
pub const BILLING_PORTAL_URL: &str =
    "https://auth.writerslogic.com/functions/v1/onrecord-billing-portal";
const BILLING_PORTAL_HOST: &str = "billing.stripe.com";
pub const SEATS_URL: &str = "https://auth.writerslogic.com/functions/v1/onrecord-seats";

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

    /// Seat management for the purchaser of an organization subscription. Returns the raw JSON
    /// body; the server's documented refusals are turned into readable `Refused` errors here.
    fn seats<'a>(
        &'a self,
        access_token: &'a AccessToken,
        request: &'a SeatRequest,
    ) -> BoxFuture<'a, Result<Vec<u8>, FetchError>>;
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum SeatRequest {
    List,
    Invite { email: String },
    Revoke { email: String },
}

/// The `error` codes `onrecord-seats` documents, as sentences the account dialog can show.
pub fn seat_refusal(status: reqwest::StatusCode, body: &[u8]) -> FetchError {
    #[derive(Deserialize)]
    struct ErrorBody {
        error: String,
    }
    let code = serde_json::from_slice::<ErrorBody>(body)
        .map(|parsed| parsed.error)
        .unwrap_or_default();
    let message = match code.as_str() {
        "no_active_subscription" => {
            "Only the purchaser of an active subscription can manage seats."
        }
        "seat_limit" => {
            "Every purchased seat is assigned. Revoke one or add seats in Manage subscription."
        }
        "seat_not_found" => "No seat is assigned to that address.",
        "owner_seat_is_implicit" => "Your own address already holds a seat as the purchaser.",
        "invalid_email" => "That is not a valid email address.",
        "rate_limited" => {
            return FetchError::Unreachable("Too many seat changes; try again later.".to_string())
        }
        _ => return status_error(status),
    };
    FetchError::Refused(message.to_string())
}

/// A server-supplied URL is handed to the system browser only when it is plain https on exactly
/// `host`, with no credentials and no port.
fn stripe_url(url: &str, host: &str, what: &str) -> Result<String, FetchError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| FetchError::Invalid(format!("{what} URL is not a URL")))?;
    let acceptable = parsed.scheme() == "https"
        && parsed.host_str() == Some(host)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none();
    if acceptable {
        Ok(parsed.into())
    } else {
        Err(FetchError::Invalid(format!(
            "{what} URL is not a {host} address"
        )))
    }
}

pub fn validate_checkout_url(url: &str) -> Result<String, FetchError> {
    stripe_url(url, CHECKOUT_HOST, "checkout")
}

pub fn validate_billing_portal_url(url: &str) -> Result<String, FetchError> {
    stripe_url(url, BILLING_PORTAL_HOST, "billing portal")
}

#[derive(Deserialize)]
struct EntitlementResponse {
    token: String,
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
    seats_url: String,
}

impl HttpAccountServer {
    pub fn new() -> Result<Self, FetchError> {
        Self::with_urls(
            ENTITLEMENT_URL,
            DELETE_ACCOUNT_URL,
            CHECKOUT_URL,
            BILLING_PORTAL_URL,
            SEATS_URL,
        )
    }

    fn with_urls(
        entitlement_url: &str,
        delete_url: &str,
        checkout_url: &str,
        portal_url: &str,
        seats_url: &str,
    ) -> Result<Self, FetchError> {
        for url in [
            entitlement_url,
            delete_url,
            checkout_url,
            portal_url,
            seats_url,
        ] {
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
            seats_url: seats_url.to_string(),
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
                .send()
                .await
                .map_err(transport)?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(FetchError::Refused(
                    "No billing account exists for this sign-in yet; subscribe first.".to_string(),
                ));
            }
            if !response.status().is_success() {
                return Err(status_error(response.status()));
            }
            let body = bounded_body(response).await?;
            let parsed: CheckoutResponse = serde_json::from_slice(&body)
                .map_err(|_| FetchError::Invalid("missing billing portal url".to_string()))?;
            validate_billing_portal_url(&parsed.url)
        })
    }

    fn seats<'a>(
        &'a self,
        access_token: &'a AccessToken,
        request: &'a SeatRequest,
    ) -> BoxFuture<'a, Result<Vec<u8>, FetchError>> {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.seats_url)
                .bearer_auth(access_token.expose())
                .json(request)
                .send()
                .await
                .map_err(transport)?;
            let status = response.status();
            let body = bounded_body(response).await?;
            if !status.is_success() {
                return Err(seat_refusal(status, &body));
            }
            Ok(body)
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
                SEATS_URL,
            ];
            for slot in 0..good.len() {
                let mut urls = good;
                urls[slot] = bad;
                assert!(
                    HttpAccountServer::with_urls(urls[0], urls[1], urls[2], urls[3], urls[4])
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn seat_refusals_become_readable_messages_and_unknown_codes_keep_the_status() {
        let refused = |code: &str| {
            seat_refusal(
                reqwest::StatusCode::CONFLICT,
                format!(r#"{{"error":"{code}"}}"#).as_bytes(),
            )
        };
        assert!(matches!(refused("seat_limit"), FetchError::Refused(m) if m.contains("Revoke")));
        assert!(matches!(
            refused("rate_limited"),
            FetchError::Unreachable(_)
        ));
        assert!(matches!(refused("something_new"), FetchError::Refused(m) if m.contains("409")));
        assert!(matches!(
            seat_refusal(reqwest::StatusCode::INTERNAL_SERVER_ERROR, b"not json"),
            FetchError::Unreachable(_)
        ));
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
    fn only_the_expected_stripe_https_host_is_handed_to_the_browser() {
        assert!(validate_checkout_url("https://checkout.stripe.com/c/pay/cs_test_1#frag").is_ok());
        assert!(validate_billing_portal_url("https://billing.stripe.com/p/session/x").is_ok());
        for bad in [
            "http://checkout.stripe.com/c/pay/x",
            "https://checkout.stripe.com.evil.example/x",
            "https://evil.example/checkout.stripe.com",
            "https://user@checkout.stripe.com/x",
            "https://checkout.stripe.com:8443/x",
            "https://billing.stripe.com/p/x",
            "https://stripe.com/x",
            "javascript:alert(1)",
            "",
        ] {
            assert!(validate_checkout_url(bad).is_err(), "{bad}");
        }
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
