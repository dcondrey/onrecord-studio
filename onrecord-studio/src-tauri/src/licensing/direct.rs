//! Account and entitlement gate for the direct (Developer ID) build. Nothing here trusts a
//! stored boolean: every decision re-derives from the signed token, the device thumbprint and
//! the rollback guard. Anything that is not a verified grant is a denial.

use std::sync::{Arc, Mutex};

use rand::RngCore;

use super::account::{
    AccessToken, AccountSession, DeviceSignature, IntegrityResult, NativeAccount, SessionResult,
    SignInResult,
};
use super::client::{AccountServer, EntitlementRequest, FetchError, SeatRequest};
use super::clock::{ClockGuard, ClockMarks, ClockStore};
use super::entitlement::{self, Denial, Grant, NonceCheck, PinnedKey, VerifyContext};
use crate::error::AppError;
use crate::types::{LicensePlan, LicenseState, LicenseStatus, SeatStatus};

const TOKEN_KEY: &str = "entitlement_token";
const CLOCK_KEY: &str = "max_seen_time";
const CLOCK_NOW_KEY: &str = "max_seen_now";
pub const REFRESH_WINDOW_SECONDS: i64 = 24 * 3600;
const DAY_SECONDS: i64 = 24 * 3600;

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX)
        })
}

fn rfc3339(seconds: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(seconds, 0).map(|time| time.to_rfc3339())
}

struct KeychainClockStore(Arc<dyn NativeAccount>);

impl KeychainClockStore {
    fn read(&self, key: &str) -> Result<Option<i64>, AppError> {
        self.0
            .secure_get(key)?
            .map(|value| {
                value
                    .parse::<i64>()
                    .map_err(|_| AppError::License("Stored clock guard is corrupt.".to_string()))
            })
            .transpose()
    }

    fn write(&self, key: &str, value: Option<i64>) -> Result<(), AppError> {
        value.map_or(Ok(()), |value| self.0.secure_set(key, &value.to_string()))
    }
}

impl ClockStore for KeychainClockStore {
    fn load(&self) -> Result<ClockMarks, AppError> {
        Ok(ClockMarks {
            max_iat: self.read(CLOCK_KEY)?,
            max_now: self.read(CLOCK_NOW_KEY)?,
        })
    }

    fn store(&self, marks: ClockMarks) -> Result<(), AppError> {
        self.write(CLOCK_KEY, marks.max_iat)?;
        self.write(CLOCK_NOW_KEY, marks.max_now)
    }
}

#[derive(Default)]
struct Cache {
    token: Option<String>,
    thumbprint: Option<String>,
}

pub struct DirectGate {
    account: Arc<dyn NativeAccount>,
    server: Arc<dyn AccountServer>,
    keys: Vec<PinnedKey>,
    clock: Result<ClockGuard, String>,
    cache: Mutex<Cache>,
}

struct Prepared {
    session: AccountSession,
    access_token: AccessToken,
    challenge: String,
    signature: DeviceSignature,
}

/// What the server-side and local parts of an account deletion did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletionReport {
    pub remote_deleted: bool,
    pub signed_out: bool,
    pub local_cleared: bool,
    pub not_done: Vec<String>,
}

impl DirectGate {
    pub fn new(
        account: Arc<dyn NativeAccount>,
        server: Arc<dyn AccountServer>,
        keys: Vec<PinnedKey>,
    ) -> Self {
        let clock = ClockGuard::open(Box::new(KeychainClockStore(account.clone())))
            .map_err(|error| error.to_string());
        Self {
            account,
            server,
            keys,
            clock,
            cache: Mutex::new(Cache::default()),
        }
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache
            .lock()
            .expect("entitlement cache must not be poisoned")
    }

    fn thumbprint(&self) -> Result<String, AppError> {
        if let Some(cached) = self.cache().thumbprint.clone() {
            return Ok(cached);
        }
        let probe = random_challenge();
        let signature = self.account.device_sign(&probe)?;
        self.cache().thumbprint = Some(signature.thumbprint.clone());
        Ok(signature.thumbprint)
    }

    fn stored_token(&self) -> Result<Option<String>, AppError> {
        if let Some(token) = self.cache().token.clone() {
            return Ok(Some(token));
        }
        let loaded = self.account.secure_get(TOKEN_KEY)?;
        if let Some(token) = &loaded {
            self.cache().token = Some(token.clone());
        }
        Ok(loaded)
    }

    fn clear_token(&self) -> Result<(), AppError> {
        self.cache().token = None;
        self.account.secure_set(TOKEN_KEY, "")
    }

