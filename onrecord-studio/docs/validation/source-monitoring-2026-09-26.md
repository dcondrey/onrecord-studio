# Acquisition, publication changes and service access

This implements roadmap priorities 10, 12 and 14 in the investigation workbench's
**Coverage and changes** view. Select a case first to link new work to it, or use
the topic selector for broader coverage.

## Consequential acquisition work

Each expectation names a custodian, record type, required period, search terms,
the analysis it would unlock, and an acquisition plan or records-request draft.
Saved research requests automatically become topic-linked expectations. These
priorities are also supplied to autonomous research planning.

The corpus search returns candidate records; it never treats a keyword match as
proof of complete production. Recording receipt requires an exact quote and
current source identity. If the underlying source changes, the receipt needs
review. A missed search result means “not located in this search,” not that an
agency lacks or withheld the record.

## Compact, cited publication differences

Accepted extraction captures the specific document immediately, even when a
large backlog awaits monitoring. The scheduled service backfills other records.
Snapshots retain Zstandard-compressed statements concerning money, dates,
obligations and outcomes with character positions and source/extraction hashes.
The comparison pairs materially similar statements, ignores simple reordering,
and preserves the before/after quotations. Changed bytes without changed
monitored text remain a distinct, limited observation.

Successful HTML bodies also supply their own document and meeting references.
The snapshot excludes attachments discovered through optional network expansion:
a failed attachment-index request cannot masquerade as a removed publication.
A missing reference means only that it is absent from the subsequently retrieved
bounded listing. File deletion, concealment and replacement identity still need
verification. Failed requests create no disappearance event.

Source removal clears the monitoring derivatives and their received-record
references. Scheduler revisions depend on evidence and user decisions, not
polling timestamps.

## Public facilities and documented operations

Already imported tables containing explicit latitude/longitude fields can
produce source-linked observation candidates. The snapshot, row number, exact
row text and row hash remain attached. A complaint or closed service request is
an observation, not automatically a completed operation. Candidates require
review before comparisons use them.

The scheduled importer advances through tables in batches of 500 rows. One
cursor per immutable snapshot prevents unsupported rows from blocking later
usable evidence; source removal also clears this progress metadata.

The UI plots recorded coordinates, allows date filtering, and links every point
to its evidence. It also accepts manually reviewed observations against retained
public excerpts. Native validation rejects guessed coordinates, unsupported
dates and weekly hours without an explicit weekly-hours measurement.

Reviewed observations support three bounded comparisons:

- Operations within 100 meters and 90 days, or the same recorded place label
  when coordinates are missing, generate a follow-up lead about repeated work.
- Two dated records for the same facility can show an hours-per-week change.
- An operation can be compared with the nearest documented facility from a
  preceding observation no more than 90 days old.

The calculations retain units, dates and source links. They do not establish
repeated displacement of the same people, program failure, actual opening hours,
a walking route, or a complete facility inventory. Missing coordinates and
hours remain unknown; a numeric zero remains a measured value.

## Checks

All ten monitoring native tests passed in the coordinated full native run
(429 passed, one ignored; `/tmp/onrecord-overhaul-native-full.log`). The four
monitoring frontend tests passed again after the final contrast changes;
`/tmp/onrecord-monitoring-ui-final.log` records that run. ESLint passed the
monitoring component, types, utilities and frontend tests.

Native fixtures exercise legacy-database initialization, compact before/after
money and obligation changes, idempotent polling, exact-document capture with a
70-record backlog, listing-reference removal, stale-citation rejection, guessed
coordinate rejection, source purge, and end-to-end table-row candidates whose
review enables a 56-to-28-hours-per-week comparison. Public-source removal
suppresses observations. A separate fixture places one usable row after 500
unsupported rows and verifies that the next batch reaches it.

Four frontend checks exercise case-scoped receipt, missing-versus-zero
measurements, and two asynchronous case-identity races. Both race tests were
proved by disabling the real guards: both failed, and passed after restoration.
Logs: `/tmp/onrecord-monitoring-identity-mutation.log` and
`/tmp/onrecord-monitoring-ui-restored.log`.

Collection and comparison bounds are displayed in the UI: up to 250 current
records and their first 12,000 characters for candidate coverage search; up to
160 monitored statements in the first 500,000 characters per document; and up to
500 observations in a view. Baselines begin when monitoring captures a record,
not at its original publication date.
