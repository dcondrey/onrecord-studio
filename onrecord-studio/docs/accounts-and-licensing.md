# Accounts, trial, and licensing

Settled design. Change it here first, then in code.

## Decision

Direct distribution (Developer ID, notarized) is the primary and launch channel. It is the only one that allows a server-anchored trial and a login gate, costs Stripe fees instead of Apple's commission, lets agencies and universities pay by invoice or purchase order, avoids App Review of forensic features, and needs no sandbox. The Mac App Store path (StoreKit) stays in the code, tested but not the focus; revisit it only if store discovery becomes worth the constraints below.

## Channels

| Channel | Gate | Account | Why |
|---|---|---|---|
| Mac App Store build (`app-store` feature) | StoreKit `AppTransaction`, fail-closed (`licensing.rs`, `native/AppLicense.swift`) | Optional, never required | Guidelines 2.4.5(vi) and 3.1.1 forbid a launch license screen or own unlock mechanism; requiring an account for a local tool risks 5.1.1 |
| **Direct build, Developer ID, notarized (`direct-license` feature), primary** | Server-signed entitlement, verified on device | Required once, then offline grace | A trial must be anchored server-side or it resets by deleting files |
| Development and tests | Open | None | Unchanged |
| TestFlight | Apple enforces the 90-day build expiry | None | No code needed for a few testers |

## Identity

Reuse the WritersLogic Supabase project (`auth.writerslogic.com`): email, Google, Apple, LinkedIn, GitHub are the backend-recognized providers (`PROVIDERS` in `licensing/account.rs`). The client is public and holds no secret; Supabase holds the provider secrets, which is why Apple works. The current sign-in screen (`LicenseGate.tsx`) only renders buttons for Google, Apple, LinkedIn, and GitHub; email is accepted by the backend but not yet wired into that UI.

- Flow: `ASWebAuthenticationSession`, PKCE, callback `com.onrecord.studio://auth/callback`. No loopback listener, so no `network.server`.
- Session and device key live in the data-protection Keychain. Nothing else is stored.
- Sign-out clears the Keychain. "Delete account" calls a server function and clears local state.
- Affiliation is **self-declared** (`SELF_DECLARED`). A work email confirmed by a one-time code to that mailbox upgrades it to `EMAIL_DOMAIN_VERIFIED`, which proves control of a mailbox, not employment. Provider `hd`, `tid` or `email` claims are never used as organization proof.

## Entitlement token

Compact JWS, `EdDSA`, `kid` header. Public key pinned in the app; rotation adds a second `kid`.

| Claim | Meaning |
|---|---|
| `iss` | `auth.writerslogic.com/onrecord` |
| `aud` | `com.onrecord.studio` |
| `sub` | Supabase user id |
| `plan` | `trial` or `licensed` |
| `trial_started_at`, `trial_ends_at` | Server-recorded, RFC 3339 UTC. The trial starts once per account, at first entitlement request |
| `dev` | Base64url SHA-256 of the device public key (Secure Enclave, P-256 X9.63) |
| `nonce` | Echo of the client challenge; a replayed token is rejected |
| `iat`, `exp` | `exp - iat` is at most 7 days |

The device proves possession by signing the challenge with its key. A token copied to another Mac fails the `dev` check.

## Client rules

- Verify signature, `iss`, `aud`, `nonce`, `dev`, `iat <= now + skew`, `now < exp`.
- Keep `max_seen_time` (largest `iat` seen) in the Keychain. If the system clock is more than 10 minutes before it, deny with `CLOCK_ROLLED_BACK` until an online refresh.
- Expired trial, expired token with no network, or a failed check all deny. Only local evidence stays readable; every workspace command returns a license error. Failing closed is the default state, not an exception.
- Refresh in the background when under 24 hours remain and the network is available.
- Every command passes `LicenseGate::require_authorized`; the gate result is one input among several checks, not a single boolean (see hardening).

## Hardening, and its limits

No client-side scheme is uncrackable. A determined attacker who controls the machine can patch the binary. The goal is to make casual bypass fail and remove every cheap route:

