# Release, Mac App Store publication, and rollback

This runbook distinguishes build evidence from a distributable artifact. The ordinary CI bundles
are intentionally unsigned, and the Windows and Linux bundles CI produces are compile evidence
only: the account, license, and update paths are macOS-native, so macOS is the only distributed
platform. Two macOS builds ship. The paid Mac App Store build is sandboxed, provisioned for
`com.onrecord.studio`, signed with Apple distribution identities, and submitted as a signed
installer package (this document). The Developer ID build is notarized and self-updating
([release-direct.md](release-direct.md)).

## Distribution contract

- Product and bundle identifier: `OnRecord Studio` / `com.onrecord.studio`.
- Current version: `1.0.0-beta.3` in `package.json`, `src-tauri/Cargo.toml`, and
  `src-tauri/tauri.conf.json`. The pre-release suffix is for the direct channel; the App Store
  build needs a numeric version, so drop the suffix in all three files before an App Store
  submission (`prepare-app-store.mjs --check` requires them to agree).
- Development and ordinary desktop builds use the default Rust feature set. They identify their
  license state as `DEVELOPMENT` and do not pretend to be paid App Store artifacts.
- Paid macOS builds use the `app-store` Cargo feature and
  `src-tauri/tauri.appstore.conf.json`. Their minimum system version is macOS 13 because that is the
  first macOS release with StoreKit `AppTransaction`.
- App Store builds use App Sandbox, client networking, and user-selected read/write file access.
  They do not request incoming-network, unsigned-executable-memory, or disabled-library-validation
  entitlements.
- Runtime data is stored below Tauri's resolved application-data container, including
  `onrecord_studio_core.db`, `onrecord_studio_corpus.db`, `documents/`, and `reports/`.
- The bundle holds one executable. Outlook PST files are read by a child process that is that same
  signed executable started with the hidden first argument `--onrecord-expand-pst`; `main` handles
  it before Tauri, StoreKit, or the license gate start. Nothing extra is signed, embedded, or
  given `com.apple.security.inherit`. The child gets the same entitlements and App Sandbox
  container as the app, and touches only the managed copy under `documents/` and a staging
  directory beside it. The mechanism was checked with a minimal C probe, not the OnRecord build: an
  ad-hoc-signed bundle with only the sandbox entitlement that started its own executable again.
  The child was sandboxed, shared the parent's container, wrote inside it, and was refused outside
  it. Repeat the check on the provisioned build by importing a PST in the sandboxed smoke test.

## Paid-license enforcement

The App Store build asks StoreKit for Apple's cryptographically verified `AppTransaction`. It
authorizes the workspace only when StoreKit reports a verified, unrevoked purchase for exactly
`com.onrecord.studio`. Unknown results, malformed payloads, bundle mismatches, revocations, and
verification failures fail closed.

The decision is enforced twice: Swift validates the Apple transaction and Rust independently
checks the returned bundle identifier. A global Tauri invoke handler (`licensing::command_is_unlicensed`
in `src-tauri/src/licensing.rs`) rejects every application command while unauthorized except
`get_license_status`, the user-initiated `refresh_license`, and the direct-build account commands
(`get_account_status`, `begin_sign_in`, `sign_out`, `delete_account`, `start_checkout`,
`activate_this_device`). In an App Store build those account commands are registered but inert:
without the `direct-license` feature each one returns "Accounts are not part of this build." The
React lock screen is therefore presentation, not the security boundary.

`AppTransaction.shared` may require App Store authentication or network access. The automatic
launch check is bounded to 20 seconds so StoreKit cannot hold application startup forever. The
explicit refresh action is unbounded because Apple may show a system authentication prompt and the
user must be allowed to finish it. An unavailable result does not create an offline bypass.

This prevents ordinary unpaid use and casual copying. It cannot make piracy impossible: a capable
attacker who controls a Mac can patch any entirely local executable. A future server could validate
the AppTransaction JWS and issue short-lived capability tokens, but that only adds material
protection if indispensable functionality is server-controlled; it would also add availability and
privacy costs. No custom license server is part of this release.

Apple references:

