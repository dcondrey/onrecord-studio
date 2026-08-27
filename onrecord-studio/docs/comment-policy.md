# Comment policy

This codebase parses hostile input (public-records portals we do not control, documents
uploaded by third parties) and makes claims about evidence that an investigator may rely on
in a legal filing. That raises the cost of a wrong comment above the cost of no comment: a
stale comment asserting a security property that the code no longer enforces is worse than
silence, because it stops the next reader from checking.

The rules below are about *which* comments earn their place, not about density.

## The one rule

**Comments describe why, not what.** The code already states what it does. A comment earns
its place when it records something the code cannot: a reason, a constraint from outside the
file, a rejected alternative, or the blast radius of changing a line.

```rust
// Bad: restates the expression.
// Increment the redirect count.
redirect_count += 1;

// Good: records the constraint and what breaks without it.
// Counted per request, not per host: a redirect chain that alternates between two
// permitted hosts would otherwise loop until the connect timeout.
redirect_count += 1;
```

If you cannot write the "why" without restating the "what", delete the comment.

## The tag form

Every tagged comment is `PREFIX(area):`. This is the form already in the tree
(`NOTE(runtime):` in `blocking.rs`, `PERF(pii):` in `storage/pii.rs`, `TODO(ipc):` in
`src/ipc.ts`), and it is what makes `rg 'SECURITY\('` a complete audit.

Areas in use, extend as needed rather than inventing a synonym:

| Area | Covers |
| --- | --- |
| `security` | Auth, SSRF, injection, redaction, anything in a trust boundary |
| `compliance` | Statutory deadlines, legal-authority evaluation, audit records |
| `storage` | Schema, migrations, indexes, durability, queue persistence |
| `protocol` / `ipc` | IPC shapes, telemetry events, serialized field names |
| `runtime` | Threading, blocking pools, cancellation, task lifetime |
| `corpus` | Cross-document indexing, patterns, evidence references |
| `pii` | Detection and masking of sensitive values |
| `integrity` | Hashing, provenance, chain of custody |
| `forensics` | Salvage, carving, recovery confidence |
| `perf` | Complexity and allocation problems with a measured or obvious cost |
| `test` | A missing test, especially one backing a `SECURITY(...)` comment |

Two prefixes already in the tree keep working and are not being swept:

- `NOTE(area):` is the general why-comment. Reach for a more specific prefix below when one
  fits; `NOTE` is correct when none does.
- `PERF(area):` is a cost note. It is a `LIMIT`-flavored comment specialized to performance.

**Pre-existing comments are grandfathered.** Do not open a diff purely to retag them, and do
not retag files another session owns. Apply this policy to comments you are writing or
already editing for another reason.

## The five kinds

Each kind has a prefix so it is greppable. Use the prefix; it is how a reviewer finds every
security assumption in the tree in one command.

### 1. Invariant comments — `INVARIANT:`

State a condition that must hold across a scope wider than one function, and name what
maintains it. Use these where the type system cannot carry the constraint.

```rust
// INVARIANT: `byte_start`/`byte_end` index into the ORIGINAL text, not the masked output.
// `mask_byte_range` rewrites the text but deliberately does not rebase these offsets,
// because callers correlate findings back to the stored document.
```

Write one when: an offset is relative to something non-obvious; two fields must be updated
together; ordering matters; a lock or a sequence number is what makes the code correct.

Do not write one for a condition a reader can see in the adjacent three lines.

### 2. Security-boundary comments — `SECURITY:`

Mark the line where untrusted input stops being trusted, and say what is being defended
against. Name the attack, not the mechanism.

```rust
// SECURITY: Every redirect hop is revalidated, not just the initial URL. A permitted host
// that 302s to 169.254.169.254 is the SSRF path this defends against.
```

**A `SECURITY:` comment must have a nearby test.** This is the rule with teeth. A security
assumption written in prose and unprotected by a test is a comment that will silently become
false. "Nearby" means the same module's `#[cfg(test)]` block, or the sibling `.test.ts`
file — close enough that deleting the code and the test together is a deliberate act.

