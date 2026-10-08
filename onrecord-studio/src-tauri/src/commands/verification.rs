//! Work-email verification: a one-time code sent to a mailbox proves control of that mailbox at
//! its domain, nothing more. The result is a domain recorded in the local profile; it changes an
//! affiliation's basis only through an explicit link the user chooses (`profile::verified`).
//!
//! The address and the code are never logged and never written to disk. The address is held in
//! memory between "send" and "confirm" so a code requested for one address cannot be spent as
//! proof for another.

use crate::{
    blocking,
    error::AppError,
    profile::{verified, ProfileStore},
    types::profile::*,
};
use std::sync::Arc;
use tauri::State;

#[cfg(not(feature = "direct-license"))]
use crate::commands::account::not_in_this_build;

#[cfg(any(test, feature = "direct-license"))]
mod guard {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const MAXIMUM_EMAIL_CHARACTERS: usize = 254;
    const MAXIMUM_LOCAL_PART_CHARACTERS: usize = 64;
    const MAXIMUM_DOMAIN_CHARACTERS: usize = 253;
    const CODE_DIGITS: usize = 6;
    /// Matches the server's hourly send limit reply when it sends no usable `Retry-After`.
    pub(crate) const DEFAULT_RETRY_AFTER_SECONDS: u32 = 3600;
    pub(crate) const MAXIMUM_RETRY_AFTER_SECONDS: u32 = 86_400;