- [AppTransaction](https://developer.apple.com/documentation/storekit/apptransaction)
- [AppTransaction.shared](https://developer.apple.com/documentation/storekit/apptransaction/shared)
- [AppTransaction.refresh()](https://developer.apple.com/documentation/storekit/apptransaction/4020517-refresh)
- [App pricing and availability](https://developer.apple.com/help/app-store-connect/reference/pricing-and-availability/app-pricing-and-availability)

## Source and dependency release gate

Run from the application directory (`onrecord-studio/`):

```sh
npm ci
npm run check:ipc
npm run check:app-store-config
npx tsc --noEmit
npm run lint
npm run test:unit
npm run build
cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo clippy --workspace --all-targets --features app-store --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --workspace --manifest-path src-tauri/Cargo.toml
cargo audit
npm audit --audit-level=high
```

The candidate fails if any command fails. `npm run build` repeats the IPC contract check so a
generated/native protocol drift cannot be hidden by running only the frontend compiler.

Before tagging, also verify:

1. The three version declarations are identical.
2. `git status --short` is empty and the intended commit is checked out.
3. The release notes identify schema changes and changes to supported source or document formats.
4. The exact candidate can add a source, ingest a small fixture, search the resulting record, open
   its evidence, and export a report while sandboxed.

## Apple prerequisites

The Account Holder must accept the current Paid Apps Agreement and set a non-free app price in App
Store Connect. In Certificates, Identifiers & Profiles, create or select:

1. The explicit App ID `com.onrecord.studio`.
2. A `Mac App Store Connect` distribution provisioning profile for that App ID.
3. An Apple Distribution application-signing identity linked to the profile.
4. A Mac Installer Distribution identity for the final `.pkg`.

Confirm locally without exposing private keys:

```sh
security find-identity -p codesigning -v
security find-identity -p basic -v
```

Profiles for other bundle identifiers are invalid even if they belong to the same team. Do not put
provisioning profiles, generated entitlements, API keys, or certificates in Git.

## Materialize App Store configuration

Review Apple's export-compliance questionnaire before setting the compliance value. Then provide:

```sh
export APPLE_TEAM_ID="YOUR10CHARTEAM"
export APPLE_APP_ID_PREFIX="YOUR10CHARPREFIX"
export APPLE_PROVISIONING_PROFILE_PATH="/absolute/path/OnRecord.provisionprofile"
export ONRECORD_EXPORT_COMPLIANCE="exempt"
# Set APPLE_EXPORT_COMPLIANCE_CODE only when Apple issued one for this build.
npm run prepare:app-store
```

The script validates formats, refuses symlinked or empty profiles, and XML-escapes the optional
export code. It writes the materialized entitlements and `Info.plist` as mode 600; the copied
provisioning profile is deliberately made world-readable (mode 644) afterward, because Tauri embeds
it in the bundle and App Store Connect rejects any package file a non-root user cannot read — the
profile carries certificates and entitlements, no secret. It does not log credential values. It also
decodes the profile and refuses to materialize one whose `com.apple.application-identifier` is not
exactly `$APPLE_APP_ID_PREFIX.com.onrecord.studio`, whose team entitlement is not `$APPLE_TEAM_ID`,
or whose `ExpirationDate` has passed, so a wrong or stale profile fails before the build rather than
at App Store Connect upload.

## Build, validate, and package

Select the Apple Distribution identity for application signing and build the App Store variant:

```sh
export APPLE_SIGNING_IDENTITY="Apple Distribution: Your Name (YOURTEAMID)"
npm run build:app-store
```

Validate the exact signed `.app`. Set `ONRECORD_APP_BUNDLE_PATH` only if Cargo's target directory is
not the default discovered by the script:

```sh
npm run validate:app-store-bundle
```

Validation fails on an ad-hoc signature, missing/mismatched provisioning profile, wrong bundle or
team identifier, missing sandbox entitlement, prohibited entitlement, non-system dynamic library,
invalid nested signature, or frontend source-map reference.

Create the signed installer:

```sh
export APPLE_INSTALLER_SIGNING_IDENTITY="3rd Party Mac Developer Installer: Your Name (YOURTEAMID)"
export ONRECORD_APP_STORE_PKG_PATH="/absolute/output/OnRecord Studio.pkg"
npm run package:app-store
```

The packaging script validates the `.app` again, runs `productbuild`, and requires `pkgutil` to
report the resulting package as signed by this team's `3rd Party Mac Developer Installer`
certificate, chaining to the Apple Worldwide Developer Relations and Apple Root certificate
authorities. That chain is accepted by App Store Connect but is deliberately not the
macOS-trusted Developer ID Installer chain used for direct installation; `pkgutil` reports it as
"signed by a developer certificate issued by Apple (Development)" rather than as notarized. Follow
Tauri's current
[Mac App Store guide](https://tauri.app/distribute/app-store/) if Apple's certificate names change.

## Upload and App Store verification

Use an App Store Connect API key with the narrowest suitable role. Do not add the `.p8` file to the
repository. Upload the exact verified package:

```sh
xcrun altool --upload-app --type macos \
  --file "$ONRECORD_APP_STORE_PKG_PATH" \
  --apiKey "$APPLE_API_KEY_ID" \
  --apiIssuer "$APPLE_API_ISSUER"
```

An upload is not a release. Wait for Apple's processing, install the TestFlight build, and repeat
the sandboxed smoke test against that Apple-processed artifact. Record the source commit, package
SHA-256, application and installer certificate fingerprints, upload result, processing result, and
smoke-test result without recording private credentials.

## Rollout and rollback

1. Back up the complete application-data container while OnRecord Studio is closed. The databases
   and retained evidence files are one logical dataset.
2. Test the Apple-processed candidate on a non-production macOS account before broad availability.
3. Confirm startup migration, source resume, search, evidence opening, export, license refresh, and
   refunded/revoked-purchase behavior.
4. If rollback is required, stop the app, preserve the failed release's data for diagnosis, install
   the last verified App Store version, and restore the pre-upgrade data container as a unit when a
   newer schema was opened. Do not mix old databases with newer `documents/` or corpus files.

The legacy SQLite-to-DuckDB migration retains its source as a sibling backup, but that narrow backup
is not a substitute for the full pre-release backup.

## Support evidence

For a release failure, record the version, macOS version, AppTransaction state (without transaction
identifiers), source kind, time range, visible structured error, and relevant Activity entries. Do
not attach confidential source documents unless the recipient is authorized to receive them.