If you write a `SECURITY:` comment and cannot write the test, write the test as
`#[ignore]`/`it.skip` with a `TODO(security):` explaining what is needed. Do not leave the
claim unbacked.

### 3. Compatibility comments — `COMPATIBILITY:`

Explain a name, shape, or behavior that is wrong on its merits and kept because changing it
would break something outside this file — a serialized IPC field the webview reads, an
on-disk schema, a persisted queue format.

```rust
// COMPATIBILITY: `pending_write_capacity` is the writer queue's REMAINING capacity, not its
// pending depth. The name is wrong and the webview reads it; renaming requires a coordinated
// Rust + TypeScript change. See docs/ipc-contract.md.
```

Every misleading public field name should carry one of these. It is the difference between a
bug and a documented wart.

### 4. Limitation comments — `LIMITATION:`

State where a heuristic stops working. **The most important use of this policy is to stop
heuristics from being described as guarantees.**

If the code does a best-effort thing, the comment must say so in words that survive being
read in a hurry. Words like "ensures", "guarantees", "prevents", "all", and "never" are
reserved for behavior that is actually total. A detector that matches common formats says
"detects common X"; it does not say "detects X".

```rust
// LIMITATION: Structural detection only. Values that do not match the expected separator
// layout are not found, so a masked result is NOT a guarantee that the text is clean.
// Treat this as defense in depth behind human review, never as a release gate.
```

Write one whenever a caller could over-trust the result: any detector, classifier, salvage
routine, confidence score, or sniffing heuristic.

### 5. TODO / FEATURE comments — `TODO(<area>):` and `FEATURE(<area>):`

`TODO` is work that should happen and has a known shape. `FEATURE` is a capability that was
deliberately deferred, not an oversight.

Both take an area tag so they can be triaged without an issue tracker. Use the stable local
set:

| Tag | Area |
| --- | --- |
| `TODO(security):` | Auth, SSRF, injection, redaction, anything in a trust boundary |
| `TODO(compliance):` | Statutory deadlines, legal-authority evaluation, audit records |
| `TODO(storage):` | Schema, migrations, indexes, durability, queue persistence |
| `TODO(protocol):` | IPC shapes, telemetry events, serialized field names |
| `TODO(perf):` | Complexity and allocation problems with a measured or obvious cost |
| `TODO(test):` | A missing test, especially one backing a `SECURITY:` comment |

**Do not invent external issue numbers.** A `TODO(security): see #482` pointing at an issue
that does not exist is worse than no reference. The area tag plus a specific sentence is the
reference.

A TODO must state the problem, not the mood. `TODO: this is ugly` is noise.
`TODO(perf): quadratic in findings — rescans the prefix per match; build a byte-to-character
index in one pass` is actionable by someone who has never seen the file.

## Long plans belong elsewhere

If a comment is running past roughly ten lines, it is not a comment. Multi-step migration
plans, design rationale with alternatives considered, and protocol specifications go in
`docs/` and get linked from a one-line comment:

```rust
// See docs/ipc-contract.md for the serialization rules this struct must satisfy.
```

The test: if the text would still be worth reading by someone not editing this function, it
belongs in a document. Comments are for the person whose cursor is on that line.

## Removing comments

Delete on sight:

- Comments that restate the line below them.
- Section banners (`// ---- helpers ----`) — that is what the file structure is for.
- Commented-out code. It is in git.
- Changelog comments (`// Added 2026-03 by ...`). Also git.
- Guarantees that the code does not deliver — either fix the comment to `LIMITATION:` or fix
  the code.

Keep, even if terse, any comment that names an external constraint, an attack, or a reason a
tempting simplification is wrong. Those are unrecoverable once deleted; you cannot rederive
them from the code.

## Review checklist

- Does every `SECURITY:` comment in the diff have a test within the same module?
- Does any comment use a totality word ("ensures", "all", "never") for a heuristic?
- Does any TODO carry an area tag and a specific, actionable sentence?
- Is any comment longer than ten lines that should be a `docs/` page?
- Did a renamed field leave a `COMPATIBILITY:` comment stale?