    /// The server's rules (`onrecord-verify-work-email/email.ts`): ASCII, one `@`, a DNS-shaped
    /// domain with at least two labels and a non-numeric last label. Returns the lowercased address.
    pub(crate) fn validate_email(raw: &str) -> Option<String> {
        if raw.chars().count() > MAXIMUM_EMAIL_CHARACTERS {
            return None;
        }
        let address = raw.trim().to_ascii_lowercase();
        if !address.is_ascii() {
            return None;
        }
        let (local, domain) = address.split_once('@')?;
        if domain.contains('@') || local.is_empty() || local.len() > MAXIMUM_LOCAL_PART_CHARACTERS {
            return None;
        }
        let local_ok = local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+/=?^_`{|}~-".contains(&byte));
        if !local_ok || local.starts_with('.') || local.ends_with('.') || local.contains("..") {
            return None;
        }
        let labels: Vec<&str> = domain.split('.').collect();
        let label_ok = |label: &&str| {
            let bytes = label.as_bytes();
            (1..=63).contains(&bytes.len())
                && bytes
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
                && bytes[0] != b'-'
                && bytes[bytes.len() - 1] != b'-'
        };
        if labels.len() < 2
            || domain.len() > MAXIMUM_DOMAIN_CHARACTERS
            || !labels.iter().all(label_ok)
            || labels
                .last()
                .is_some_and(|last| last.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        Some(address)
    }

    pub(crate) fn validate_code(raw: &str) -> Option<String> {
        let code = raw.trim();
        (code.len() == CODE_DIGITS && code.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| code.to_string())
    }

    pub(crate) fn domain_of(address: &str) -> &str {
        address.rsplit_once('@').map_or("", |(_, domain)| domain)
    }

    struct Pending {
        user_id: String,
        address: String,
        expires_at: Instant,
    }

    /// The one outstanding code request of this session, in memory only.
    pub(crate) struct PendingWorkEmail(Mutex<Option<Pending>>);

    pub(crate) enum PendingCheck {
        Match,
        Missing,
        Expired,
    }

    impl PendingWorkEmail {
        pub(crate) const fn new() -> Self {
            Self(Mutex::new(None))
        }

        fn slot(&self) -> std::sync::MutexGuard<'_, Option<Pending>> {
            // A poisoned lock still holds a coherent Option; recovering never widens what is proven.
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }

        pub(crate) fn set(&self, user_id: &str, address: &str, ttl: Duration) {
            *self.slot() = Some(Pending {
                user_id: user_id.to_string(),
                address: address.to_string(),
                expires_at: Instant::now() + ttl,
            });
        }

        pub(crate) fn clear(&self) {
            *self.slot() = None;
        }

        pub(crate) fn check(&self, user_id: &str, address: &str) -> PendingCheck {
            let mut slot = self.slot();
            match slot.as_ref() {
                Some(pending) if pending.user_id == user_id && pending.address == address => {
                    if Instant::now() >= pending.expires_at {
                        *slot = None;
                        PendingCheck::Expired
                    } else {
                        PendingCheck::Match
                    }
                }
                _ => PendingCheck::Missing,
            }
        }
    }
}

#[cfg(any(test, feature = "direct-license"))]
use guard::*;

#[cfg(feature = "direct-license")]
static PENDING: PendingWorkEmail = PendingWorkEmail::new();

#[cfg(any(test, feature = "direct-license"))]
mod transport {
    use super::*;
    use crate::licensing::account::{AccessToken, NativeAccount, SessionResult};
    use serde::Deserialize;
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Duration;

    pub const WORK_EMAIL_URL: &str =
        "https://auth.writerslogic.com/functions/v1/onrecord-verify-work-email";
    const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
    const MAX_RESPONSE_BYTES: usize = 8 * 1024;

    pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum RequestAnswer {
        Sent { expires_in_seconds: u32 },
        InvalidEmail,
        RateLimited { retry_after_seconds: u32 },
        Unauthorized,
        SendFailed,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ConfirmAnswer {
        Verified { domain: String, is_free_mail: bool },
        CodeInvalid { attempts_remaining: Option<u32> },
        ExpiredOrExhausted,
        RateLimited { retry_after_seconds: u32 },
        Unauthorized,
    }

    pub trait WorkEmailServer: Send + Sync {
        fn request_code<'a>(
            &'a self,
            token: &'a AccessToken,
            address: &'a str,
        ) -> BoxFuture<'a, Result<RequestAnswer, AppError>>;
        fn confirm_code<'a>(
            &'a self,
            token: &'a AccessToken,
            code: &'a str,
        ) -> BoxFuture<'a, Result<ConfirmAnswer, AppError>>;
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Body {
        error: Option<String>,
        expires_in_seconds: Option<u32>,
        attempts_remaining: Option<u32>,
        domain: Option<String>,
        is_free_mail: Option<bool>,
    }

    fn unusable(what: &str) -> AppError {
        AppError::Network(format!("The verification server sent an unusable {what}."))
    }

    fn parse(body: &[u8]) -> Body {
        serde_json::from_slice(body).unwrap_or(Body {
            error: None,
            expires_in_seconds: None,
            attempts_remaining: None,
            domain: None,
            is_free_mail: None,
        })
    }

    pub(super) fn retry_after(header: Option<&str>) -> u32 {
        header
            .and_then(|value| value.trim().parse::<u32>().ok())
            .map_or(DEFAULT_RETRY_AFTER_SECONDS, |seconds| {
                seconds.clamp(1, MAXIMUM_RETRY_AFTER_SECONDS)
            })
    }

    pub(super) fn interpret_request(
        status: u16,
        retry_after_header: Option<&str>,
        body: &[u8],
    ) -> Result<RequestAnswer, AppError> {
        let parsed = parse(body);
        match (status, parsed.error.as_deref()) {
            (200, _) => match parsed.expires_in_seconds {
                Some(expires_in_seconds) if expires_in_seconds > 0 => {
                    Ok(RequestAnswer::Sent { expires_in_seconds })
                }
                _ => Err(unusable("response")),
            },
            (401, _) => Ok(RequestAnswer::Unauthorized),
            (400, Some("invalid_email")) => Ok(RequestAnswer::InvalidEmail),
            (429, _) => Ok(RequestAnswer::RateLimited {
                retry_after_seconds: retry_after(retry_after_header),
            }),
            (502, Some("send_failed")) => Ok(RequestAnswer::SendFailed),
            _ => Err(AppError::Network(format!(
                "The verification server refused the request (status {status})."
            ))),
        }
    }

    pub(super) fn interpret_confirm(
        status: u16,
        retry_after_header: Option<&str>,
        body: &[u8],
    ) -> Result<ConfirmAnswer, AppError> {
        let parsed = parse(body);
        match (status, parsed.error.as_deref()) {
            (200, _) => match parsed.domain {
                Some(domain) if !domain.is_empty() => Ok(ConfirmAnswer::Verified {
                    domain: domain.to_ascii_lowercase(),
                    // A missing flag is not "not free mail": refuse to guess.
                    is_free_mail: parsed.is_free_mail.ok_or_else(|| unusable("response"))?,
                }),
                _ => Err(unusable("response")),
            },
            (401, _) => Ok(ConfirmAnswer::Unauthorized),
            (400, Some("code_invalid")) => Ok(ConfirmAnswer::CodeInvalid {
                attempts_remaining: parsed.attempts_remaining,
            }),
            (400, Some("code_expired_or_exhausted")) => Ok(ConfirmAnswer::ExpiredOrExhausted),
            (429, _) => Ok(ConfirmAnswer::RateLimited {
                retry_after_seconds: retry_after(retry_after_header),
            }),
            _ => Err(AppError::Network(format!(
                "The verification server refused the request (status {status})."
            ))),
        }
    }

    pub struct HttpWorkEmailServer {
        client: reqwest::Client,
        url: String,
    }

    impl HttpWorkEmailServer {
        pub fn new() -> Result<Self, AppError> {
            Self::with_url(WORK_EMAIL_URL)
        }

        pub(super) fn with_url(url: &str) -> Result<Self, AppError> {
            let parsed = reqwest::Url::parse(url)
                .map_err(|error| AppError::Network(format!("endpoint URL: {error}")))?;
            if parsed.scheme() != "https" || parsed.host_str().is_none() {
                return Err(AppError::Network("endpoint must be https".to_string()));
            }
            let client = reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(CONNECT_TIMEOUT)
                .timeout(REQUEST_TIMEOUT)
                .build()
                .map_err(|error| AppError::Network(format!("HTTP client: {error}")))?;
            Ok(Self {
                client,
                url: url.to_string(),
            })
        }

        async fn post(
            &self,
            token: &AccessToken,
            body: serde_json::Value,
        ) -> Result<(u16, Option<String>, Vec<u8>), AppError> {
            // reqwest errors may embed the URL but never the Authorization header or the body.
            let transport = |error: reqwest::Error| {
                AppError::Network(format!(
                    "The verification server is unreachable: {}",
                    error.without_url()
                ))
            };
            let mut response = self
                .client
                .post(&self.url)
                .bearer_auth(token.expose())
                .json(&body)
                .send()
                .await
                .map_err(transport)?;
            let status = response.status().as_u16();
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string);
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(transport)? {
                if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
                    return Err(unusable("response (too large)"));
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok((status, retry_after, bytes))
        }
    }

    impl WorkEmailServer for HttpWorkEmailServer {
        fn request_code<'a>(
            &'a self,
            token: &'a AccessToken,
            address: &'a str,
        ) -> BoxFuture<'a, Result<RequestAnswer, AppError>> {
            Box::pin(async move {
                let (status, retry, body) = self
                    .post(
                        token,
                        serde_json::json!({ "action": "request", "email": address }),
                    )
                    .await?;
                interpret_request(status, retry.as_deref(), &body)
            })
        }

        fn confirm_code<'a>(
            &'a self,
            token: &'a AccessToken,
            code: &'a str,
        ) -> BoxFuture<'a, Result<ConfirmAnswer, AppError>> {
            Box::pin(async move {
                let (status, retry, body) = self
                    .post(
                        token,
                        serde_json::json!({ "action": "confirm", "code": code }),
                    )
                    .await?;
                interpret_confirm(status, retry.as_deref(), &body)
            })
        }
    }

    pub enum Credentials {
        NotSignedIn,
        SignedIn { user_id: String, token: AccessToken },
    }

    pub async fn credentials(account: &Arc<dyn NativeAccount>) -> Result<Credentials, AppError> {
        let account = Arc::clone(account);
        blocking::spawn("Work email session", move || match account.session()? {
            SessionResult::SignedIn(session) => Ok(Credentials::SignedIn {
                user_id: session.user_id,
                token: account.access_token()?,
            }),
            SessionResult::SignedOut => Ok(Credentials::NotSignedIn),
            SessionResult::Failed(message) => Err(AppError::License(message)),
        })
        .await?
    }

    fn code_report(
        status: WorkEmailCodeStatus,
        expires_in_seconds: Option<u32>,
        retry_after_seconds: Option<u32>,
    ) -> WorkEmailCodeReport {
        WorkEmailCodeReport {
            status,
            expires_in_seconds,
            retry_after_seconds,
        }
    }

    pub async fn request_code(
        account: &Arc<dyn NativeAccount>,
        server: &dyn WorkEmailServer,
        pending: &PendingWorkEmail,
        email: &str,
    ) -> Result<WorkEmailCodeReport, AppError> {
        let Some(address) = validate_email(email) else {
            return Ok(code_report(WorkEmailCodeStatus::InvalidEmail, None, None));
        };
        let (user_id, token) = match credentials(account).await? {
            Credentials::NotSignedIn => {
                return Ok(code_report(WorkEmailCodeStatus::NotSignedIn, None, None))
            }
            Credentials::SignedIn { user_id, token } => (user_id, token),
        };
        Ok(match server.request_code(&token, &address).await? {
            RequestAnswer::Sent { expires_in_seconds } => {
                pending.set(
                    &user_id,
                    &address,
                    Duration::from_secs(u64::from(expires_in_seconds)),
                );
                code_report(WorkEmailCodeStatus::Sent, Some(expires_in_seconds), None)
            }
            RequestAnswer::InvalidEmail => {
                code_report(WorkEmailCodeStatus::InvalidEmail, None, None)
            }
            // No new code was issued, so an earlier pending request still stands.
            RequestAnswer::RateLimited {
                retry_after_seconds,
            } => code_report(
                WorkEmailCodeStatus::RateLimited,
                None,
                Some(retry_after_seconds),
            ),
            RequestAnswer::Unauthorized => {
                code_report(WorkEmailCodeStatus::NotSignedIn, None, None)
            }
            RequestAnswer::SendFailed => {
                // The server replaced its code, so the earlier one is dead too.
                pending.clear();
                code_report(WorkEmailCodeStatus::SendFailed, None, None)
            }
        })
    }

    fn confirm_report(status: WorkEmailConfirmStatus) -> WorkEmailConfirmReport {
        WorkEmailConfirmReport {
            status,
            domain: None,
            is_free_mail: false,
            attempts_remaining: None,
            verified_at: None,
            retry_after_seconds: None,
        }
    }

    /// `Verified` carries the domain but no `verified_at`: the caller records it and fills that in.
    pub async fn confirm_code(
        account: &Arc<dyn NativeAccount>,
        server: &dyn WorkEmailServer,
        pending: &PendingWorkEmail,
        email: &str,
        code: &str,
    ) -> Result<WorkEmailConfirmReport, AppError> {
        let (Some(address), Some(code)) = (validate_email(email), validate_code(code)) else {
            return Ok(confirm_report(WorkEmailConfirmStatus::InvalidInput));
        };
        let (user_id, token) = match credentials(account).await? {
            Credentials::NotSignedIn => {
                return Ok(confirm_report(WorkEmailConfirmStatus::NotSignedIn))
            }
            Credentials::SignedIn { user_id, token } => (user_id, token),
        };
        match pending.check(&user_id, &address) {
            PendingCheck::Match => {}
            PendingCheck::Missing => {
                return Ok(confirm_report(WorkEmailConfirmStatus::NoPendingRequest))
            }
            PendingCheck::Expired => {
                return Ok(confirm_report(WorkEmailConfirmStatus::ExpiredOrExhausted))
            }
        }
        Ok(match server.confirm_code(&token, &code).await? {
            ConfirmAnswer::Verified {
                domain,
                is_free_mail,
            } => {
                pending.clear();
                if domain != domain_of(&address) {
                    // The server confirmed a different domain than the one asked for: a code for
                    // another address must never be recorded as proof for this one.
                    return Err(AppError::Network(
                        "The verification server confirmed a different domain than requested."
                            .to_string(),
                    ));
                }
                WorkEmailConfirmReport {
                    domain: Some(domain),
                    is_free_mail,
                    ..confirm_report(WorkEmailConfirmStatus::Verified)
                }
            }
            ConfirmAnswer::CodeInvalid { attempts_remaining } => {
                if attempts_remaining == Some(0) {
                    pending.clear();
                }
                WorkEmailConfirmReport {
                    attempts_remaining,
                    ..confirm_report(WorkEmailConfirmStatus::CodeInvalid)
                }
            }
            ConfirmAnswer::ExpiredOrExhausted => {
                pending.clear();
                confirm_report(WorkEmailConfirmStatus::ExpiredOrExhausted)
            }
            ConfirmAnswer::RateLimited {
                retry_after_seconds,
            } => WorkEmailConfirmReport {
                retry_after_seconds: Some(retry_after_seconds),
                ..confirm_report(WorkEmailConfirmStatus::RateLimited)
            },
            ConfirmAnswer::Unauthorized => confirm_report(WorkEmailConfirmStatus::NotSignedIn),
        })
    }
}

#[cfg(feature = "direct-license")]
fn native_account() -> Arc<dyn crate::licensing::account::NativeAccount> {
    // The native account is stateless (Keychain behind a C ABI), so a second handle sees the same
    // session as the license gate's.
    #[cfg(all(target_os = "macos", not(test)))]
    {
        Arc::new(crate::licensing::account::MacAccount)
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        Arc::new(crate::licensing::account::UnavailableAccount)
    }
}

#[tauri::command]
pub async fn request_work_email_code(email: String) -> Result<WorkEmailCodeReport, AppError> {
    #[cfg(feature = "direct-license")]
    {
        let server = transport::HttpWorkEmailServer::new()?;
        transport::request_code(&native_account(), &server, &PENDING, &email).await
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = email;
        Err(not_in_this_build())
    }
}

#[tauri::command]
pub async fn confirm_work_email_code(
    email: String,
    code: String,
    store: State<'_, Arc<ProfileStore>>,
) -> Result<WorkEmailConfirmReport, AppError> {
    #[cfg(feature = "direct-license")]
    {
        let server = transport::HttpWorkEmailServer::new()?;
        let mut report =
            transport::confirm_code(&native_account(), &server, &PENDING, &email, &code).await?;
        if let (WorkEmailConfirmStatus::Verified, Some(domain)) =
            (report.status, report.domain.clone())
        {
            let is_free_mail = report.is_free_mail;
            let saved = blocking::on_shared(&store, "Record verified domain", move |store| {
                store.update(|current| Ok(verified::record_domain(current, &domain, is_free_mail)))
            })
            .await?;
            report.verified_at = saved
                .verified_domains
                .iter()
                .find(|item| Some(&item.domain) == report.domain.as_ref())
                .map(|item| item.verified_at.clone());
        }
        Ok(report)
    }
    #[cfg(not(feature = "direct-license"))]
    {
        let _ = (email, code, store);
        Err(not_in_this_build())
    }
}

/// The user's explicit choice. A refusal is a typed result with its reason, not an error, and it
/// writes nothing.
#[tauri::command]
pub async fn link_affiliation_verified_domain(
    input: AffiliationDomainLinkInput,
    store: State<'_, Arc<ProfileStore>>,
) -> Result<AffiliationLinkReport, AppError> {
    blocking::on_shared(&store, "Link verified domain", move |store| {
        let mut outcome = None;
        let saved = store.update(|current| {
            let current = current.ok_or_else(|| {
                AppError::DataParsing("There is no profile to link a domain in".to_string())
            })?;
            let (next, status, message) = verified::link_affiliation(current, &input);
            outcome = Some((status, message));
            Ok(next)
        })?;
        let (status, message) = outcome
            .ok_or_else(|| AppError::Internal("The link decision was not produced".to_string()))?;
        Ok(AffiliationLinkReport {
            status,
            message,
            profile: ProfileState {
                status: ProfileStatus::Set,
                profile: Some(saved),
            },
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::transport::*;
    use super::*;
    use crate::licensing::account::test_support::TestAccount;
    use crate::licensing::account::{AccessToken, NativeAccount, SessionResult};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    struct FakeServer {
        request: Mutex<Vec<RequestAnswer>>,
        confirm: Mutex<Vec<ConfirmAnswer>>,
        confirm_calls: AtomicUsize,
        seen_tokens: Mutex<Vec<String>>,
    }

    impl FakeServer {
        fn new(request: Vec<RequestAnswer>, confirm: Vec<ConfirmAnswer>) -> Self {
            Self {
                request: Mutex::new(request),
                confirm: Mutex::new(confirm),
                confirm_calls: AtomicUsize::new(0),
                seen_tokens: Mutex::new(Vec::new()),
            }
        }
    }

    impl WorkEmailServer for FakeServer {
        fn request_code<'a>(
            &'a self,
            token: &'a AccessToken,
            _: &'a str,
        ) -> BoxFuture<'a, Result<RequestAnswer, AppError>> {
            self.seen_tokens
                .lock()
                .unwrap()
                .push(token.expose().to_string());
            let answer = self.request.lock().unwrap().remove(0);
            Box::pin(async move { Ok(answer) })
        }
        fn confirm_code<'a>(
            &'a self,
            _: &'a AccessToken,
            _: &'a str,
        ) -> BoxFuture<'a, Result<ConfirmAnswer, AppError>> {
            self.confirm_calls.fetch_add(1, Ordering::SeqCst);
            let answer = self.confirm.lock().unwrap().remove(0);
            Box::pin(async move { Ok(answer) })
        }
    }

    fn account() -> Arc<dyn NativeAccount> {
        Arc::new(TestAccount::signed_in("thumb"))
    }

    fn sent() -> RequestAnswer {
        RequestAnswer::Sent {
            expires_in_seconds: 600,
        }
    }

    fn verified(domain: &str, is_free_mail: bool) -> ConfirmAnswer {
        ConfirmAnswer::Verified {
            domain: domain.to_string(),
            is_free_mail,
        }
    }

    #[test]
    fn email_validation_accepts_the_servers_forms_and_refuses_the_rest() {
        assert_eq!(
            validate_email("  Jo.Doe+x@Sandiego.GOV ").as_deref(),
            Some("jo.doe+x@sandiego.gov")
        );
        let local_max = "a".repeat(64);
        assert!(validate_email(&format!("{local_max}@a.co")).is_some());
        assert!(validate_email(&format!("{}@a.co", "a".repeat(65))).is_none());
        let long_domain = format!(
            "{}.{}.{}.com",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63)
        );
        assert!(validate_email(&format!("x@{long_domain}")).is_some());
        assert!(validate_email(&format!("{}@{}.com", "a".repeat(64), "b".repeat(190))).is_none());
        assert!(validate_email(&format!("x@{}", "a".repeat(255))).is_none());
        for bad in [
            "",
            "x",
            "@a.co",
            "x@",
            "x@a",
            "x@@a.co",
            "x@a@b.co",
            "x@a.123",
            "x@[1.2.3.4]",
            ".x@a.co",
            "x.@a.co",
            "x..y@a.co",
            "x@-a.co",
            "x@a-.co",
            "x@a..co",
            "x y@a.co",
            "\"x\"@a.co",
            "jörg@a.co",
            "x@exämple.com",
        ] {
            assert!(validate_email(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn code_validation_is_exactly_six_digits() {
        assert_eq!(validate_code(" 012345 ").as_deref(), Some("012345"));
        for bad in [
            "",
            "12345",
            "1234567",
            "12345a",
            "١٢٣٤٥٦",
            "123 456",
            "+12345",
        ] {
            assert!(validate_code(bad).is_none(), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn a_free_mail_confirmation_is_reported_as_free_mail_and_never_upgrades() {
        let server = FakeServer::new(vec![sent()], vec![verified("gmail.com", true)]);
        let pending = PendingWorkEmail::new();
        request_code(&account(), &server, &pending, "me@gmail.com")
            .await
            .unwrap();
        let report = confirm_code(&account(), &server, &pending, "me@gmail.com", "123456")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailConfirmStatus::Verified);
        assert!(report.is_free_mail);
        let profile = verified::record_domain(None, "gmail.com", report.is_free_mail);
        let profile = crate::profile::apply_input(
            Some(profile),
            ProfileInput {
                display_name: None,
                affiliations: vec![AffiliationInput {
                    id: None,
                    name: "Gmail".into(),
                    aliases: vec![],
                }],
            },
        )
        .unwrap();
        let input = AffiliationDomainLinkInput {
            affiliation_id: profile.affiliations[0].id.clone(),
            domain: "gmail.com".into(),
        };
        let (after, status, _) = verified::link_affiliation(profile, &input);
        assert_eq!(status, AffiliationLinkStatus::FreeMailDomain);
        assert_eq!(after.affiliations[0].basis, AffiliationBasis::SelfDeclared);
    }

    #[tokio::test]
    async fn a_code_requested_for_one_address_cannot_verify_another() {
        let server = FakeServer::new(vec![sent()], vec![verified("evil.example", false)]);
        let pending = PendingWorkEmail::new();
        request_code(&account(), &server, &pending, "a@acme.com")
            .await
            .unwrap();
        // Different address, same domain: still refused before any attempt is spent.
        let report = confirm_code(&account(), &server, &pending, "b@acme.com", "123456")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailConfirmStatus::NoPendingRequest);
        assert_eq!(server.confirm_calls.load(Ordering::SeqCst), 0);
        // Right address, but the server names another domain: discarded, never recorded.
        let result = confirm_code(&account(), &server, &pending, "a@acme.com", "123456").await;
        assert!(matches!(result, Err(AppError::Network(_))));
        // The pending request is spent either way.
        let again = confirm_code(&account(), &server, &pending, "a@acme.com", "123456")
            .await
            .unwrap();
        assert_eq!(again.status, WorkEmailConfirmStatus::NoPendingRequest);
    }

    #[tokio::test]
    async fn a_pending_request_belongs_to_the_session_that_made_it() {
        let server = FakeServer::new(vec![sent()], vec![]);
        let pending = PendingWorkEmail::new();
        request_code(&account(), &server, &pending, "a@acme.com")
            .await
            .unwrap();
        pending.set("someone-else", "a@acme.com", Duration::from_secs(60));
        let report = confirm_code(&account(), &server, &pending, "a@acme.com", "123456")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailConfirmStatus::NoPendingRequest);
    }

    #[tokio::test]
    async fn signed_out_and_malformed_input_never_reach_the_server() {
        let out = TestAccount::signed_in("t");
        out.state().session = SessionResult::SignedOut;
        let out: Arc<dyn NativeAccount> = Arc::new(out);
        let server = FakeServer::new(vec![], vec![]);
        let pending = PendingWorkEmail::new();
        let report = request_code(&out, &server, &pending, "a@acme.com")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailCodeStatus::NotSignedIn);
        let report = confirm_code(&out, &server, &pending, "a@acme.com", "123456")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailConfirmStatus::NotSignedIn);
        let report = request_code(&account(), &server, &pending, "nope")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailCodeStatus::InvalidEmail);
        let report = confirm_code(&account(), &server, &pending, "a@acme.com", "12345")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailConfirmStatus::InvalidInput);
        assert_eq!(server.confirm_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn wrong_codes_report_attempts_and_the_last_one_ends_the_request() {
        let server = FakeServer::new(
            vec![sent()],
            vec![
                ConfirmAnswer::CodeInvalid {
                    attempts_remaining: Some(1),
                },
                ConfirmAnswer::CodeInvalid {
                    attempts_remaining: Some(0),
                },
            ],
        );
        let pending = PendingWorkEmail::new();
        request_code(&account(), &server, &pending, "a@acme.com")
            .await
            .unwrap();
        let first = confirm_code(&account(), &server, &pending, "a@acme.com", "111111")
            .await
            .unwrap();
        assert_eq!(
            (first.status, first.attempts_remaining),
            (WorkEmailConfirmStatus::CodeInvalid, Some(1))
        );
        let last = confirm_code(&account(), &server, &pending, "a@acme.com", "222222")
            .await
            .unwrap();
        assert_eq!(last.attempts_remaining, Some(0));
        let after = confirm_code(&account(), &server, &pending, "a@acme.com", "333333")
            .await
            .unwrap();
        assert_eq!(after.status, WorkEmailConfirmStatus::NoPendingRequest);
    }

    #[tokio::test]
    async fn rate_limit_keeps_the_earlier_request_and_send_failure_drops_it() {
        let server = FakeServer::new(
            vec![
                sent(),
                RequestAnswer::RateLimited {
                    retry_after_seconds: 3600,
                },
                RequestAnswer::SendFailed,
            ],
            vec![verified("acme.com", false)],
        );
        let pending = PendingWorkEmail::new();
        request_code(&account(), &server, &pending, "a@acme.com")
            .await
            .unwrap();
        let limited = request_code(&account(), &server, &pending, "a@acme.com")
            .await
            .unwrap();
        assert_eq!(
            (limited.status, limited.retry_after_seconds),
            (WorkEmailCodeStatus::RateLimited, Some(3600))
        );
        request_code(&account(), &server, &pending, "a@acme.com")
            .await
            .unwrap();
        let after = confirm_code(&account(), &server, &pending, "a@acme.com", "123456")
            .await
            .unwrap();
        assert_eq!(after.status, WorkEmailConfirmStatus::NoPendingRequest);
    }

    #[tokio::test]
    async fn an_expired_request_is_reported_without_asking_the_server() {
        let server = FakeServer::new(vec![], vec![]);
        let pending = PendingWorkEmail::new();
        pending.set("user-1", "a@acme.com", Duration::ZERO);
        let report = confirm_code(&account(), &server, &pending, "a@acme.com", "123456")
            .await
            .unwrap();
        assert_eq!(report.status, WorkEmailConfirmStatus::ExpiredOrExhausted);
        assert_eq!(server.confirm_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn server_replies_map_to_typed_answers() {
        assert_eq!(
            interpret_request(200, None, br#"{"ok":true,"expiresInSeconds":600}"#).unwrap(),
            sent()
        );
        assert!(interpret_request(200, None, b"{}").is_err());
        assert_eq!(
            interpret_request(429, Some("120"), br#"{"error":"rate_limited"}"#).unwrap(),
            RequestAnswer::RateLimited {
                retry_after_seconds: 120
            }
        );
        assert_eq!(
            interpret_request(429, Some("999999999"), b"").unwrap(),
            RequestAnswer::RateLimited {
                retry_after_seconds: MAXIMUM_RETRY_AFTER_SECONDS
            }
        );
        assert_eq!(
            interpret_request(429, None, b"").unwrap(),
            RequestAnswer::RateLimited {
                retry_after_seconds: DEFAULT_RETRY_AFTER_SECONDS
            }
        );
        assert_eq!(
            interpret_request(400, None, br#"{"error":"invalid_email"}"#).unwrap(),
            RequestAnswer::InvalidEmail
        );
        assert_eq!(
            interpret_request(502, None, br#"{"error":"send_failed"}"#).unwrap(),
            RequestAnswer::SendFailed
        );
        assert_eq!(
            interpret_request(401, None, br#"{"error":"unauthorized"}"#).unwrap(),
            RequestAnswer::Unauthorized
        );
        assert!(interpret_request(500, None, br#"{"error":"server_error"}"#).is_err());
        assert_eq!(
            interpret_confirm(
                400,
                None,
                br#"{"error":"code_invalid","attemptsRemaining":3}"#
            )
            .unwrap(),
            ConfirmAnswer::CodeInvalid {
                attempts_remaining: Some(3)
            }
        );
        assert_eq!(
            interpret_confirm(400, None, br#"{"error":"code_expired_or_exhausted"}"#).unwrap(),
            ConfirmAnswer::ExpiredOrExhausted
        );
        assert_eq!(
            interpret_confirm(
                200,
                None,
                br#"{"ok":true,"domain":"Acme.com","isFreeMail":false,"proves":"mailbox_control"}"#
            )
            .unwrap(),
            verified("acme.com", false)
        );
        // A missing free-mail flag is refused rather than read as "not free mail".
        assert!(interpret_confirm(200, None, br#"{"ok":true,"domain":"gmail.com"}"#).is_err());
    }

    #[test]
    fn only_https_endpoints_are_accepted() {
        assert!(HttpWorkEmailServer::new().is_ok());
        for bad in ["http://auth.example/x", "ftp://a/b", "not a url"] {
            assert!(HttpWorkEmailServer::with_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn nothing_in_the_recorded_profile_carries_the_address_or_code() {
        let profile = verified::record_domain(None, "acme.com", false);
        let text = serde_json::to_string(&profile).unwrap();
        assert!(!text.contains("me@") && !text.contains("123456"));
        let _ = crate::profile::disclosure_sentence(Some(&profile));
    }
}