| Cheap route | Defense |
|---|---|
| Reset the trial by deleting files, reinstalling, or changing the clock | Trial start is server-side per account; clock rollback is detected |
| Copy a license or token to another Mac | Device-bound `dev` claim plus challenge nonce |
| Forge a token | Ed25519, pinned key, server-held signing key |
| Edit a saved status | Nothing trusted is saved; the token is re-verified at each launch and refresh |
| Patch one boolean | The gate is checked in several independent places, including at command dispatch and again before evidence is read for analysis |
| Inject code, attach a debugger | Hardened runtime, library validation, `PT_DENY_ATTACH` in release, runtime code-signature self-check |
| Share one account across many people | Account-bound, device-bound; the server limits active devices per account |
| Reuse one account on many Macs, or many macOS users on one Mac | The device key is per macOS user, not per physical machine: a second user on the same Mac holds a second key and counts as a second device. The device count itself is enforced server-side; the client only recognizes the server's HTTP 409 `device_limit` response (`client.rs`) and offers to revoke the oldest device, so the exact number of allowed devices is not a value in this codebase and is unverifiable here. App Attest is the planned machine-level improvement |
| Run a modified build that reports integrity `unknown` | Accepted risk: `unknown` (for example an unsigned or development build) still runs, because denying it would brick legitimate debug and repackaged-for-support builds. Only `tampered` denies |

Out of scope: defeating a reverse engineer who patches every check. The value of this app is also in updates and server-side entitlement, which a cracked copy does not get.

## Non-goals

- No portal credential storage or authenticated portal fetching; linked staff accounts can expose non-public records. Requesters claim requests by number and import notification emails instead.
- Affiliation never changes a finding, confidence, or statutory result.

## Direct build operations

- Signing: Developer ID Application certificate and a Developer ID provisioning profile for `com.onrecord.studio`, embedded so the `keychain-access-groups` entitlement in `src-tauri/Entitlements.direct.plist` is honored (plus `network.client` and `files.user-selected.read-write`; no App Attest entitlement exists yet, see below).
- Notarization and stapling are part of the release script; a build that is not notarized is not released.
- Updates: signed updater (key kept offline), never an unsigned download. Version and entitlement are independent: an update does not extend a trial or license.
- Payment: Stripe product for OnRecord Studio in the WritersLogic Stripe account; the webhook maps the price id to the entitlement plan. Refund and revocation set `revoked` server-side and the next refresh denies. Display pricing is a client-side constant, `src/config/pricing.ts` ("the only place price facts live"): $79/seat/year for 1 seat, $69/seat/year for 2-9 seats, $49/seat/year for 10 or more seats, billed yearly, 1-999 seats. `start_checkout` passes the chosen seat count as `quantity`, capped at `MAX_CHECKOUT_QUANTITY = 999` in `client.rs`; Stripe is still the source of truth for what is actually charged.
- App Attest: not implemented. No `app-attest-opt-in` feature, entitlement, or `CDhash` check exists anywhere in `src-tauri/` or the native bridge; it remains the planned machine-level improvement noted above, never a gate when it ships.

## Status (2026-09-30, updated)