    /// The application's integrity, then the signed-in session; anything else is the denial.
    fn verified_session(&self) -> Result<AccountSession, LicenseStatus> {
        if let IntegrityResult::Tampered(detail) = self.account.integrity_check() {
            return Err(denied(
                LicenseState::Unverified,
                format!("The application failed its integrity check. {detail}"),
                None,
            ));
        }
        match self.account.session() {
            Ok(SessionResult::SignedIn(session)) => Ok(session),
            Ok(SessionResult::SignedOut) => Err(denied(
                LicenseState::SignInRequired,
                "Sign in to use OnRecord Studio.".to_string(),
                None,
            )),
            Ok(SessionResult::Failed(message)) => {
                Err(denied(LicenseState::Unavailable, message, None))
            }
            Err(error) => Err(denied(LicenseState::Unavailable, error.to_string(), None)),
        }
    }

    /// Full evaluation: integrity, session, clock, device, then the stored token.
    pub fn evaluate(&self, now: i64) -> LicenseStatus {
        match self.verified_session() {
            Ok(session) => self.evaluate_token(now, Some(&session)),
            Err(status) => status,
        }
    }

    fn evaluate_token(&self, now: i64, session: Option<&AccountSession>) -> LicenseStatus {
        let clock = match &self.clock {
            Ok(clock) => clock,
            Err(message) => {
                return denied(LicenseState::Unavailable, message.clone(), session);
            }
        };
        if clock.is_rolled_back(now) {
            return denied(
                LicenseState::ClockRolledBack,
                "The system clock is behind a time already seen. Correct it and go online to refresh."
                    .to_string(),
                session,
            );
        }
        if let Err(error) = clock.note_now(now) {
            return denied(LicenseState::Unavailable, error.to_string(), session);
        }
        let now = clock.effective(now);
        let thumbprint = match self.thumbprint() {
            Ok(value) => value,
            Err(error) => return denied(LicenseState::Unavailable, error.to_string(), session),
        };
        let token = match self.stored_token() {
            Ok(Some(token)) => token,
            Ok(None) => {
                return denied(
                    LicenseState::Unavailable,
                    "No entitlement is stored. Go online to refresh.".to_string(),
                    session,
                )
            }
            Err(error) => return denied(LicenseState::Unavailable, error.to_string(), session),
        };
        let context = VerifyContext {
            now,
            nonce: NonceCheck::Stored,
            device_thumbprint: &thumbprint,
        };
        match entitlement::verify(&token, &self.keys, &context) {
            Ok(verified) => {
                if let Some(mismatch) = subject_mismatch(&verified, session) {
                    return mismatch;
                }
                if let Err(error) = clock.observe(verified.issued_at) {
                    return denied(LicenseState::Unavailable, error.to_string(), session);
                }
                granted(now, &verified, session)
            }
            Err(denial) => denial_status(now, &denial, session),
        }
    }

    /// Cheap independent check for command dispatch, ingestion start and evidence reads: no
    /// native call and no shared verdict, only the cached token's signature, device, and expiry
    /// plus the rollback guard.
    pub fn reverify(&self, now: i64) -> Result<(), AppError> {
        let deny = |message: String| Err(AppError::License(message));
        let Ok(clock) = &self.clock else {
            return deny("The clock guard is unavailable.".to_string());
        };
        if clock.is_rolled_back(now) {
            return deny("The system clock is behind a time already seen.".to_string());
        }
        // Best effort: a failed write still leaves the in-memory mark advanced.
        let _ = clock.note_now(now);
        match self.verify_cached(clock.effective(now)) {
            Some(Ok(_)) => Ok(()),
            Some(Err(denial)) => deny(denial.to_string()),
            None => deny("No verified entitlement is loaded.".to_string()),
        }
    }

    /// The cached token verified as a stored one, or `None` when no token or thumbprint is
    /// cached. Makes no native call.
    fn verify_cached(&self, now: i64) -> Option<Result<entitlement::VerifiedEntitlement, Denial>> {
        let (token, thumbprint) = {
            let cache = self.cache();
            (cache.token.clone()?, cache.thumbprint.clone()?)
        };
        let context = VerifyContext {
            now,
            nonce: NonceCheck::Stored,
            device_thumbprint: &thumbprint,
        };
        Some(entitlement::verify(&token, &self.keys, &context))
    }

    /// Advances the persisted wall-clock high-water mark; called on the background tick.
    pub fn note_time(&self, now: i64) {
        if let Ok(clock) = &self.clock {
            let _ = clock.note_now(now);
        }
    }

