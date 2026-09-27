# SDHC shelter records: acquisition that reaches the evidence

The fresh-install collector already schedules City budget exports, audit reports,
Council records and homelessness indexes. It did not seed the Housing Commission
board's operating-agreement records or City purchasing/payment indexes. More
seriously, SDHC's board page publishes many PDF documents through extensionless
WordPress routes. The generic crawler treated those URLs as additional HTML pages;
they competed with navigation under the page limit instead of entering the
document-download budget.

The adapter now recognizes literal document links inside the SDHC board's
published meeting tables. It accepts report labels, agendas, minutes,
presentations and resolutions only on the verified board index/archive and the
same publisher. It preserves the full row's descriptive context in the link title
and schedules consequential reports before slides and navigation. It neither
generates missing URLs nor executes page scripts. Actual file signatures still
determine which parser runs.

## Reproducible production check

`src-tauri/tests/sdhc_acquisition_live.rs` creates a fresh DuckDB database and
corpus, then uses the production `QueueManager` with **default source guidance**
to acquire the board index and its first eight automatically ranked reports with
a **one-page, nine-resource** budget. No document URL, selector or topic filter is
supplied to guide collection. Three known shelter reports are assertions on the
result, not inputs to discovery. The check requires searchable document bodies,
source hashes, fiscal text and the amounts below, and retrieves funding evidence
through a natural-language research query. It uses no model calls. Originals and the temporary
database are deleted when the test finishes; a compact result trace is written
to the operating system's temporary directory as
`onrecord-sdhc-acquisition-validation.json`.

The live check **passed on September 27, 2026 in 10.49 seconds**. The three
asserted report bodies contained 19,731, 19,032 and 22,043 characters respectively
(60,806 total), from 1,363,374 bytes of original PDFs. The research query returned
eight excerpts and included the Imperial Shelter funding passage with its source
URL. The [compact trace](sdhc-acquisition-2026-09-27.json) records the automatic
first-eight selection, original-file hashes, exact URLs and short source quotes.
The selection also reached the agency's unaudited FY2026 second-quarter financial
statements. This test deeply checks the three shelter report bodies; it does not
assert that every selected report contains equally useful evidence.

| Published report | Program | FY2027 proposed operator amount |
|---|---|---:|
| [HCR26-014](https://sdhc.org/103_hcr26-014-imperial-shelter_final/) | Imperial Shelter for single adults | $3,523,195 |
| [HCR26-015](https://sdhc.org/104_hcr26-015-harm-reduction-shelter_final/) | Harm Reduction Interim Shelter | $1,970,278 |
| [HCR26-016](https://sdhc.org/101_hcr26-016-single-adults-shelter_final/) | Single Adults Interim Shelter at Pacific Highway | $2,865,640 |

The reports supply much more than totals: proposed terms, bed capacity,
provider-selection information, anticipated funding, administrative costs and
authority to vary compensation. These fields support specific next requests for
executed agreements, amendments, invoices and period-matched service outcomes.

These are published approval reports. They do not establish executed agreements,
actual spending, occupied bed-days or housing outcomes. Capacity alone is not a
comparable outcome denominator. The live test verifies three current records
inside a bounded automatic collection and does not claim complete archive or
shelter-performance coverage.
The native PDF text layer inserts spaces inside some identifiers, for example
`HCR26-0 14`. The test removes whitespace only when checking the report identifier;
it requires the actual amount literal and preserves the unmodified source quote.

The deterministic unit test
`sdhc_board_extensionless_reports_are_downloads_with_usable_context` checks
classification, descriptive labels, priority and rejection of unrelated-page
promotion. The live command is:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test sdhc_acquisition_live -- --ignored --nocapture
```

## Public follow-up evidence, before requesting unavailable records

The following official records were inspected separately while checking what the
collector should pursue next. They are not evidence of a successful automated
download unless a collection trace identifies them.

- The [FY2026 third-quarter budget monitoring report](https://www.sandiego.gov/sites/default/files/2026-05/fy2026-third-quarter-budget-monitoring-report.pdf),
  Attachment 7, PDF pages 80–82, reports $507,141 in credits from SDHC for FY2025
  overbilling across shelters and outreach. It also identifies projected
  underspending of $441,143 from delayed Rachel's Promise expansion and $539,542
  in costs for Newton shelter relocation and repairs after flooding. These are
  three different financial events, not amounts to add into a waste estimate.
  Program results cover July 2025 through February 2026; projected fiscal-year
  expenses must not be divided by those partial-year results as a realized cost
  per outcome.
- SDHC's [August 2026 Rachel's Promise expansion announcement](https://sdhc.org/news-release/rachels-promise-shelter-expansion/)
  reports 2,864 FY2026 family referrals to the Coordinated Shelter Intake Program
  and 58 accommodated referrals. The source does not establish that these are
  unique families or all referrals to Rachel's Promise. This supplies a specific
  question about unmet demand, referral duplication and the effect of the delay;
  it does not establish why each referral was unfilled.
- The [FY2025 Newton shelter board report](https://sdhc.org/wp-content/uploads/2025/05/HCR25-026-Alpha-Newton-Bridge-Shelter.Final_.pdf)
  includes FY2024 and partial-FY2025 participants and housing exits, plus a
  proposed FY2026 agreement. It can support a dated program history and questions
  about noncompetitive procurement. Its cross-year figures are not a valid
  same-period cost/outcome calculation.

Executed agreements, invoice-level credits, matched service periods and referral
dispositions remain narrower gaps after these public records are examined.
