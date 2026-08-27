# Query language structure

This document is the handoff point for the multi-surface search system.

## Product decision

The canonical user-facing grammar is a readable qualifier syntax:

```text
"body camera" agency:"City Attorney" status:closed after:2025-01-01 -has:response
```

Users should not need to know this syntax. The evidence search should eventually
offer three equivalent entry points:

1. Visual filters and editable chips.
2. Qualifier text with autocomplete and inline validation.
3. Natural-language text with an explicit interpreted-query preview.

## Canonical model

`src/query/types.ts` is the source of truth. Every surface returns `QueryAst`.
Saved searches should store the AST plus the original input for display.

## Compatibility boundary

`src/query/compile.ts` adapts the richer AST to the current flat
`query_audit_records` IPC command. Deferred filters are returned explicitly so
the UI can explain what is not yet enforced. No unsupported qualifier should be
silently ignored.

## TODO / NOTES

- TODO(query): Add a typed Rust IPC payload for `QueryAst`.
- TODO(query): Resolve `agency:"name"` against the loaded agency index.
- TODO(query): Add date, document-existence, type, portal, and negation support
  to the Rust query planner.
- TODO(query): Add parenthesized boolean groups and a query formatter.
- TODO(query): Add saved-search persistence and shareable links.
- TODO(query): Add parser/property tests for quoting, escaping, dates, and
  malformed input.
- NOTE(query): Natural-language interpretation must remain reviewable; never
  execute an opaque model result without showing its filters.
- NOTE(query): SQL/Lucene compatibility should be an import/export feature,
  not a second execution engine.