    /// True when the stored entitlement is missing, unverifiable, or has under 24 hours left.
    pub fn needs_refresh(&self, now: i64) -> bool {
        match self.verify_cached(now) {
            Some(Ok(verified)) => verified.expires_at - now < REFRESH_WINDOW_SECONDS,
            Some(Err(_)) | None => true,
        }
    }

    /// Asks the server for a fresh entitlement. The server's answer is verified like any other
    /// token. Offline, the stored entitlement decides; a refusal never falls back to it.
    pub async fn refresh(self: &Arc<Self>, now: i64) -> LicenseStatus {
        self.refresh_with(now, false).await
    }

    /// `revoke_oldest_device` is sent only when the user explicitly confirmed it; the server
    /// then frees a slot by revoking the account's least recently active device.
    pub async fn refresh_with(
        self: &Arc<Self>,
        now: i64,
        revoke_oldest_device: bool,
    ) -> LicenseStatus {
        let worker = self.clone();
        let prepared = tokio::task::spawn_blocking(move || worker.prepare_request()).await;
        let Prepared {
            session,
            access_token,
            challenge,
            signature,
        } = match prepared {
            Ok(Ok(prepared)) => prepared,
            Ok(Err(status)) => return *status,
            Err(error) => return worker_failure(&error.to_string()),
        };
        let request = EntitlementRequest {
            challenge: challenge.clone(),
            device_signature: signature.signature_b64url,
            device_public_key: signature.public_key_b64url,
            revoke_oldest_device: revoke_oldest_device.then_some(true),
        };
        match self.server.fetch_entitlement(&access_token, &request).await {
            Ok(token) => {
                self.accept_fresh_token(now, &token, &challenge, &signature.thumbprint, &session)
            }
            Err(FetchError::DeviceLimit) => {
                let _ = self.clear_token();
                denied(
                    LicenseState::DeviceLimitReached,
                    FetchError::DeviceLimit.to_string(),
                    Some(&session),
                )
            }
            Err(FetchError::Refused(message)) => {
                // An unreadable Keychain also fails every later read closed.
                let _ = self.clear_token();
                denied(LicenseState::Unverified, message, Some(&session))
            }
            Err(FetchError::Unreachable(_) | FetchError::Invalid(_)) => {
                self.evaluate_token(now, Some(&session))
            }
        }
    }

    fn prepare_request(&self) -> Result<Prepared, Box<LicenseStatus>> {
        let session = self.verified_session().map_err(Box::new)?;
        let fail = |error: AppError| {
            Box::new(denied(
                LicenseState::Unavailable,
                error.to_string(),
                Some(&session),
            ))
        };
        let access_token = self.account.access_token().map_err(fail)?;
        let challenge = random_challenge();
        let signature = self.account.device_sign(&challenge).map_err(fail)?;
        self.cache().thumbprint = Some(signature.thumbprint.clone());
        Ok(Prepared {
            session,
            access_token,
            challenge,
            signature,
        })
    }

    fn accept_fresh_token(
        &self,
        now: i64,
        token: &str,
        challenge: &str,
        thumbprint: &str,
        session: &AccountSession,
    ) -> LicenseStatus {
        let context = VerifyContext {
            now,
            nonce: NonceCheck::Challenge(challenge),
            device_thumbprint: thumbprint,
        };
        let verified = match entitlement::verify(token, &self.keys, &context) {
            Ok(verified) => verified,
            Err(denial) => return denial_status(now, &denial, Some(session)),
        };
        if let Some(mismatch) = subject_mismatch(&verified, Some(session)) {
            return mismatch;
        }
        let Ok(clock) = &self.clock else {
            return denied(
                LicenseState::Unavailable,
                "The clock guard is unavailable.".to_string(),
                Some(session),
            );
        };
        if let Err(error) = clock.reset_after_online(now, verified.issued_at) {
            return denied(LicenseState::Unavailable, error.to_string(), Some(session));
        }
        let persisted = {
            let mut cache = self.cache();
            cache.token = Some(token.to_string());
            self.account.secure_set(TOKEN_KEY, token)
        };
        let mut status = granted(now, &verified, Some(session));
        if persisted.is_err() {
            status
                .message
                .push_str(" The entitlement could not be saved for offline use.");
        }
        status
    }

    /// The access token of the signed-in session, or `License(not_signed_in)` when there is none.
    /// The Keychain read happens off the async runtime like every other account call.
    async fn signed_in_token(
        self: &Arc<Self>,
        not_signed_in: &'static str,
    ) -> Result<AccessToken, AppError> {
        let worker = self.clone();
        tokio::task::spawn_blocking(move || match worker.account.session() {
            Ok(SessionResult::SignedIn(_)) => worker.account.access_token(),
            Ok(_) => Err(AppError::License(not_signed_in.to_string())),
            Err(error) => Err(error),
        })
        .await
        .map_err(|error| AppError::Internal(format!("Account worker failed: {error}")))?
    }

