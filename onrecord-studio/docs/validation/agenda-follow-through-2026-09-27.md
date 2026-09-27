# Exact Council agenda follow-through

The deeper live investigation acquired the September 28, 2026 Council agenda but
its lexical excerpts ended before the complete Riverbed grant amendment. The
model consequently omitted an actionable meeting. `research/agenda.rs` now reads
the retained official agenda structure directly: an explicit City of San Diego
Council agenda identity, an unambiguous full meeting date in the heading, and
complete topic-relevant item regions ending at the next explicit item heading.

The [original published agenda](https://www.sandiego.gov/sites/default/files/2026-09/9-28-26-special.pdf)
has SHA-256
`2fcf7ff0011c1047bf58cc50120ab00d970b07a135982b63a67dbe0a5433dc9f`.
Item 605 continues across the page 8 heading and proposes acceptance,
appropriation and expenditure of $3,666,572 and a term extension to June 14, 2027.
The meeting date comes from the agenda heading, not that extension or the dates
of minutes being approved.

The processed text fixture was extracted from the public PDF with `pdftotext
-layout` and whitespace compacted; the original PDF was removed. A second fixture
preserves the native retained excerpt and its original offset from the earlier
live dossier. It verifies that the native extraction's page-heading text does
not truncate the grant action. Generated evidence preserves character offsets,
source URL and original-file hash. Date evidence is a narrow intact heading
span, avoiding unrelated adjacent telephone numbers.

The candidate inventory supplies exact date and item citations. If a model omits
an item, an explicitly disclosed deterministic fallback can suggest comparing
prior/proposed funding, scope, deliverables and reporting, checking current
participation instructions, and preserving the eventual vote and executed
version. Historical meetings instead suggest retrieving the published vote and
executed version. No participation deadline, adoption, payment or execution is
inferred.

Coverage is deliberately bounded: at most 24 retained agendas, 500,000 characters
per document, six candidates, and three fallback actions. Items longer than
12,000 characters are flagged rather than truncated. A last item without a
following explicit boundary, ambiguous dates, unofficial copies and other
meeting-body/layout formats are not accepted by this extractor. The absence of a
candidate does not establish that no relevant meeting exists.

All four targeted native tests passed: complete actual-agenda item/date/offsets,
the page-spanning native excerpt, rejection of unsupported dates/sources, and the
DuckDB-to-fallback path through the existing citation validator. The subsequent
live investigation's preflight also recovered Item 605 from the actual native
retained extraction before making any model call.
