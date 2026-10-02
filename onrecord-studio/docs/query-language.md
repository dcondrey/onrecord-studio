# Query language structure

This document is the handoff point for the multi-surface search system.

## Product decision

The canonical user-facing grammar is a readable qualifier syntax:

```text
"body camera" agency:"City Attorney" status:closed portal:oakland flag:late
```

Users do not need to know this syntax. The evidence search currently combines the
qualifier text field with visual department/status controls. Natural-language interpretation and
saved searches are outside the supported release surface until they have a reviewable execution
plan and persistence contract.

Executable qualifier fields are `status`, `flag`, `portal`, and a uniquely resolvable `agency`.
Negation, duplicate flat fields, and ambiguous agency names are visibly marked **not applied**.
Every other `field:value` form is rejected with a query warning and is never sent to the backend as
document text.

## Canonical model

`src/query/types.ts` is the source of truth. Every surface returns `QueryAst`.
Saved searches should store the AST plus the original input for display.

## Compatibility boundary

`src/query/compile.ts` adapts the AST to the bounded flat `query_audit_records_page` IPC command.
Deferred filters are returned explicitly and rendered in amber with a count. Unknown qualifier
fields produce a warning and are excluded from the free-text predicate.

## Safety boundary

Natural-language interpretation must remain reviewable; never execute an opaque model result
without showing its filters. This is no longer hypothetical: `src/query/naturalLanguage.ts` is
implemented and `src/components/filters/SearchFilter.tsx` imports it, so the rule binds today. What
it interprets is a deterministic scaffold over dates, status, and response terms, not a model, and
every filter it produces is rendered as a chip before it is applied. SQL/Lucene compatibility
belongs in an import/export adapter, not a second execution engine.