    /// Requires a live session; the server decides the price and the quantity is bounded.
    pub async fn start_checkout(
        self: &Arc<Self>,
        quantity: Option<u32>,
    ) -> Result<String, AppError> {
        let token = self.signed_in_token("Sign in before purchasing.").await?;
        self.server
            .start_checkout(&token, quantity)
            .await
            .map_err(|error| AppError::Network(error.to_string()))
    }

    pub async fn start_billing_portal(self: &Arc<Self>) -> Result<String, AppError> {
        let token = self
            .signed_in_token("Sign in to manage your subscription.")
            .await?;
        self.server
            .start_billing_portal(&token)
            .await
            .map_err(|error| AppError::Network(error.to_string()))
    }

    /// Applies an invite or revoke and returns the purchaser's current seat list; a bare `List`
    /// only reads. A refusal the server documents arrives as a `License` error with its sentence.
    pub async fn seats(self: &Arc<Self>, request: SeatRequest) -> Result<SeatStatus, AppError> {
        let token = self.signed_in_token("Sign in to manage seats.").await?;
        let token = &token;
        let call = |request: SeatRequest| async move {
            self.server
                .seats(token, &request)
                .await
                .map_err(|error| match error {
                    FetchError::Refused(message) => AppError::License(message),
                    other => AppError::Network(other.to_string()),
                })
        };
        if !matches!(request, SeatRequest::List) {
            call(request).await?;
        }
        let body = call(SeatRequest::List).await?;
        serde_json::from_slice::<SeatStatus>(&body).map_err(|_| {
            AppError::Network("The account server sent an unusable seat list.".to_string())
        })
    }

    pub fn begin_sign_in(&self, provider: &str) -> Result<SignInResult, AppError> {
        self.account.sign_in(provider)
    }

    pub fn session(&self) -> Result<SessionResult, AppError> {
        self.account.session()
    }

    /// Clears the Keychain session and every cached entitlement.
    pub fn sign_out(&self) -> Result<(), AppError> {
        self.account.sign_out()?;
        self.clear_token()
    }

    /// Server deletion first (native sign-out invalidates the token it needs), then native
    /// sign-out, then local entitlement state. Each step runs regardless of the earlier ones,
    /// so the local part never depends on the network; the report names exactly what was not
    /// done.
    pub async fn delete_account(self: &Arc<Self>) -> DeletionReport {
        let mut not_done = Vec::new();
        let remote_deleted = match self.account.access_token() {
            Ok(token) => match self.server.delete_account(&token).await {
                Ok(()) => true,
                Err(error) => {
                    not_done.push(format!(
                        "Server-side account deletion was not completed: {error}"
                    ));
                    false
                }
            },
            Err(error) => {
                not_done.push(format!(
                    "Server-side account deletion was not attempted, no access token: {error}"
                ));
                false
            }
        };
        let signed_out = match self.account.sign_out() {
            Ok(()) => true,
            Err(error) => {
                not_done.push(format!("Sign-out failed: {error}"));
                false
            }
        };
        let local_cleared = match self.clear_token() {
            Ok(()) => true,
            Err(error) => {
                not_done.push(format!("Stored entitlement was not cleared: {error}"));
                false
            }
        };
        DeletionReport {
            remote_deleted,
            signed_out,
            local_cleared,
            not_done,
        }
    }
}

