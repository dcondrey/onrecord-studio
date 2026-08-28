# Release and rollback runbook

This runbook describes the release process implemented by this repository. It does not assert that
signing credentials exist: `.github/workflows/ci.yml` intentionally produces unsigned bundles, and
`src-tauri/tauri.conf.json` currently has no macOS signing identity or entitlements configured.
Those CI artifacts are build evidence, not public release artifacts.

## Supported build contract

- Product and bundle identifier: `OnRecord Studio` / `com.onrecord.studio`.
- Current version: `1.0.0` in `package.json`, `src-tauri/Cargo.toml`, and
  `src-tauri/tauri.conf.json`.
- Bundles: every Tauri desktop target; CI exercises Ubuntu, macOS, and Windows.
- macOS deployment target: 11.0. The app enables Tauri's macOS private API and native vibrancy,
  so an App Store submission is not part of this repository's configured release path.
- Runtime data: Tauri's resolved application-data directory, with
  `onrecord_studio_core.db`, `onrecord_studio_corpus.db`, `documents/`, and `reports/` beneath it.

## Release gate

Run from the application directory (`onrecord-studio/`):

```sh
npm ci
npm run check:ipc
npx tsc --noEmit
npm run lint
npm run test:unit
npm run build
cargo fmt --all --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --workspace --all-targets --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --workspace --manifest-path src-tauri/Cargo.toml
npx tauri build --ci
```

The release candidate fails if any command fails. `npm run build` repeats the IPC contract check;
keeping the explicit earlier invocation makes protocol drift visible before the longer build.

Before tagging, also verify:

1. The three version declarations above are identical.
2. `git status --short` is empty and the intended commit is checked out.
3. `npm audit --audit-level=high` and the repository's RustSec CI job pass.
4. The bundle launches on each target platform, opens the main window, can add or paste a small
   source, can search the resulting record, and can export a report.
5. The release notes list schema changes and any change to supported source or document formats.

## Signing and publication

Do not publish the unsigned CI bundle. A publishing workflow must supply platform signing material
through the CI secret store, build from the reviewed tag, and perform each platform's trust check
against the produced artifact. The repository contains no signing keys, certificates, passwords,
notarization credentials, or timestamp-service credentials.

For a signed release, record without exposing secrets:

- the source commit and tag;
- SHA-256 for every installer/archive;
- build workflow run and target operating system;
- signing identity subject/fingerprint;
- notarization or platform-verification result where the platform requires it;
- smoke-test result for the exact signed artifact.

Publication is blocked unless those records exist for every advertised platform.

## Rollout

1. Back up the complete Tauri application-data directory while OnRecord Studio is closed. The core
   database and retained evidence files are one logical dataset; backing up only the database is
   incomplete.
2. Install the signed candidate on a non-production profile and run the smoke checks above.
3. Release to a small operator group first. Confirm startup migration, source resume, search,
   evidence opening, and report export with a copied dataset.
4. Publish broadly only after the pilot produces no unexplained ingestion failures or integrity
   warnings.

## Rollback

1. Stop OnRecord Studio before changing its installation or data.
2. Preserve the failed release's entire application-data directory for diagnosis.
3. Reinstall the last verified signed artifact.
4. If the newer release opened or migrated the data, restore the pre-upgrade application-data
   backup as a unit. Do not mix an older core database with newer `documents/` or corpus files.
5. Launch offline first, confirm that the saved-source list and evidence open correctly, then resume
   network ingestion.

The legacy SQLite-to-DuckDB migration retains its source as a sibling backup, but that narrow backup
is not a substitute for the full pre-release application-data backup described here.

## Support evidence

When reporting a release failure, include the version, operating system, source kind, time range,
the visible structured error, and the relevant Activity entries. Do not attach confidential source
documents unless the recipient is authorized to receive that evidence.
