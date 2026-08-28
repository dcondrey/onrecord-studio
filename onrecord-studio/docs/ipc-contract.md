# IPC contract

Every value that crosses the Tauri boundary is a wire format with two independent
implementations: a Rust type that serializes it and a TypeScript type that asserts its shape.
`tsc` cannot see the Rust side and `cargo` cannot see the TypeScript side, so nothing catches a
mismatch except the rules in this document and the checks in `scripts/check-ipc-contract.mjs`.

This document describes the contract as it is. It is not a proposal, and this session did not
change any field name.

## Serialized naming

Two conventions, applied by `serde` attributes, never by hand:

| Rust | Attribute | Wire |
| --- | --- | --- |
| Struct fields | `#[serde(rename_all = "camelCase")]` | `portal_key` becomes `portalKey` |
| Enum variants | `#[serde(rename_all = "SCREAMING_SNAKE_CASE")]` | `NearDuplicate` becomes `NEAR_DUPLICATE` |

Examples of the enum convention: `CorpusPatternKind`, `PiiKind`, `OcrRequirementReason`. The
TypeScript counterpart is a string-literal union that must list the same members in the same
spelling:

```ts
export type CorpusPatternKind =
  | 'EXACT_DUPLICATE' | 'NEAR_DUPLICATE' | 'SHARED_STATUTE'
  | 'SHARED_ENTITY' | 'SHARED_TERM' | 'TEMPORAL_CLUSTER';
```

Telemetry is the one place both conventions apply at once, because `IngestionProgress` renames
its variants *and* its fields to camelCase. See below.

`Option<T>` serializes to `T | null`, not to an absent key, unless the field carries
`skip_serializing_if`. TypeScript should model those as `T | null`, and as `T | null |
undefined` only where a `skip_serializing_if` is actually present.

## AppError serialization

`AppError` (`src-tauri/src/error.rs`) is adjacently tagged:

```rust
#[serde(tag = "errorType", content = "message")]
```

so a rejected command rejects with `{ "errorType": ..., "message": ... }`.

Three details matter and each one is a trap:

**1. `errorType` is the Rust variant name, not `category()`.** The wire value is `"Storage"`,
not `"STORAGE"`. `AppError::category()` returns the SCREAMING_SNAKE form and **is not
serialized at all** — it exists for logging and native-side branching. Do not write a frontend
comparison against `"STORAGE"`; it will never match.

**2. `message` is usually a string, but not always.** Every variant carries a single `String`
except `OcrRequired`, which is a struct variant. Its `content` is therefore an object:

```json
{
  "errorType": "OcrRequired",
  "message": {
    "path": "…",
    "reason": "LOW_CHARACTER_DENSITY",
    "density": 0.004,
    "minimum_density": 0.02
  }
}
```

This asymmetry is why `formatNativeError` in `src/ipc.ts` routes `message` through
`formatErrorMessage`, which JSON-stringifies a non-string. A consumer that assumes
`typeof message === 'string'` renders `[object Object]` for exactly the OCR case.

Note `minimum_density`, not `minimumDensity`. `AppError` carries `tag` and `content` but **not**
`rename_all_fields`, so this is the one payload on the bridge whose fields are snake_case. Adding
the attribute would be a breaking protocol change. `error.rs` has a test pinning the current
spelling; do not "fix" it without versioning the change.

**3. `is_retryable()` and `is_source_repairable()` are native-side policy and do not cross the
bridge.** The frontend cannot ask whether an error is retryable; it can only match on
`errorType`. Keep the two in sync by hand or move the decision into a serialized field.

All native calls funnel through `invokeCommand`, which wraps any rejection in
`NativeCommandError` carrying the raw `details`. That preserves the structured error for a
caller that wants to branch on `errorType` while giving a readable `message` by default.

## Telemetry protocol versioning

Ingestion telemetry is emitted on the `ingestion-telemetry` event as a flat object:

```json
{
  "protocolVersion": 1,
  "sequence": "1",
  "emittedAt": "2026-08-25T05:00:00Z",
  "type": "pageCompleted",
  "payload": { "portalKey": "…", "page": 3, "recordsFound": 25 }
}
```

The envelope is `IngestionTelemetryEnvelope`, which `#[serde(flatten)]`s an adjacently tagged
`IngestionProgress` (`tag = "type"`, `content = "payload"`). That is why `type` and `payload`
sit beside `protocolVersion` rather than nested under an `event` key.

Rules:

- **`protocolVersion` is currently the literal `1`**, and TypeScript types it as the literal
  `1` rather than `number`. The frontend rejects any other value before dispatching. Bumping it
  is therefore a deliberate breaking change on both sides at once, which is the intended
  friction.
- **Adding a variant is backward compatible on the wire but not in practice.** A consumer
  without a `default` case silently drops the new event. Adding a variant does not require a
  version bump; it does require a TypeScript union member and a handler.
- **Renaming or removing a variant or a payload field is breaking and is not mechanically
  guarded.** `IngestionProgress` derives `Serialize` only, with no `Deserialize`, so there is no
  round-trip test that could catch it.
- **The stream is lossy by design.** Telemetry sends discard their result, so `sequence` gaps
  are expected. Do not build gap detection on top of it.

Contrast `IngestionTask`, which is `Serialize + Deserialize` because it is persisted in the
durable frontier, and which uses `#[serde(default)]` on newer fields so rows written before a
field existed still deserialize. Telemetry has no equivalent mechanism because it is never read
back.