Deployed to the WritersLogic Supabase project (ref aswcfxodrgcnjbwrcjrl): migrations `20260930120000` and `20260930130000`, secrets (entitlement signing key id `or-2026-1`, peppers, `ONRECORD_STRIPE_PRICE_IDS` = the live price, `TRIAL_DAYS=30`), functions `onrecord-entitlement`, `onrecord-verify-work-email`, `onrecord-checkout`, and the edited `stripe-webhook`. These are server secrets with no client-side counterpart; the trial length is not a constant in this repository and cannot be verified here (the 14-day figure in `entitlement.rs`'s `test_support::trial_claims` is only a test fixture, not the deployed value). Deployed functions answer 401 to an unauthenticated call. The entitlement public key is pinned by kid (`or-2026-1`) in `src-tauri/src/licensing/entitlement.rs`'s `ENTITLEMENT_PUBLIC_KEYS`; an empty list there denies every token by design, so confirm the key is still present before release. The signing private key is backed up only at `~/.config/onrecord-studio/signing/entitlement-signing.pem`; keep a copy in a password manager.

Local build inputs live in the git-ignored `.env.direct.local`: anon key, `APPLE_SIGNING_IDENTITY` (Developer ID Application, pinned by SHA-1), Developer ID profile path (`~/.config/onrecord-studio/provisioning/current/`), notary profile name `onrecord-notary` (the credential itself must be created with `xcrun notarytool store-credentials`, which needs an app-specific password).

Built and gated: server functions and webhook mapping (31+ Deno tests), Swift account bridge (46 harness checks), direct-build config, prepare and notarize scripts, `build:direct` wrapper, website `/onrecord` page (unit tests, not viewed in a browser). In progress at time of writing: Rust `licensing/` gate, sign-in and license UI, local profile and affiliation lens, PII and profile test fixes.

Not verified end to end: any real sign-in, Keychain and Secure Enclave use, `build:direct` and notarization, a live Stripe purchase, browser rendering of the website page.

Open work: seat assignment for organization purchases (extra seats are billed but only the purchaser is licensed until a seat table and invite flow exist); signed updater; `SUPPORT_EMAIL` and `DOWNLOAD_URL` in the website config; scheduled deletion of trial-device rows older than 24 months and a privacy-policy disclosure; mounting the deadline-appeal and report-export panels; final full gate (fmt, clippy, tests, tsc, eslint, IPC check, vitest); Team ID stays `U3PZN7P3E5` (personal account) until the developer account is converted to an organization.

## v1 release checklist (2026-09-30)

Code streams: request tracker and `.eml` import (in progress, `parsers/eml.rs` currently duplicates helpers); visual QA of new screens (in progress); claim-versus-data detector (ab, in progress); work-email verification screen; organization seat assignment (server and app); signed updater and release packaging (DMG, notarization, update feed); in-app help documentation (inventory, topics, screenshots, contract check); security review of licensing, sign-in and server functions; final full gate (fmt, clippy with and without `direct-license`, tests, `tsc`, `eslint`, IPC check, vitest, contrast, direct-license link build).

Only the owner can do: notary credentials (`xcrun notarytool store-credentials`), `SUPPORT_EMAIL` and `DOWNLOAD_URL` in the website config, Stripe coupons and the ten webhook events on the live endpoint, privacy policy and terms text, moving the entitlement signing key backup to a password manager, hosting the release artifacts, and authorizing a commit, tag and release (nothing is committed).

## Release state at 2026-09-30 (end of day)

Committed in the main repo (checkpoint 127be59 plus help and updater commits). Gates at that point: fmt, clippy (default and `direct-license`), tsc, eslint, vitest (431), IPC and help-coverage checks green; `cargo build --features direct-license` links; `build-direct.mjs --check` passes. `cargo test --lib`: 924 pass, 3 fail, all in the claim-validation stream (`ClaimContradictsData` follow-up template x2, ambiguous-scope claim check).

Server: migrations through `20260930160000`, functions onrecord-entitlement, onrecord-verify-work-email, onrecord-checkout, onrecord-delete-account, onrecord-seats, stripe-webhook deployed. Updates: worker `updates-writerslogic` serves `updates.writerslogic.com/onrecord/updates/<target>/<arch>/<version>` from R2 key `onrecord/latest.json` (204 until published). Site `/onrecord` deployed; site commits local, not pushed.

Product decisions: AI research is off on a fresh install (opt-in), documents and analysis stay on the Mac; the account and trial record, and a work email when verified, go to the WritersLogic service.

Owner-only remaining: notary credentials (`xcrun notarytool store-credentials onrecord-notary`), Stripe coupons and the ten webhook events on the live endpoint, privacy policy and terms, backup of the entitlement signing key and `updater.key` (+ password), run `build:direct` and upload artifacts per `release-direct.md`, set `DOWNLOAD_URL`, push site commits, authorize push, tag and release.

Unverified end to end: real sign-in, Keychain and Secure Enclave, a live Stripe purchase, notarization, an update against a lower-versioned build, the new screens in a browser.

## Billing portal and plan intervals (2026-10-01)

Cancellation uses Stripe's Customer Portal through the shared `stripe-portal` function
(`https://auth.writerslogic.com/functions/v1/stripe-portal`, deployed from the website repo). The
app's `start_billing_portal` command sends the signed-in session and a fixed return URL and opens
only a `https://billing.stripe.com` address. No new server code is needed: the webhook already
keeps a subscription `active` until Stripe sends `customer.subscription.deleted` at the period end,
so a portal cancellation (`cancel_at_period_end`) leaves the license valid until then and is not
refunded; the next entitlement refresh after that denies. The portal's own configuration in the
Stripe dashboard must allow cancellation and seat (quantity) changes.

A yearly price is a Stripe step, not an app step: create the price on the existing product, add
its id to the `ONRECORD_STRIPE_PRICE_IDS` secret (comma-separated; `onrecord-checkout` accepts any
listed `priceId` and `stripe-webhook` grants on any of them), then teach the app to send
`priceId` for the chosen interval and show both prices in `src/config/pricing.ts`. Until a second
price is listed, checkout omits `priceId` and the single live price is used.
