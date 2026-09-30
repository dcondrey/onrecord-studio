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

Reuse the WritersLogic Supabase project (`auth.writerslogic.com`): email, Google, Apple, LinkedIn, GitHub. The client is public and holds no secret; Supabase holds the provider secrets, which is why Apple works.

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

Out of scope: defeating a reverse engineer who patches every check. The value of this app is also in updates and server-side entitlement, which a cracked copy does not get.

## Non-goals

- No portal credential storage or authenticated portal fetching; linked staff accounts can expose non-public records. Requesters claim requests by number and import notification emails instead.
- Affiliation never changes a finding, confidence, or statutory result.

## Direct build operations

- Signing: Developer ID Application certificate and a Developer ID provisioning profile for `com.onrecord.studio`, embedded so keychain access groups and the App Attest opt-in entitlement are honored.
- Notarization and stapling are part of the release script; a build that is not notarized is not released.
- Updates: signed updater (key kept offline), never an unsigned download. Version and entitlement are independent: an update does not extend a trial or license.
- Payment: Stripe product for OnRecord Studio in the WritersLogic Stripe account; the webhook maps the price id to the entitlement plan. Refund and revocation set `revoked` server-side and the next refresh denies.
- App Attest (`app-attest-opt-in`, `CDhash`, macOS 27 and later) is an optional extra signal for the direct build only, never a gate.

## Status (2026-09-30, updated)

Deployed to the WritersLogic Supabase project (ref aswcfxodrgcnjbwrcjrl): migrations `20260930120000` and `20260930130000`, secrets (entitlement signing key id `or-2026-1`, peppers, `ONRECORD_STRIPE_PRICE_IDS` = the live price, `TRIAL_DAYS=30`), functions `onrecord-entitlement`, `onrecord-verify-work-email`, `onrecord-checkout`, and the edited `stripe-webhook`. Deployed functions answer 401 to an unauthenticated call. The entitlement public key is pinned by kid in `src-tauri/src/licensing` (verify it is there before release). The signing private key is backed up only at `~/.config/onrecord-studio/signing/entitlement-signing.pem`; keep a copy in a password manager.

Local build inputs live in the git-ignored `.env.direct.local`: anon key, `APPLE_SIGNING_IDENTITY` (Developer ID Application, pinned by SHA-1), Developer ID profile path (`~/.config/onrecord-studio/provisioning/current/`), notary profile name `onrecord-notary` (the credential itself must be created with `xcrun notarytool store-credentials`, which needs an app-specific password).

Built and gated: server functions and webhook mapping (31+ Deno tests), Swift account bridge (46 harness checks), direct-build config, prepare and notarize scripts, `build:direct` wrapper, website `/onrecord` page (unit tests, not viewed in a browser). In progress at time of writing: Rust `licensing/` gate, sign-in and license UI, local profile and affiliation lens, PII and profile test fixes.

Not verified end to end: any real sign-in, Keychain and Secure Enclave use, `build:direct` and notarization, a live Stripe purchase, browser rendering of the website page.

Open work: seat assignment for organization purchases (extra seats are billed but only the purchaser is licensed until a seat table and invite flow exist); signed updater; `SUPPORT_EMAIL` and `DOWNLOAD_URL` in the website config; scheduled deletion of trial-device rows older than 24 months and a privacy-policy disclosure; mounting the deadline-appeal and report-export panels; final full gate (fmt, clippy, tests, tsc, eslint, IPC check, vitest); Team ID stays `U3PZN7P3E5` (personal account) until the developer account is converted to an organization.

## v1 release checklist (2026-09-30)

Code streams: request tracker and `.eml` import (in progress, `parsers/eml.rs` currently duplicates helpers); visual QA of new screens (in progress); claim-versus-data detector (ab, in progress); work-email verification screen; organization seat assignment (server and app); signed updater and release packaging (DMG, notarization, update feed); in-app help documentation (inventory, topics, screenshots, contract check); security review of licensing, sign-in and server functions; final full gate (fmt, clippy with and without `direct-license`, tests, `tsc`, `eslint`, IPC check, vitest, contrast, direct-license link build).

Only the owner can do: notary credentials (`xcrun notarytool store-credentials`), `SUPPORT_EMAIL` and `DOWNLOAD_URL` in the website config, Stripe coupons and the ten webhook events on the live endpoint, privacy policy and terms text, moving the entitlement signing key backup to a password manager, hosting the release artifacts, and authorizing a commit, tag and release (nothing is committed).

## Release state at 2026-09-30 (continuation notes)

Deployed and committed (writerslogic.com repo, local commits not pushed; deploy there is tag-triggered, the site was deployed by hand with wrangler): `/onrecord` page live at https://writerslogic.com/onrecord (support@writerslogic.com, DOWNLOAD_URL empty until a signed build ships); migrations through `20260930160000_onrecord_revocations.sql`; functions onrecord-entitlement, onrecord-verify-work-email, onrecord-checkout, onrecord-delete-account, onrecord-seats, stripe-webhook. Push the writerslogic.com commits to main so the next tagged deploy keeps the page.

Security review (no criticals) fixes: server revocations done and deployed; client fixes in progress (device_sign outcome literal "signed" vs "ok" release blocker, wall-clock high-water mark for clock rollback, delete-account POST and order, token sub vs session user, device-limit confirm flow with `activate_this_device`). After they land: add the device-limit confirm UI to the license gate, re-run the full gate, commit.

Main repo: a full gate was started (fmt, clippy, cargo test --lib, tsc, eslint, IPC check, vitest) with results in /tmp/g5_*.txt; commit a checkpoint when green, including session ab's claim-validation files (analysis/claim_validation.rs, detectors/claim_data_mismatch.rs, docs/ROADMAP.md, docs/analysis_methods.md) and the finding.rs, findings.ts, detectors/mod.rs, artifacts, generated contracts, DocumentViewer work. Nothing in the main repo is committed yet from this session's streams; ab committed its own two commits (3b11bca, abdb0716).

In progress streams: request tracker and `.eml` import, work-email verification screen, updater and DMG packaging (docs/release-direct.md), help documentation inventory (scratchpad help/), licensing security fixes. Still to do afterwards: help documentation topics and screenshots with a contract check, small QA leftovers (SelfReview/useProfile error prefix, inline sign-out link size, Save profile contrast 4.41:1, PostIngestBrief light remap), `cargo build --features direct-license` link test, Cloudflare updates feed (existing worker workers/updates in writerslogic.com serves R2 bucket writerslogic-releases; add an OnRecord route and latest.json), final gate and release commit. Wrangler is logged in (account WritersLogic); the Cloudflare work is authorized.

Owner-only remaining: notary credentials, Stripe coupons and the two ACH events plus eight others on the live webhook, privacy policy and terms, backup of the entitlement signing key, hosting artifacts, authorizing push, tag and release.