fn random_challenge() -> String {
    use base64::Engine;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn base_status(
    state: LicenseState,
    authorized: bool,
    message: String,
    session: Option<&AccountSession>,
) -> LicenseStatus {
    LicenseStatus {
        enforced: true,
        authorized,
        state,
        message,
        bundle_id: Some(entitlement::AUDIENCE.to_string()),
        app_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        original_app_version: None,
        app_transaction_id: None,
        app_id: None,
        environment: Some("direct".to_string()),
        original_purchase_date: None,
        signed_date: None,
        revocation_date: None,
        plan: None,
        trial_ends_at: None,
        days_left: None,
        account_email: session.and_then(|session| session.email.clone()),
        sign_in_provider: session.and_then(|session| session.provider.clone()),
        entitlement_expires_at: None,
    }
}

pub fn worker_failure(detail: &str) -> LicenseStatus {
    denied(
        LicenseState::Unavailable,
        format!("The account worker failed: {detail}"),
        None,
    )
}

/// A token is valid only for the account signed in now.
fn subject_mismatch(
    verified: &entitlement::VerifiedEntitlement,
    session: Option<&AccountSession>,
) -> Option<LicenseStatus> {
    let session = session?;
    (verified.subject != session.user_id).then(|| {
        denied(
            LicenseState::Unverified,
            "The stored entitlement belongs to a different account.".to_string(),
            Some(session),
        )
    })
}

fn denied(state: LicenseState, message: String, session: Option<&AccountSession>) -> LicenseStatus {
    base_status(state, false, message, session)
}

fn granted(
    now: i64,
    verified: &entitlement::VerifiedEntitlement,
    session: Option<&AccountSession>,
) -> LicenseStatus {
    let mut status = match verified.grant {
        Grant::Licensed => base_status(
            LicenseState::Licensed,
            true,
            "OnRecord Studio is licensed.".to_string(),
            session,
        ),
        Grant::Trial { ends_at } => {
            let days_left = (ends_at - now + DAY_SECONDS - 1) / DAY_SECONDS;
            let mut status = base_status(
                LicenseState::Trial,
                true,
                format!("Trial: {days_left} day(s) left."),
                session,
            );
            status.trial_ends_at = rfc3339(ends_at);
            status.days_left = Some(days_left);
            status
        }
    };
    status.plan = Some(verified.grant.plan());
    status.entitlement_expires_at = rfc3339(verified.expires_at);
    status
}

fn denial_status(now: i64, denial: &Denial, session: Option<&AccountSession>) -> LicenseStatus {
    let message = denial.to_string();
    match *denial {
        Denial::Expired { expires_at } => {
            let mut status = denied(LicenseState::EntitlementExpired, message, session);
            status.entitlement_expires_at = rfc3339(expires_at);
            status
        }
        Denial::TrialExpired { ends_at } => {
            let mut status = denied(LicenseState::TrialExpired, message, session);
            status.plan = Some(LicensePlan::Trial);
            status.trial_ends_at = rfc3339(ends_at);
            status.days_left = Some(0.max((ends_at - now) / DAY_SECONDS));
            status
        }
        Denial::IssuedInFuture => denied(LicenseState::ClockRolledBack, message, session),
        Denial::Malformed(_)
        | Denial::UnknownKey
        | Denial::BadSignature
        | Denial::WrongClaim(_)
        | Denial::LifetimeTooLong => denied(LicenseState::Unverified, message, session),
    }
}

#[cfg(test)]
mod tests {
    use super::super::account::test_support::TestAccount;
    use super::super::client::BoxFuture;
    use super::super::entitlement::test_support::*;
    use super::*;

    const THUMB: &str = "thumb-1";

    // The Keychain bridge refuses any key outside its allow-list, and a refused clock key leaves
    // every license check denied. Tests run against a mock account, so check the Swift list here.
    #[test]
    fn every_stored_key_is_allowed_by_the_native_keychain_bridge() {
        let swift = include_str!("../../native/AppAccount.swift");
        let allowed = swift
            .lines()
            .find(|line| line.contains("static let allowedKeys"))
            .expect("the bridge declares its allowed keys");
        for key in [TOKEN_KEY, CLOCK_KEY, CLOCK_NOW_KEY] {
            assert!(
                allowed.contains(&format!("\"{key}\"")),
                "{key} is not in the bridge allow-list: {allowed}"
            );
        }
    }

    type ReplyFn = Box<dyn FnMut(&EntitlementRequest) -> Result<String, FetchError> + Send>;

    /// Serves whatever the test scripted, signing claims with the request's own challenge.
    struct FakeServer {
        reply: Mutex<ReplyFn>,
        delete: Mutex<Result<(), FetchError>>,
        watch: Mutex<Option<TestAccount>>,
        signed_out_at_delete: Mutex<Option<u32>>,
    }

    impl FakeServer {
        fn new(
            reply: impl FnMut(&EntitlementRequest) -> Result<String, FetchError> + Send + 'static,
        ) -> Arc<Self> {
            Arc::new(Self {
                reply: Mutex::new(Box::new(reply)),
                delete: Mutex::new(Ok(())),
                watch: Mutex::new(None),
                signed_out_at_delete: Mutex::new(None),
            })
        }
    }

    impl AccountServer for FakeServer {
        fn fetch_entitlement<'a>(
            &'a self,
            _: &'a AccessToken,
            request: &'a EntitlementRequest,
        ) -> BoxFuture<'a, Result<String, FetchError>> {
            let result = (self.reply.lock().unwrap())(request);
            Box::pin(async move { result })
        }
        fn delete_account<'a>(
            &'a self,
            _: &'a AccessToken,
        ) -> BoxFuture<'a, Result<(), FetchError>> {
            if let Some(account) = self.watch.lock().unwrap().as_ref() {
                *self.signed_out_at_delete.lock().unwrap() = Some(account.state().signed_out);
            }
            let result = self.delete.lock().unwrap().clone();
            Box::pin(async move { result })
        }
        fn start_checkout<'a>(
            &'a self,
            _: &'a AccessToken,
            _: Option<u32>,
        ) -> BoxFuture<'a, Result<String, FetchError>> {
            Box::pin(async { Ok("https://checkout.stripe.com/c/pay/x".to_string()) })
        }
        fn start_billing_portal<'a>(
            &'a self,
            _: &'a AccessToken,
        ) -> BoxFuture<'a, Result<String, FetchError>> {
            Box::pin(async { Ok("https://billing.stripe.com/p/session/x".to_string()) })
        }
        fn seats<'a>(
            &'a self,
            _: &'a AccessToken,
            _: &'a SeatRequest,
        ) -> BoxFuture<'a, Result<Vec<u8>, FetchError>> {
            Box::pin(async {
                Ok(
                    br#"{"subscriptionActive":true,"quantity":3,"usedSeats":0,"seats":[]}"#
                        .to_vec(),
                )
            })
        }
    }

    fn gate_with(
        account: &TestAccount,
        signer: &TestSigner,
        server: Arc<FakeServer>,
    ) -> Arc<DirectGate> {
        Arc::new(DirectGate::new(
            Arc::new(account.clone()),
            server,
            vec![signer.pinned()],
        ))
    }

    fn licensed_server(signer: Arc<TestSigner>) -> Arc<FakeServer> {
        FakeServer::new(move |request| Ok(signer.sign(&licensed_claims(&request.challenge, THUMB))))
    }

    fn offline_server() -> Arc<FakeServer> {
        FakeServer::new(|_| Err(FetchError::Unreachable("offline".to_string())))
    }

    #[tokio::test]
    async fn a_fresh_licensed_entitlement_authorizes_and_is_persisted() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        let gate = gate_with(&account, &signer, licensed_server(signer.clone()));
        let status = gate.refresh(NOW).await;
        assert!(status.authorized, "{}", status.message);
        assert_eq!(status.state, LicenseState::Licensed);
        assert_eq!(
            status.account_email.as_deref(),
            Some("reporter@example.org")
        );
        assert!(account.state().store.contains_key(TOKEN_KEY));
        gate.reverify(NOW).unwrap();
    }

    #[tokio::test]
    async fn offline_with_a_valid_stored_token_authorizes_and_expired_denies() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        gate_with(&account, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;

        // New process, no network: only the stored token exists.
        let offline = gate_with(&account, &signer, offline_server());
        let status = offline.refresh(NOW).await;
        assert!(status.authorized, "{}", status.message);
        assert_eq!(status.state, LicenseState::Licensed);

        let later = NOW + 4 * 24 * 3600;
        let restarted = gate_with(&account, &signer, offline_server());
        let status = restarted.refresh(later).await;
        assert!(!status.authorized);
        assert_eq!(status.state, LicenseState::EntitlementExpired);
        assert!(restarted.reverify(later).is_err());
    }

    #[tokio::test]
    async fn trial_reports_days_left_and_expires() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        let ends_at = NOW + 2 * 24 * 3600 - 60;
        let server = {
            let signer = signer.clone();
            FakeServer::new(move |request| {
                Ok(signer.sign(&trial_claims(&request.challenge, THUMB, ends_at)))
            })
        };
        let gate = gate_with(&account, &signer, server);
        let status = gate.refresh(NOW).await;
        assert_eq!(status.state, LicenseState::Trial);
        assert_eq!(status.days_left, Some(2));
        assert!(status.trial_ends_at.is_some());
        assert_eq!(status.plan, Some(LicensePlan::Trial));

        let after = gate_with(&account, &signer, offline_server());
        let status = after.refresh(ends_at + 1).await;
        assert_eq!(status.state, LicenseState::TrialExpired);
        assert!(!status.authorized);
    }

    #[tokio::test]
    async fn a_rolled_back_clock_denies_until_time_catches_up() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        gate_with(&account, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;
        // iat was NOW-60; a clock 11 minutes before it is rolled back.
        let rolled = NOW - 60 - 11 * 60;
        let gate = gate_with(&account, &signer, offline_server());
        let status = gate.refresh(rolled).await;
        assert_eq!(status.state, LicenseState::ClockRolledBack);
        assert!(!status.authorized);
        assert!(gate.reverify(rolled).is_err());
    }

    #[tokio::test]
    async fn a_token_for_another_device_or_nonce_never_authorizes() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        for wrong in ["device", "nonce"] {
            let server = {
                let signer = signer.clone();
                FakeServer::new(move |request| {
                    let claims = if wrong == "device" {
                        licensed_claims(&request.challenge, "other-mac")
                    } else {
                        licensed_claims("replayed", THUMB)
                    };
                    Ok(signer.sign(&claims))
                })
            };
            let gate = gate_with(&account, &signer, server);
            let status = gate.refresh(NOW).await;
            assert!(!status.authorized, "{wrong}");
            assert_eq!(status.state, LicenseState::Unverified);
            assert!(gate.reverify(NOW).is_err());
        }
    }

    #[tokio::test]
    async fn every_non_verified_outcome_is_unauthorized() {
        let signer = Arc::new(TestSigner::generate("k1"));
        let other = Arc::new(TestSigner::generate("k1"));

        let signed_out = TestAccount::signed_in(THUMB);
        signed_out.state().session = SessionResult::SignedOut;
        let status = gate_with(&signed_out, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;
        assert_eq!(status.state, LicenseState::SignInRequired);
        assert!(!status.authorized);

        let failed = TestAccount::signed_in(THUMB);
        failed.state().session = SessionResult::Failed("keychain".to_string());
        assert!(
            !gate_with(&failed, &signer, offline_server())
                .evaluate(NOW)
                .authorized
        );

        let tampered = TestAccount::signed_in(THUMB);
        tampered.state().integrity = IntegrityResult::Tampered("patched".to_string());
        assert!(
            !gate_with(&tampered, &signer, licensed_server(signer.clone()))
                .refresh(NOW)
                .await
                .authorized
        );

        let no_device = TestAccount::signed_in(THUMB);
        no_device.state().fail_device_sign = true;
        assert!(
            !gate_with(&no_device, &signer, licensed_server(signer.clone()))
                .refresh(NOW)
                .await
                .authorized
        );

        let account = TestAccount::signed_in(THUMB);
        assert!(
            !gate_with(&account, &signer, offline_server())
                .refresh(NOW)
                .await
                .authorized
        );
        for reply in [
            Ok("garbage".to_string()),
            Ok(other.sign(&licensed_claims("x", THUMB))),
            Err(FetchError::Refused("403".to_string())),
        ] {
            let server = FakeServer::new(move |_| reply.clone());
            let gate = gate_with(&account, &signer, server);
            assert!(!gate.refresh(NOW).await.authorized);
            assert!(gate.reverify(NOW).is_err());
        }
    }

    #[tokio::test]
    async fn a_server_refusal_discards_the_stored_entitlement() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        gate_with(&account, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;
        let refusing = FakeServer::new(|_| Err(FetchError::Refused("403".to_string())));
        let gate = gate_with(&account, &signer, refusing);
        assert!(!gate.refresh(NOW).await.authorized);
        assert!(
            !gate_with(&account, &signer, offline_server())
                .evaluate(NOW)
                .authorized
        );
    }

    #[tokio::test]
    async fn setting_the_clock_back_after_the_last_iat_denies_with_the_server_unreachable() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        gate_with(&account, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;
        let iat = NOW - 60;
        // The app keeps running for two days; the mark follows the wall clock, not the token.
        let later = NOW + 2 * 24 * 3600;
        assert!(
            gate_with(&account, &signer, offline_server())
                .evaluate(later)
                .authorized
        );

        for (label, now, expect) in [
            ("just after iat", iat + 1, false),
            ("limit + 1", later - 601, false),
            ("limit", later - 600, true),
            ("limit - 1", later - 599, true),
            ("forward", later + 3600, true),
        ] {
            let gate = gate_with(&account, &signer, offline_server());
            let status = gate.refresh(now).await;
            assert_eq!(status.authorized, expect, "{label}: {}", status.message);
            if !expect {
                assert_eq!(status.state, LicenseState::ClockRolledBack, "{label}");
                assert!(gate.reverify(now).is_err(), "{label}");
            }
        }
    }

    #[tokio::test]
    async fn a_setback_inside_the_tolerance_cannot_extend_an_expired_grant() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        gate_with(&account, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;
        let exp = NOW - 60 + 3 * 24 * 3600;
        let expired = gate_with(&account, &signer, offline_server());
        assert!(!expired.evaluate(exp + 300).authorized);
        let moved_back = gate_with(&account, &signer, offline_server());
        assert!(!moved_back.evaluate(exp - 240).authorized);
    }

    #[tokio::test]
    async fn a_correct_online_refresh_recovers_from_a_clock_that_ran_ahead() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        gate_with(&account, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;
        gate_with(&account, &signer, offline_server()).evaluate(NOW + 86_400);
        let gate = gate_with(&account, &signer, licensed_server(signer.clone()));
        assert!(!gate.evaluate(NOW).authorized);
        assert!(gate.refresh(NOW).await.authorized);
        assert!(
            gate_with(&account, &signer, offline_server())
                .evaluate(NOW)
                .authorized
        );
    }

    #[tokio::test]
    async fn a_token_for_another_account_never_authorizes() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        gate_with(&account, &signer, licensed_server(signer.clone()))
            .refresh(NOW)
            .await;
        if let SessionResult::SignedIn(session) = &mut account.state().session {
            session.user_id = "user-2".to_string();
        }
        let status = gate_with(&account, &signer, offline_server()).evaluate(NOW);
        assert!(!status.authorized);
        let fresh = gate_with(&account, &signer, licensed_server(signer.clone()));
        assert!(!fresh.refresh(NOW).await.authorized);
    }

    #[tokio::test]
    async fn device_limit_is_typed_and_revocation_needs_explicit_confirmation() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        let server = {
            let signer = signer.clone();
            FakeServer::new(move |request| {
                if request.revoke_oldest_device == Some(true) {
                    Ok(signer.sign(&licensed_claims(&request.challenge, THUMB)))
                } else {
                    Err(FetchError::DeviceLimit)
                }
            })
        };
        let gate = gate_with(&account, &signer, server);
        let status = gate.refresh(NOW).await;
        assert_eq!(status.state, LicenseState::DeviceLimitReached);
        assert!(!status.authorized);
        let status = gate.refresh_with(NOW, true).await;
        assert!(status.authorized, "{}", status.message);
    }

    #[tokio::test]
    async fn refresh_window_opens_under_24_hours() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        let gate = gate_with(&account, &signer, licensed_server(signer.clone()));
        assert!(gate.needs_refresh(NOW), "nothing stored");
        gate.refresh(NOW).await;
        assert!(!gate.needs_refresh(NOW));
        let exp = NOW - 60 + 3 * 24 * 3600;
        assert!(!gate.needs_refresh(exp - REFRESH_WINDOW_SECONDS));
        assert!(gate.needs_refresh(exp - REFRESH_WINDOW_SECONDS + 1));
    }

    #[tokio::test]
    async fn sign_out_removes_authorization_and_delete_reports_each_part() {
        let account = TestAccount::signed_in(THUMB);
        let signer = Arc::new(TestSigner::generate("k1"));
        let gate = gate_with(&account, &signer, licensed_server(signer.clone()));
        gate.refresh(NOW).await;
        gate.sign_out().unwrap();
        assert!(gate.reverify(NOW).is_err());
        assert_eq!(gate.evaluate(NOW).state, LicenseState::SignInRequired);

        let account = TestAccount::signed_in(THUMB);
        let server = licensed_server(signer.clone());
        *server.delete.lock().unwrap() = Err(FetchError::Unreachable("offline".to_string()));
        *server.watch.lock().unwrap() = Some(account.clone());
        let gate = gate_with(&account, &signer, server.clone());
        let report = gate.delete_account().await;
        assert!(report.local_cleared && report.signed_out);
        assert!(!report.remote_deleted);
        assert_eq!(report.not_done.len(), 1);
        assert_eq!(account.state().signed_out, 1);
        assert_eq!(
            *server.signed_out_at_delete.lock().unwrap(),
            Some(0),
            "the server delete must run before native sign-out"
        );
    }

    #[tokio::test]
    async fn checkout_requires_a_signed_in_session() {
        let signer = Arc::new(TestSigner::generate("k1"));
        let account = TestAccount::signed_in(THUMB);
        let gate = gate_with(&account, &signer, offline_server());
        assert!(gate.start_checkout(Some(3)).await.is_ok());
        account.state().session = SessionResult::SignedOut;
        assert!(gate.start_checkout(None).await.is_err());
    }

    #[test]
    fn persisted_clock_guard_that_cannot_be_read_denies() {
        let account = TestAccount::signed_in(THUMB);
        account
            .state()
            .store
            .insert(CLOCK_KEY.to_string(), "not-a-number".to_string());
        let signer = TestSigner::generate("k1");
        let gate = gate_with(&account, &signer, offline_server());
        assert!(!gate.evaluate(NOW).authorized);
        assert!(gate.reverify(NOW).is_err());
    }
}
