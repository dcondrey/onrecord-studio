<img align="left" width="96" alt="OnRecord Studio logo" src="onrecord-studio/assets/app-icon.svg">

### OnRecord Studio

A desktop investigation tool for journalists and lawyers working public-records releases. It
ingests agency portals, catalogs and local files, then surfaces statutory deadline violations,
redaction and provenance findings, and cross-document patterns as claims traceable to evidence.

<br clear="left">

> **Archived.** Development stopped in October 2026 at version 1.0.0. The repository is kept
> read-only for reference and for anyone who wants to pick the work up. The public datasets it
> produced continue to be refreshed independently (see [Data](#data)).

## What it does

- **Acquire.** Pulls releases from public-records portals, open-data catalogues and regional
  sources, or imports local PDFs, spreadsheets, email archives and scans. Scanned pages are made
  searchable with the operating system's text recognition.
- **Evaluate.** Counts statutory response clocks per jurisdiction, flags redaction and
  text-integrity anomalies, reads C2PA Content Credentials, and records a confidence basis for
  every finding rather than a bare verdict.
- **Analyse.** Exact SQL over DuckDB for budgets, actuals and payments; local vector search over
  extracted text; cross-document identifier linking; standing searches that watch sources for
  change.
- **Report.** Findings are claims with evidence classes, page locators and source digests. Cases
  export as signed, portable packages that a reader can verify offline.

The investigation the tool was built around, and the reports it produced, are in
[`investigations/san-diego-accountability/`](investigations/san-diego-accountability/README.md).

## Repository layout

| Path | Holds |
|---|---|
| `onrecord-studio/` | The application: Tauri 2 shell, React 18 + TypeScript frontend, Rust core, DuckDB storage |
| `onrecord-studio/src-tauri/src/storage/` | Storage layer, one file per concern (the `sqlite` module name is history; the engine is DuckDB) |
| `onrecord-studio/src-tauri/src/analysis/` | Statutory evaluation, detectors, financial reconciliation, story desk |
| `onrecord-studio/src-tauri/src/provenance.rs` | C2PA manifest reading and the provenance confidence ladder |
| `onrecord-studio/src/ipc.ts` | The only place the frontend may invoke native commands; checked by `scripts/check-ipc-contract.mjs` |
| `onrecord-studio/docs/` | Architecture, methods, data sources, data-pack design, validation records, user guide |
| `investigations/` | Published reports, evidence tables and sample sources |
| `.github/workflows/` | CI (lint, types, Rust on three platforms, dependency advisories, bundles) and the daily snapshot job, both disabled by archiving |

## Building

Requires Node 24, Rust stable and the Tauri 2 prerequisites for your platform. From
`onrecord-studio/`:

```sh
npm install
npm run tauri -- dev
```

`npm run dev` starts only the frontend preview; acquisition, storage, search and extraction need
the native application. Release packaging is described in `docs/release.md` and
`docs/release-direct.md`.

The quality gate the project was held to:

```sh
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --lib --all-targets
cargo test --manifest-path src-tauri/Cargo.toml --lib
npx tsc --noEmit
npm run lint
npx -y node@24 node_modules/vitest/vitest.mjs run
node scripts/check-ipc-contract.mjs
node scripts/check-help-coverage.mjs
npm run check:methods
```

## Data

The San Diego and California public records the application worked from are published as nine
datasets on Hugging Face under `Writerslogic/onrecord-*` (money, reports, enforcement, capacity,
snapshots, wayback, CA homelessness system, CA housing, CA public money). Each ships the original
files byte for byte with SHA-256 digests, a raw table of every publisher column, a conformed
Parquet table and a signed manifest. The design is in
[`docs/data-packs.md`](onrecord-studio/docs/data-packs.md); per-source notes are in
[`docs/data-sources/`](onrecord-studio/docs/data-sources/).

Weekly refreshes run from the separate
[onrecord-dataset-updater](https://github.com/dcondrey/onrecord-dataset-updater) repository and
do not depend on this one.

## Documentation

Start with [`onrecord-studio/docs/architecture.md`](onrecord-studio/docs/architecture.md), then
[`storage-and-evidence-architecture.md`](onrecord-studio/docs/storage-and-evidence-architecture.md)
for what stored evidence does and does not claim, and
[`METHODS.md`](onrecord-studio/docs/METHODS.md) for the analytical methods. The
[user guide](onrecord-studio/docs/user-guide.md) is the material shipped in the application under
Help. [`CLAUDE.md`](CLAUDE.md) records the engineering rules the codebase was held to.

## Distribution

Version 1.0.0 was packaged for macOS 13 and later as a Mac App Store build and as a notarized
direct download. Store listing material is in `onrecord-studio/docs/app-store/`.

## Licence

No licence file is present. Until one is added, treat the application source as all-rights-reserved.
The published reports under `investigations/` carry their own CC BY 4.0 notice.