## Process-local versus durable identifiers

The single most common bridge bug is assuming an identifier is stable when it is not. Three
different lifetimes appear in one payload.

| Identifier | Lifetime | Ordered? | Safe to persist or key on? |
| --- | --- | --- | --- |
| `sequence` | **Process-local.** Restarts at `"1"` on every app launch | Within one process only | **No** |
| `syncRunId` | Durable for the run | **No** — it is a random UUID v4 | Yes, as an identity; never as an ordering |
| `documentId`, `requestId` | Durable, deterministic | No | Yes |
| `queuedTasks` | Durable-ish (seeded from the frontier at pool start) | n/a | Read as a gauge, not a total |
| `completedTasks`, `failedTasks`, `pagesProcessed`, `requestsProcessed`, `documentsProcessed` | **Process-local.** Reset to 0 on restart | n/a | **No** |

Two consequences worth stating outright:

- **`sequence` is a string, and it is deduplication metadata, not an ordinal.** It is
  transported as a decimal string for uniform handling across runtimes. The frontend uses it
  only for `Set` membership. Note that string identity is not numeric identity: `"01"` and `"1"`
  are different keys. Comparing it numerically also means trusting a value that can exceed
  `Number.MAX_SAFE_INTEGER`.
- **`PortalRuntimeSnapshot` mixes durabilities in one struct.** After a restart that adopts a
  durable frontier, `queuedTasks` reflects persisted truth while every completion counter starts
  at 0 for the same logically continuing run. A progress bar computing
  `completed / (completed + queued)` reads wrong across a restart.

Three telemetry variants carry **no `portalKey`**: `legacyCoreDatabaseMigrated`,
`corpusReconciliationStarted`, and `corpusReconciliationCompleted`. They are app-scoped. A
consumer that keys every event by portal drops them.

`documentId` and `bytesSaved` serialize as JSON numbers. Synthetic document identifiers are
deliberately generated below 2^53 so they stay exactly representable in JavaScript, but
upstream portal identifiers are unconstrained `u64` and are **not** protected by that. An
upstream identifier above 2^53 loses precision on the TypeScript side.

## Compatibility-field naming

Some serialized names are wrong on their merits and are kept because renaming them is a
coordinated change across the bridge and, for persisted shapes, across existing data.

The worked example is `pendingWriteCapacity` on `CorpusIndexStatus`:

```rust
// src-tauri/src/types/corpus.rs
// Compatibility name: this is the writer queue's remaining capacity, not its pending depth.
pub pending_write_capacity: usize,
```

It is `tokio::sync::mpsc::Sender::capacity()`, which is **remaining free slots**. So the value
runs backwards from what the name suggests:

- `64` means the writer queue is **empty** and the indexer is idle.
- `0` means the queue is **full** and the indexer is saturated.

A UI that renders it as a backlog gauge shows a full bar when nothing is happening. Both the
Rust and TypeScript declarations carry the warning, and both must keep carrying it until the
field is renamed in a versioned change.

Rules for this class of field:

- Never fix only one side. The name is the contract.
- Keep a `COMPATIBILITY:`-style comment on **both** declarations. A reader of `src/types/corpus.ts`
  will never see a comment that lives only in Rust.
- Record the intended new name in the comment so the eventual rename is mechanical.

## Updating both sides

Any change to a serialized shape is a two-file change at minimum, and often four:

1. The Rust type (`src-tauri/src/types/*.rs`, `src-tauri/src/error.rs`, or the telemetry enum).
2. The TypeScript type (`src/types/*.ts`).
3. `src/ipc.ts` if the change touches a command's arguments or result.
4. The consumer, if a new telemetry variant needs a handler.

`npm run check:ipc` checks the generated bridge and its use:

- **Command shapes.** `scripts/generate-contracts.mjs` derives every registered command's argument
  names, optionality, and response type from its Rust signature into
  `src/generated/nativeCommands.ts`.
- **Shared protocol shapes.** The same generator derives all `IngestionProgress` payloads,
  `VerificationState`, `TimelineAnomalyMarker`, and `AnalysisConfig` into `src/generated/`.
- **Staleness.** Check mode regenerates all four files in memory and compares them byte-for-byte
  with the checked-in output, so any supported Rust field or variant change fails the build.
- **Centralization.** It fails if any file under `src/` other than `src/ipc.ts` calls `invoke(`
  directly. All native calls must route through `invokeCommand`.

The generator deliberately supports the Rust schema forms used by this bridge: named command
parameters, `Result<T, AppError>`, struct-style ingestion variants, unit audit enums, `Option<T>`,
`Vec<T>`, primitive scalars, and application types re-exported from `src/types`. Unsupported syntax
is a hard generator error rather than a partial contract. Review is still required for:

- **Semantic compatibility.** A field can retain its serialized type while changing meaning.
- **Nested application types.** The generator references their exported TypeScript names; changes
  within those independently maintained structs still require paired review.
- **Consumer behavior.** A generated union ensures the payload is typed, but a consumer can still
  intentionally ignore a telemetry event. `App.tsx` records every event and explicitly handles
  the state-changing variants.

Run `npm run generate:contracts` after an intentional Rust protocol change and commit the generated
files with that change. Never edit files under `src/generated/` directly.
