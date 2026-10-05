# OnRecord Studio - Forensic Analytical Methodologies & Calculation Specifications

This document defines the calculations the Rust backend performs over ingested records. Every
threshold below is a named constant in the source; the citation after each one is authoritative,
and this document is wrong if they ever disagree.

Storage note: DuckDB is the persistent store. SQLite appears only in `parsers/database.rs`, which
opens investigator-supplied SQLite files read-only, and in the one-way upgrade path in
`storage/legacy_import.rs`.

---

## 1. Chronological Timeline Anomaly Detection

The native Rust backend analyzes public record timestamp histories to detect structural signs of data manipulation or administrative backlog trimming.

### A. Chronological Discrepancy Scans
The system compares creation and modification timestamps to identify logical sequence errors:
\[\Delta t_{\text{skew}} = t_{\text{updated\_at}} - t_{\text{created\_at}}\]
\[\text{Condition Flag} = \Delta t_{\text{skew}} < 0 \implies \text{UPDATE\_BEFORE\_CREATION}\]

\[\Delta t_{\text{lifespan}} = t_{\text{closed\_at}} - t_{\text{created\_at}}\]
\[\text{Condition Flag} = \Delta t_{\text{lifespan}} < 0 \implies \text{CLOSE\_BEFORE\_CREATION}\]

### B. Arbitrary Closure Index (Fast-Drop Filtering)
Agencies may quickly dismiss incoming public interest requests to artificially lower their open-ticket metrics. The system flags this behavior by measuring response times against a strict processing window:
\[\text{FastDrop} = (\Delta t_{\text{lifespan}} \le 300\text{ seconds}) \land (\text{Status} = \text{"Closed"})\]
* **Constant:** `SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS = 300` (`analysis/timeline.rs`).
* **Triage Routing:** Triggering this flag marks the record as `SUSPICIOUSLY_FAST_CLOSURE` and ranks it higher in the inspector's manual audit list.

---

## 2. Multi-Variable Departmental Outlier Detection (OLAP Core)

The analytical engine uses DuckDB to calculate citywide performance metrics, identifying specific agencies with statistically abnormal delays.

```text
[ DuckDB core (persistent, on disk) ]
              |
              |  live sibling connection (Connection::try_clone)
              v
[ AnalyticsEngine ] -- group-by aggregation: mean closure latency per agency
              |
              v
       global mean and population standard deviation
              |
              v
       per-agency Z-score -> outlier flag
```

The analytics node must read through a sibling connection of the writer, never through a separate
connection that `ATTACH`es the core file: an attaching connection observes a snapshot frozen at
attach time and would report the database as it stood when the process started.

### A. Global Variance Formulations
The system computes the global average response latency (μ) and standard deviation (σ) across all processed cases:
\[\mu_{\text{global}} = \frac{1}{N}\sum_{i=1}^{N}\Delta t_{\text{lifespan}, i}\]

\[\sigma_{\text{global}} = \sqrt{\frac{1}{N}\sum_{i=1}^{N}(\Delta t_{\text{lifespan}, i} - \mu_{\text{global}})^2}\]

### B. Departmental Z-Score Ranking
The system monitors departmental performance by calculating Z-scores relative to citywide processing averages:
\[Z_{\text{agency}} = \frac{\mu_{\text{agency}} - \mu_{\text{global}}}{\sigma_{\text{global}}}\]
* **Classification:** one threshold is implemented. An agency is reported as a statutory-deadline
  outlier when \(Z \ge 2.0\) **or** its statutory violation count is non-zero
  (`STATUTORY_DEADLINE_Z_SCORE = 2.0`, `analysis/anomaly.rs`). There is no intermediate tier.
* \(\sigma_{\text{global}}\) is the population standard deviation, and the Z-score is defined as
  zero whenever the sample size is zero or \(\sigma_{\text{global}} = 0\), so a single-agency
  corpus never reports an outlier.

---

## 3. High-Density Text Redaction Analysis

When files are parsed, the system measures the density of redacted text blocks relative to standard content to identify hidden information dumps.

\[\text{Redaction Ratio} (R_d) = \frac{\text{Number of characters covered by redaction matches}}{\text{Total character length of the extracted text}}\]

The numerator counts *characters*, not placeholders: `[REDACTED]████` scores 1.0, not 2/14.
* **Pattern kinds** (`RedactionPatternKind`, `analysis/cluster.rs`), matched by a hand-written
  scanner rather than a regular expression engine: `ExplicitRedactedLabel`,
  `WithholdingExemption`, and `BlackUnicodeBlock` (runs of `U+2588`).
* **Triage Routing:** Files where \(R_d > 0.45\) are flagged `HEAVY_REDACTION_DENSITY`
  (`HEAVY_REDACTION_DENSITY_THRESHOLD = 0.45`, `analysis/cluster.rs`), pointing investigators to
  files that may warrant legal review or a formal appeal.

---

## 4. Local NLP Keyword Clustering & Density Maps

To isolate records tied to systemic homelessness or housing rights violations, the engine processes unformatted description text through a case-insensitive keyword filter.

### A. Word-Stem Target Matrix
Matching is case-folded against `enforcement_keyword` and the bounded two-token phrase scanner in
`enforcement_keywords` (`analysis/cluster.rs`). Six labels are implemented; each maps its accepted
surface forms to one stable label:

| Label | Accepted tokens |
|---|---|
| `sweep` | `sweep`, `sweeps`, `sweeping`, `sweeped`, `swept` |
| `encampment` | `encampment`, `encampments` |
| `eviction` | `evict`, `evicts`, `evicted`, `evicting`, `eviction`, `evictions` |
| `clean` | `clean`, `cleans`, `cleaned`, `cleaning`, `cleanup`, `cleanups` |
| `displace` | `displace`, `displaces`, `displaced`, `displacing`, `displacement`, `displacements` |
| `property storage` | exact adjacent phrase `property storage` or `property stored` |

### B. Density Calculation
$$\text{Keyword Density} (D_k) = \frac{\text{Total Count of Keyword Matches}}{\text{Total Count of Words in Case Description}} \times 100$$
* **Application:** High $D_k$ scores filter out generic municipal maintenance requests, isolating target enforcement events across the dataset.

## 5. Prose Figures Checked Against Imported Tables

`analysis/claim_validation.rs`, run by the `claim_data_mismatch` detector; findings carry the kind `CLAIM_CONTRADICTS_DATA`.

### A. What is extracted
A sentence in a public document becomes a **quantitative claim** when a token-phrase cue (never a regular expression) names a measure and a figure: a stated total (`the adopted budget of $1.2 billion`), a percent change against a prior year (`spending fell 12% from the prior year`), a within-budget statement, or a balance statement (`balanced`, `surplus`, `deficit`). Each claim carries the **scope its own words state**: fiscal year, fund, department, and the basis (`proposed`, `revised`, `adopted`, or actual spending). A sentence stating no fiscal year inherits the last one named in the document's opening (its heading, the first 600 bytes); the claim records that the year was inherited, and confidence is lower for it (§D). A sentence naming two dollar amounts or two percentages (`grew from $1.0 million to $1.2 million`) is a comparison, not one figure, and yields no claim. A hedge (`about`, `at least`, `under`) is recorded with the figure.

### B. How a claim is bound to data
A claim is evaluated only against datasets whose jurisdiction matches the document's source, through the existing financial mapping roles (`budget`, `actual`) and a `FinancialScope`. Fund and department names must match exactly one distinct value in the bound dataset. Every comparison runs through `execute_restricted_sql`; the query ids and the aggregated rows are the evidence, so a reader can re-run the same query.

### C. Tolerance
A prose figure is compared at the precision it states, not to the cent:
$$\text{band} = \pm \tfrac{1}{2}\,\text{unit of the last significant digit stated}$$
so `$1.2 billion` is consistent with any total in $[\$1.15\text{B}, \$1.25\text{B}]$; `about` doubles the band, `at least`/`under` make it one-sided. A figure outside its own band is a **mismatch**; inside it is **consistent**.

### D. What a finding does and does not claim
* A `CLAIM_CONTRADICTS_DATA` finding states only: *this sentence's figure, at the precision it states, falls outside the aggregate of the rows it binds to*, with the bound scope, the band, the observed value, and the query ids. It is a pattern signal about text versus table (`is_legal_claim` is false). It is not a statement of intent, error, or law.
* A claim that cannot be bound is **unreconciled**, never a contradiction: no dataset for the jurisdiction, ambiguous fund or department, no fiscal year, no stated budget-versus-actual basis, a partial or rejected-row dataset, or a claim kind the data cannot support. Balance, surplus and deficit claims are evaluated only when datasets are mapped to both the `revenue` and `actual` roles (revenue − actual expenditures for the bound scope; a stated amount is held to its rounding band, a bare word only to the sign); otherwise they are unreconciled. An account-number convention is not inferred to split one table into both roles.
* Confidence is derived from what was observed: 6000 basis points for a mismatch whose scope was completed from context, +1500 when the sentence named the fund, +1500 when it stated the fiscal year itself rather than inheriting the heading's. A complete dataset is a precondition, not a bonus. Confidence is unassessed whenever the claim is unreconciled.
* Consistent and unreconciled outcomes are returned by the sweep alongside mismatches so a reader can see what was checked and what could not be; only mismatches are stored as findings.

### E. Statements as claim sources
A **public statement** is what an agency said, as opposed to a record of what it did: a press release, an official statement or remarks, a meeting notice, or a budget message. It is a `DocumentClass` (`PublicStatement`) derived from the document's own words with the same name-and-opening-weighted cue scoring as every other class (`for immediate release`, `press release`, `statement from`, `remarks`, `regular meeting of`, `state of the city`, …); agendas and minutes stay under `Minutes`, and both classes count as statement-side sources. Nothing about ingestion changes: a statement is pasted or downloaded like any document, and the claim check runs on it the same way. When the source is a statement, the finding's headline reads **"Official statement vs released record"** and cites both the statement span and the query ids behind the released data. The finding states that the two do not match at the stated precision; it never says the statement was false or that anyone misled — a revised export, a different basis, or a later correction can account for the gap, and the finding says so.

## 6. Lead Ranking (Story Desk)

`analysis/story_desk.rs`, served by `list_story_leads`; rendered in the Story desk view.

Undismissed findings are ordered by a **public-interest score**: a saturating sum of basis points from named factors, each carrying the observed fact behind it so a reader can check it against the finding. The factors and their weights live in one table in the module: money at stake (largest `$` figure named in the headline, explanation or quotes, banded at $10k / $100k / $1M), scale (distinct documents, requests and agencies across the finding's citations and the findings sharing its request, document or entity; cross-corpus kinds count as multi-agency), recurrence (other undismissed findings from the same detector for the same source), a record contradicting a statement (the contradiction kinds), a counted statutory or policy violation (`is_legal_claim` kinds only, and the only factor allowed to use that word), evidence strength (measured confidence scaled to a fifth; unassessed contributes nothing and says so), and freshness. Ties break on severity, then last seen, then id.

The score is a ranking heuristic over facts the detectors recorded. It is not a measurement of newsworthiness and never a claim about any record; pattern signals remain labelled as pattern signals on the desk, and a finding's own confidence and citations remain the evidence.

## 7. Lead Dossier

`analysis/lead_dossier.rs`, served by `build_lead_dossier` and opened from a finding in the inbox.

A dossier assembles what is already stored around one finding: the **evidence chain** (each cited span resolved to its document, request, agency and recorded digest, or marked unresolved), the **related findings** that touch the same request or the same documents, a **falsification test** (one fixed, kind-specific statement of what would disconfirm the lead and the records that would show it — an exhaustive table, so a new finding kind cannot ship without one), and the **gaps**: figures in the cited documents that could not be checked against the imported tables, each with its reason, and documents whose text could not be read at all. Nothing is persisted or inferred; a dossier restates observations and says what is missing. It is not a legal claim and uses no word of judgement.

## 8. Facility Coverage From Imported Facility Lists

`analysis/facility_coverage.rs` (hours grammar and per-area arithmetic), `storage/facilities.rs` (mapping, area membership), commands `get_facility_coverage` and `save_facility_coverage_mapping`. This is a descriptive view. It produces no finding and no confidence value.

### A. Input
An agency facility list is imported through the ordinary analytical import (`REGIONAL_DATA`). A `FacilityCoverageMapping`, stored per snapshot in `facility_coverage_mappings`, names the source columns: facility name (required), address or latitude and longitude (at least one), and optionally a facility id, daily open hours and category. When a category column is mapped the user lists the category values that identify a public restroom (exact match, case ignored); a row with no recorded category is not counted and the report states how many such rows exist. Without a category column every row is counted.

Areas are supplied by the user and stored in the same mapping: a name plus exactly one of an inclusive latitude/longitude box or a list of facility ids. A box needs mapped coordinates; rows whose coordinates do not cast to a finite number are reported as `locationNotRecorded` and fall inside no box. Addresses are never geocoded. Listed ids that match no counted row are reported per area.

### B. Hours grammar
Each hours value is one of three states: **recorded**, **uninterpretable**, or **not recorded** (blank). Recorded forms, after lowercasing and removing whitespace: `closed` (recorded, open no minutes); `24 hours`, `24hrs`, `24/7`, `open 24 hours` (open all 1440 minutes); or one or more ranges separated by `,` or `;`, each `start-end` or `start to end`. An endpoint is `HH:MM` on a 24-hour clock (`24:00` allowed only as an end), `h[:mm]am|pm`, `noon`, or `midnight`. A bare hour without minutes or a meridiem (`8-5`) is uninterpretable. An end of `00:00`, `24:00`, `12am` or `midnight` means minute 1440. An end before the start wraps: `20:00-02:00` is `[1200,1440) ∪ [0,120)`. A range whose start equals its end (`00:00-00:00`) is uninterpretable, as is any weekday-qualified, seasonal, or descriptive schedule. If any range in a value fails, the whole value is uninterpretable.

### C. Per-area arithmetic
Minutes are the unit; the view renders them as hours. For each area, over facilities with **recorded** hours only:
$$\text{open\_minutes\_covered} = \left|\{\,m \in [0,1440) : \exists f,\ m \in \text{open}(f)\,\}\right|, \qquad \text{minutes\_with\_none\_open} = 1440 - \text{open\_minutes\_covered}$$
Both are absent (`null`, shown as "not recorded") when the area has no facility with recorded hours, including an area with zero facilities; they are never 0 or 1440 by default. The area row also reports counts of public restrooms, with recorded hours, with uninterpretable hours, and with hours not recorded, so a reader can see how much of the area the time figures describe. A facility whose hours are uninterpretable or not recorded may be open during a minute counted as having none open.

### D. What the view does not claim
It does not state that a facility was actually open, that a closure occurred, or that any commitment was broken. A comparison against a documented service commitment would be a separate, counted evaluation; none is performed here.

## 9. Budget Line Comparisons

`analysis/budget_lines.rs`, read by `DuckDbRepository::budget_line_data` (`storage/financial.rs`) and run by the `budget_line_comparison` detector.

### A. What is read
The mapped `budget` and `actual` roles are grouped by agency, currency, fiscal year, fund, department and budget basis (and by account for overtime lines), with the same expense-account filter the financial investigation applies to the City exports. Only `adopted` budget rows, and budget rows with no basis, are read. A group whose amounts do not all parse sums to NULL. Each read is capped at 10,000 grouped rows (`BUDGET_LINE_ROW_CAP`).

### B. The three measures
All in fixed-point arithmetic (four decimal places); percentages are computed only against a positive reference.
* **Department year over year** (`DEPARTMENT_SPENDING_YEAR_OVER_YEAR`): actual spending of one department in one fund, FY \(y-1\) to FY \(y\), for every year pair the fund's actuals contain. Finding when \(|\text{observed} - \text{prior}| \times 10000 \ge 1000 \times \text{prior}\) (`YEAR_OVER_YEAR_FINDING_BASIS_POINTS = 1000`).
* **Overtime against its adopted line** (`OVERTIME_ABOVE_ADOPTED_LINE`): an account is an overtime line when its label contains the word `overtime` in both tables and no label of that account names anything else. Actual spending on the account is compared with its adopted rows for the same fiscal year, fund and department.
* **Fund against adopted** (`FUND_ACTUAL_ABOVE_ADOPTED`): a fund's actual spending for a year against its adopted amount.
* The two adopted comparisons report when \((\text{observed} - \text{adopted}) \times 10000 \ge 500 \times \text{adopted}\) (`ABOVE_ADOPTED_FINDING_BASIS_POINTS = 500`). Both thresholds are inclusive and compared exactly, so 9.9999% is not 10%.

### C. Unreconciled, never a number
A comparison is unreconciled, with no value and no finding, when: the role is not mapped; its snapshot is partial or has rejected rows; a read was truncated; any row of the role names no fiscal year, fund, agency or currency; a row in the compared fund and year names no department (department and overtime measures); a budget row states no basis; an amount is missing or unparsable; either side has no rows (a department absent from one year, a fund with no adopted rows); the reference total is zero or negative; the department or account label column is unmapped; or an account's labels disagree within or across the tables.

### D. What a finding does and does not claim
* Every kind is a pattern signal (`is_legal_claim` is false). Adopted is the original appropriation, not the revised legal authority; the test of spending against revised authority is a separate question these kinds do not answer.
* Confidence: 9000 basis points for the fund and department comparisons (`LEDGER_CONFIDENCE`), withholding the rest because a fiscal-year export does not certify a closed ledger; 8000 for overtime (`OVERTIME_CONFIDENCE`), whose line is identified by the agency's own account label.
* A finding cites no document; its explanation names each dataset, its source hash, and the query ids of the grouped reads.

## 10. Encampment Abatement Notice Timing

`analysis/detectors/encampment_notice.rs`; findings carry `ENCAMPMENT_NOTICE_SHORTFALL` (legal claim) or `ENCAMPMENT_NOTICE_STATED_SHORTFALL` (pattern signal).

### A. What is extracted
Only documents with an encampment cue (`encampment`, `camp`, `homeless`, `unhoused`, `tent`, ...) are read. In a non-policy document, each date takes its role from the nearest token-phrase cue in the six words before it: **posting** (`posted`), **notice** (`notice`, `notified`, `served`), or **cleanup** (`cleanup`, `abatement`, `removal`, `vacate`, `sweep`, ...). A date with no cue has no role. The start is the earliest posting date, or the earliest notice date when no posting date is recorded; the end is the latest cleanup date. Both choices can only lengthen the interval. An end before the start is skipped. A **stated period** is a calendar-day period (`3 days`, `3 calendar days`, `three (3) days`) in a sentence with a notice or removal cue and no property-storage cue (`stored`, `claim`, `retrieve`, ...). Periods in business or working days are dropped because counting them needs a holiday calendar, and a guess could turn a compliant notice into a shortfall. Periods in hours (the common `72-hour notice`) are out of scope and are not read, so such notices yield no finding.

### B. The minimum notice period
* **Agency policy.** A document classified as a policy, of the same agency and portal as the notice, with a sentence holding an obligation (`shall`, `must`), a notice or posting cue, a lead cue (`before`, `prior to`, `advance`), a removal cue, no storage cue, and exactly one calendar-day period. The smallest such duty in the policy applies. When several policies apply, the one most recently in force on the start date is used. A policy that states no effective date is dated by its posting, and confidence is lower.
* **Stored authority.** A `legal_statutes` row whose own title or text has a notice cue and an encampment cue, with `mandated_deadline_days` recorded, is read as a minimum notice period in calendar days. On any other row, including the built-in CPRA determination deadline, that column means something else and is never used. The row applies only when its `jurisdiction` equals the portal's display name or its recognised source jurisdiction, and its effective window covers the start date. There is no jurisdiction hierarchy, so a state-level row does not apply to a city portal automatically. The detector never supplies a minimum of its own.

### C. What a finding does and does not claim
* `ENCAMPMENT_NOTICE_SHORTFALL` needs a recorded start and a recorded cleanup date fewer calendar days apart than the minimum. It is a counted comparison of the documents' stated dates. Confidence is 10000 only when the posting date, the cleanup date and the duty's effective date were all stated. It is 1000 lower when the notice date stands in for the posting date, and 1500 lower when the duty's effective date is not stated.
* `ENCAMPMENT_NOTICE_STATED_SHORTFALL` is a notice stating a period below the minimum when both dates are not recorded. It is one statement against another (`is_legal_claim` is false), starts at 5000, and is 1500 lower for an unstated effective date. It is 1000 lower again when no date in the notice places it in time, so which rule was in force rests on the document's upload.
* A missing date is reported as not recorded, never as zero. Dates are paired per document, so a log listing many sites yields its earliest start and latest end, which is conservative and usually no finding. Neither kind establishes when the notice was physically posted or the cleanup took place beyond what the documents state.

## 11. Meeting Calendar and Agenda Suggestions
`research::meetings` builds a calendar of council, board and commission meetings from documents already ingested. It makes no network or model call.

### A. What makes a meeting
* A public document is a meeting document when its heading (or, without retained text, its file name or description) names a publishing body from the `BODIES` table and names itself as an agenda or minutes (whichever word appears first; `non-agenda` is not an agenda), or is a video (`video/*` content type, or `video`, `recording`, `webcast` in its name).
* The date is read only from the heading: the text before the first `Item N:` heading, at most 12,000 characters. Every weekday-and-full-date phrase there must name the same date, and each weekday must match its date. One date is recorded, with a citation. No date, or disagreeing dates, yields `DATE_NOT_RECORDED` with a note saying which; a date is never taken from a file-name pattern, the body text, or another document. A minutes heading followed by an approval of earlier minutes that names a weekday date therefore reads as not recorded.
* Agenda, minutes and video are one meeting only when they share the publisher's meeting identifier (`meetingId` on the City's agenda host), or name the same body with the same heading date in the same source. A document with no recorded date is its own meeting.
* A meeting dated today or later is `UPCOMING`; earlier is `PAST`.

### B. Suggestions for upcoming meetings
Only complete agenda items are read: an item runs from its `Item N:` heading to the next one, so the last listed item, whose end is not proven, is not evaluated and a limitation says so. Every cue is a word-token phrase. A suggestion lists each reason with its basis and the agenda sentence it matched:

| reason | matched against | basis |
|---|---|---|
| Investigation topic | the research topics' terms, inflections spelled out | Stated |
| Watched agency / party | agencies in monitoring coverage expectations; responsible parties in casework commitments (names of two or more words) | Stated |
| Stored finding subject | the agency a non-dismissed finding is about | Counted when the finding kind is a legal claim, otherwise Pattern |
| Evidence gap | the record class an open gap needs | Stated; a gap is an untested route, never a finding |
| Stated figure | `extract_claims` over the item, evaluated as in §5 | Counted when compared (consistent or mismatch), Stated when unreconciled; a figure with no mapped dataset is not a suggestion |

Suggestions per meeting are ranked by the number of distinct reason kinds and capped at 12; a cap that drops items is recorded as a limitation.

### C. Public-comment drafts
Each suggestion carries an editable draft. It quotes only cited passages: the item, the matched agenda sentences, and for figures the stated figure and the observed table value. A stored finding is named by id and basis, never quoted, so a finding headline cannot become the commenter's assertion. The comment deadline and instructions are quoted only from agenda sentences that name a comment and a deadline (or a way to submit); otherwise the draft says "not recorded". The draft names its bases and states that it does not characterize anyone's motives or the lawfulness of any action.

## 12. Stated Response Times Checked Against Recorded Closures

`analysis/detectors/response_time_claim.rs`; findings carry the kind `CLAIM_CONTRADICTS_DATA` with the headline "Stated average/median response time …".

### A. What is extracted
A sentence in a public document (or an official statement) becomes a **response-time claim** when token-phrase cues name a metric (`average`, `mean`, `on average`, `median`), a records-request context, a response context (`response time`, `closed within`, `responds in`, …) and a figure in days (`10 days`, `eight days`). The hedge (`about`, `within`, `under`, `over`) and the period the sentence names (a calendar year, or a fiscal year read July to June) are recorded with it. A sentence stated in business days yields no claim: only calendar days are measured, and converting by a guess would put a number in the agency's mouth.

### B. What it is compared with
For the agency that answered the request the document belongs to, the portal's own request rows give each request's opened-at and closed-at. Latency is their difference in calendar days for rows where both instants parse and are in order. The average and the median are computed over requests opened in the stated period, or over every recorded closure when no period is stated (and the finding says which). Fewer than five closures is not a record of practice and yields nothing.

### C. Tolerance
The stated figure is held to ±10 % of itself, never less than half a day; `about` doubles the band; `within`/`under` and `over`/`at least` make it one-sided. Each metric is compared only with its own measurement: an `average` claim against the mean, a `median` claim against the median.

### D. What a finding does and does not claim
The finding states that the sentence's figure, at the precision it states, falls outside the closures the portal recorded for that agency and period, with the sample size and both measured figures. It is a pattern signal (`is_legal_claim` false). It does not decide which figure is right: the agency may count business days, a different start event, or requests the portal does not hold, and the finding says so. Confidence is derived from the closure sample size (+50 basis points per closure above five, to +2500) and from whether the sentence named its own period (+1000).

## 13. Redaction Consistency Across Releases

`analysis/detectors/redaction_inconsistency.rs`; findings carry the kind `REDACTION_INCONSISTENT_ACROSS_RELEASES`.

### A. Which copies are compared
Public documents in one portal with the same file name (trimmed, case-folded), released under different requests, whose extracted text differs. Up to six copies per name are compared pairwise in both directions; copies under different names are not paired, so a renamed release is not compared.

### B. How a divergence is found
The withholding copy's redaction spans come from the same structural detector the density metric uses (`[REDACTED]` labels, black-block runs, `withheld under … exemption`). For each span, the five words before it and the five after are matched in the other copy; when both contexts occur in order with one to forty words between them, and no redaction of the other copy's own overlaps that gap, the gap is a passage one copy redacted and the other released. Both passages are cited with their character ranges, so a reader can open each copy at the place.

### C. What a finding does and does not claim
The finding states that two releases made different redaction decisions about the same passage. It does not say which was right: a later release may apply an exemption the earlier one did not, a requester may have been entitled to more, or the copies may be different versions of the record. It is a pattern signal; confidence is derived from the number of diverging passages (5000 basis points plus 500 per passage, to six).

## 14. Plain-Language Layer
`src/content/plainLanguage.ts` gives every `FindingKind` and every `VerificationState` three short statements for a non-specialist: what was observed, what it does not show, and what to do next. The finding card and inbox row show them behind an "In plain words" disclosure (`PlainLanguageDisclosure`), collapsed by default.

### A. What it does and does not claim
* It explains a finding; it never classifies one. The basis sentence under the text is derived from `isLegalClaimKind`, the same source as the basis chip, so a pattern signal cannot be upgraded and a counted comparison cannot be softened. Confidence and citations are unchanged and remain visible.
* The tables are `Record`s over `FindingKind` and `VerificationState`, so a new variant fails `tsc` until it has text. `scripts/check-help-coverage.mjs` fails on the same omission, and on a key with an empty field, without a build.
* `src/tests/plainLanguage.test.tsx` rejects accusatory wording for every kind, rejects wording of violation or breach for a kind or state that is not a legal claim, and requires the `LatencyOutlier` explanation to say it is a comparison with peers.
* An evidence gap is an untested route. It is never explained as a finding and never as evidence of compliance or non-compliance.

## 15. Rolling Productions

`analysis/detectors/rolling_production.rs`; findings carry the kind `ROLLING_PRODUCTION`. Release events are stored in `request_release_events` (`storage/sqlite/release_events.rs`).

### A. What is stored
Every `Document(s) Released` and `Document(s) Released to Requester` event on a hosted NextRequest timeline (`ingestion/hosted_nextrequest.rs::decode_timeline`) becomes one row: the byline exactly as given, its wall-clock time at the US Pacific offset in force that day (the same conversion the opened and closed dates use), the portal's event id, and the file names the event lists, one per `<br>` line. The count of listed names is NULL when the event lists none: an event without a list is not a release of zero documents. A re-crawl replaces the request's rows. A timeline longer than `MAXIMUM_TIMELINE_PAGES` is read in part, so a stored history is a lower bound.

### B. What fires
An installment is a calendar date, in the portal's time, with at least one release event; several uploads on one day count once. A request with at least 3 installments whose first and last are more than 90 days apart yields one finding listing every installment date and the files listed on it.

The thresholds were measured on 142 sandiego.nextrequest.com requests matching "encampment" (timelines read 2026-10-03): 126 have a release event, 24 have two or more, 23 on two or more dates. Spans of the multi-date requests, longest first: 810, 558, 524, 479, 283, 184, 172, 160, 132, 74, 68, 47, 44, 34, 32, 28, 27, 21, 13, 12, 12, 7, 7 days. Two dates are most often a release and one supplement (25-1161: two dates, 160 days, does not fire); the 90-day threshold sits in the gap between 74 and 132, so a day's drift does not move a request across it. With both thresholds, 8 of the 142 requests fire (20-2226: 27 dates over 810 days, from 30 release events).

### C. What a finding does and does not claim
The finding restates recorded dates: the request's records were released on N separate dates over D days. It is a pattern signal (`is_legal_claim` false) with confidence unassessed, since nothing is inferred to score. It does not say a deadline was missed: records may be produced in stages, and the requester may have agreed to a schedule or changed the request. No release date is used as `determination_at`; the timeline does not record a determination.

## 16. Denials Checked Against Earlier Releases

`analysis/detectors/no_records_contradicted.rs`; findings carry the kind `NO_RECORDS_CONTRADICTED_BY_RELEASE`.

### A. Which denials are tested
A public document whose text states a plain denial ("no responsive records", "no responsive documents", "no records exist", "unable to locate any records", ...; the shared `extract_denial` pattern) on a request with a stated description or title. A denial limited to a year or fiscal year within 120 characters of the phrase is not tested: a release from outside that scope does not contradict it.

### B. Candidate recall
The on-device embedding model (`bge-small-en-v1.5`, unit vectors) scores every embedded document of the same portal by the cosine of its closest chunk against the denied request's description (the title only when there is no description; NextRequest titles are a truncated copy of it). The model proposes candidates and decides nothing.

### C. Deterministic checks
A candidate is cited only when all hold: cosine at or above `SIMILARITY_THRESHOLD`; public; same portal; a different request; that request was assigned the same `agency_id` as the denying request (exact id: a multi-department assignment is its own agency id and is not matched, and a request with no recorded agency matches nothing); its `created_at` is strictly before the denying request's `closed_at`, both parsed as RFC 3339 instants (an unparseable or absent date fails the check); and its own text is not a denial. `SIMILARITY_THRESHOLD` is 0.86: on the San Diego collection the highest cosine of any release judged not to contradict its denial was 0.8531 (see ROADMAP, "Findings on real collections"). It is a precision floor; recall is unmeasured.

### D. Not run
Without an installed embedding model, or with no embedded document, the detector reports not-run with the reason (`RefreshSummary::not_run`) and leaves its earlier findings in place. It never falls back to word overlap, which matched every San Diego denial to unrelated releases. The benchmark harness and `live_evaluation` run without the semantic index and so report this detector as not run.

### E. What a finding does and does not claim
The finding states that the agency had already released a document, before closing the request with a denial, whose text is near the request's own at the stated cosine, and cites the closest passage with its character range. Cosine measures closeness of subject and wording, not responsiveness, so the finding carries no confidence score (`Unassessed`) and is a pattern signal (`is_legal_claim` false). It does not show the search was inadequate: the release may fall outside what was asked.

## 17. Service-Report Leads

`src/utils/serviceAccountability.ts`, run by automatic discovery through the restricted SQL boundary and shown under What stands out. Verified natively by `tests/data_discovery.rs::service_report_checks_run_on_real_imported_rows`, which imports two copies of one source and a closed-reports table.

### A. Which tables are read
A table is treated as service reports when its columns include, by the publisher's own names, a service type (`service_name`, `request_type`, …), a close date (`date_closed`, …) and either an opened date or a recorded age (`case_age_days`, …). A status column, a place column and up to two area columns (a district and a community) are used when present. Other tables get no service checks.

### B. What is counted
* **Open** means no close date and, where a status exists, a status that does not name a referral. **Closed** means a close date and no referral. Referred reports are counted apart: they often carry no close date and have left the table rather than stayed open in it.
* **Age** is the publisher's recorded age when the table has one, otherwise the days from the opened date to the close date, or to the snapshot's retrieval date for an open report. A negative recorded age (the publisher's -999) is an unrecorded age: the report is still counted, its age is not.
* **Backlog**: open reports per service, how many are older than 365 days, and the earliest opened date. Reported at 25 or more reports older than a year.
* **Owner and budget**: where the table has a staff-group column (`case_record_type`, which the publisher's dictionary defines as the City staff group responsible for responding), the backlog check reports the group most of a service's open reports carry and how many carry it; a group is named only when more than half do. The group is a label, not a department code: of the nine values in the 2026-09-14 open file only "Parks & Recreation" is spelled as an operating-budget `dept_name`. A fixed table in `serviceAccountability.ts` reads TSW and TSW ROW as Transportation and Stormwater, Storm Water Code Enforcement as Stormwater, DSD as Development Services, ESD Complaint/Report as Environmental Services and Neighborhood Policing as Police, and the lead says "read here as"; Parking and Traffic Engineering are left unassigned and the lead says so. When a table with the operating budget's columns is imported, `service_budget` sums `amount` per department for the two latest fiscal years that have rows marked `adopted`, counting only accounts beginning with 5. The publisher's file also carries revenue under accounts beginning with 4 (Transportation, FY 25 adopted: 190.6 million expense, 185.2 million revenue), a `proposed` cycle, and a latest year (FY 26) with no cycle at all, which is not called adopted. A department with an unreadable amount, or with no row, gets no figure. The amount is the whole department's, not the service's.
* **Areas**: the median age of open reports, and the median days to close, per service and area against the median for that service across all areas. An area is listed with at least 30 reports, at least three areas reporting, and a median at least 1.5 times the all-area median (and, for closure, at least seven days slower). For open reports the all-area median must be 30 days or more, unless the area's own median is over 365 days: Encampment reports are a median 10 days old citywide and 904 in Council District 4 (158 open, 2026-09-14 copy). The two areas with the most reports older than a year are shown, plus up to two for services named encampment, homeless, shelter or restroom, which larger services would otherwise outrank. Area names are compared without regard to letter case, because publishers change case between years.
* **Repeated places**: ten or more open reports with the same service and the same recorded place text, the oldest open more than 90 days.
* **Referrals**: per service, reports whose status names a referral; reported where a service has at least 100, making up at least a quarter of its reports.
* **Recurring**: a service and area listed as slower to close in three or more imported tables.
* **Backlog change**: when the same source URI was imported on two retrieval dates, the count of open reports older than a year in the earliest and latest copies, with the three services that moved most. A report leaves an open file when it is closed or referred; the comparison does not say which.
* **Movement**: for the same two copies, where both have a report id (`service_request_id`, …): reports open in the earliest copy and not open in the latest (left), and reports open in the latest and not open in the earliest (arrived), per service. What became of a report that left is read from the latest copy and from up to eight other service-report tables with a report id retrieved on or after the earliest copy's date: a close date, a referral without a close date, or no record in any of them. When no table could hold such a record the lead says the outcome is not in the source, rather than reporting a count of "no record".

### C. Tables the app fetches for itself
`src/utils/dataGaps.ts` names a published table that would answer what the imported ones cannot, and `DiscoveryController` fetches it through `fetch_city_dataset`: one at a time, after every table is profiled, automatically only when Data upkeep is enabled with Recheck sources on (otherwise on "Fetch it now"), with a six-hour wait after a failure and the managed-file budget checked before the download. Three gaps are recognized, all for Get It Done:
* the latest copy of the open-reports file is seven or more days old: fetch the file again, which becomes a second copy (a different file is a new snapshot; the earlier one is kept);
* reports that left the open file have no close or referral record: fetch the closed-reports file for each year between the two copies (`GET_IT_DONE_OUTCOMES`). That file exceeds the import limit, so only rows whose `service_request_id` appears in an imported open copy are kept; the snapshot records the publisher file's digest and the filter. An Encampment-only subset, or a closed file retrieved before the latest open copy, does not close the gap;
* a backlog names a staff group read as a budget department and no table with the operating budget's columns is imported: fetch the City operating budget (`OPERATING_BUDGET`, one file of about 64 MB covering every published year).

### D. What a lead does and does not claim
A lead is a count of the publisher's records and carries no legal claim. An open report does not prove the problem persists, since the table records no inspections or work. A difference between areas can reflect what was reported as well as how it was handled. Several reports at one place can describe one fault, and a street name without a number can cover many blocks. Tables are compared as imported and can overlap. Every threshold above is a display threshold stated in the lead's method line.

### E. Leads stored as findings
Once every imported table has been profiled, `DiscoveryController` sends the feed's leads to `record_service_leads` (`analysis/service_leads.rs`), which stores each as a finding of kind `SERVICE_REPORT_LEAD` under the detector slug `service_report_leads`. The native side does not recount: it refuses a lead whose snapshot is not in the catalog, writes the source line (display name, file digest, retrieval date) from the catalog's own record, and stores the lead with no confidence score (`Unassessed`) and `is_legal_claim` false. The finding id covers the snapshot, its digest, the lead's key, title and detail, so changed counts make a new finding. Earlier leads that are not sent again are removed only when every imported table was checked without error; an unfinished or failed pass leaves them in place, and a dismissed lead stays dismissed. A set is sent once per change; a failed store is retried after five minutes. These findings name no portal agency, so `research::meetings` (§11) does not yet match them to agenda items.

## 18. Police Stop Leads

`src/utils/stopLeads.ts`, run by automatic discovery like §17 and stored as `POLICE_STOP_LEAD` findings (§17 E; `ServiceLeadInput.family` selects the kind). Verified natively by `tests/data_discovery.rs::stop_record_checks_run_on_imported_tables_with_the_publishers_headers`, which imports four tables with the publisher's upper-case headers.

### A. Which tables are read
A persons table is one with `id`, `pid` and `perceivedunhoused` columns (SDPD's RIPA stops file). Companion tables carry `id`, `pid` and one of `result`, `reasonforstopcodetext` or `basisforpropertyseizure`, and no `perceivedunhoused`; the latest imported of each is used. Persons are matched on stop and person number, compared as whole numbers because the tables import them with different types. Without a results table nothing is computed. `dataGaps.ts` asks for each missing companion (`RIPA_STOP_RESULTS`, `RIPA_STOP_REASONS`, `RIPA_PROPERTY_SEIZURE_BASIS`), and for the persons table (`RIPA_STOPS`) when an open Get It Done copy lists Encampment reports and no persons table is imported.

### B. What is counted
* **Groups**: `perceivedunhoused` true or false. Any other value (the publisher's older `0`) is a third group that is counted and left out of every rate.
* **Outcomes**: per group, persons; persons with a result naming a custodial arrest; of those on an outstanding warrant and without a warrant, each counted apart; persons with a citation or an in-field cite and release; persons with a property-seizure row; persons with no result row. A lead needs 100 persons in each group. The first stop date ignores dates before 2000 (the file has a year-23 entry).
* **Reasons**: the forty reason code texts with the most persons perceived as unhoused, with both groups' persons and arrests without a warrant for each. A reason is compared when both groups have 100 persons, and called higher at 1.5 times the other group's rate and at least three percentage points, tested by cross-multiplication. Warrant arrests are left out of this comparison because a warrant arrest follows from the warrant, not the stop.
* **Property**: persons perceived as unhoused per recorded basis for seizure.

### C. What a lead does and does not claim
Perception is the officer's entry, not housing status. A person can have several results and several reasons and is counted under each. Rates are not adjusted for where officers were assigned. A difference in rates is not a finding of unequal treatment, and the lead says so. The property table records a basis, not where property was kept or whether it was returned or discarded.

## 19. Abatement Log Leads

`src/utils/abatementLeads.ts`, run by automatic discovery and stored as `ABATEMENT_LOG_LEAD` findings (§17 E). Verified natively by `tests/data_discovery.rs::abatement_logs_pair_one_notice_with_one_abatement_in_the_released_format`, which imports two tables with the headers and the `7/1/2025, 8:00 AM` time format of the logs San Diego released to requests 25-5806 and 26-772.

### A. Which tables are read
A notice log has a posting-time column (`date_posted`, …), an address column and latitude and longitude columns. A completion log has a closing-time column (`date_time_closed`, …) and the same place columns, and no posting-time column. A table with service-report roles (§17 A) is neither. The check runs on a notice log when a completion log is imported, using the latest one. Released spreadsheets import as general tables whose types are inferred from values, so the time text is kept as recorded and parsed in the query.

### B. What is counted
Rows are keyed by the address text (trimmed, lower case) and the coordinates rounded to five decimal places. Hours from posting to closing are counted only at places with exactly one notice and exactly one abatement, so a notice is never paired with another notice's abatement: completed before posting, within 3 hours, within 24 hours, from 24 to 72 hours, 72 hours or more, and the median of the non-negative values. Also counted: abatements dated between the first and last posting at a place with no notice in the log, and notices with no coordinates or no readable time. A lead needs 30 paired places.

### C. What a lead does and does not claim
The logs record a posting time and a closing time. They do not record the notice period that applied, so 24 and 72 hours are yardsticks and the lead is not a finding that notice fell short; `ENCAMPMENT_NOTICE_SHORTFALL` (a notice document against a stated minimum) is the kind that makes that comparison. Notice may have been given by a route the posting log does not record, and two rows at one place can be different events. Neither log records whether anyone was present or what was done with belongings.


## 20. Arrest and Citation Log Leads

`src/utils/chargeLeads.ts`, run by automatic discovery and stored as `CHARGE_LOG_LEAD` findings (§17 E). Verified natively by `tests/data_discovery.rs::charge_log_checks_count_arrests_and_citations_not_charge_rows`, which imports a table with the headers and values of the logs SDPD released to requests 25-7080 and 26-6187.

### A. Which tables are read
A table with `activitynumber`, `activitytype` and `violationsection` columns. `activitydate` and `race` are read when present. Each row is a charge; rows sharing an activity number are one arrest or citation, and every count is of arrests and citations, not rows.

### B. What is counted
An arrest or citation is counted when any of its rows cites San Diego Municipal Code 63.0404 (encampment), 54.0110 (encroachment; also released as 54.011 because spreadsheets drop a trailing zero), Penal Code 647(e) (lodging) or Municipal Code 56.55 (urinating or defecating in public). Sections are compared in upper case with spaces removed, by prefix or exact value, so 647(F) is not 647(E). Counted: all arrests and citations in the file, those citing a listed section, of those arrests and citations, arrests in which every row cites a listed section, and the count per section. A lead needs 30. Where race is recorded, the twelve most frequent races among the counted are returned with their counts among the counted and among all; entries that are blank or the publisher's `NULL` are left out of both. The lead names at most three races, each with 30 of the counted, and none when the file holds only listed sections.

### C. What a lead does and does not claim
The log records a section, not housing status. The listed sections are also charged against people who are housed, and encroachment covers objects as well as people. The log does not record whether shelter was offered, where a restroom was open, or what became of the charge. The race comparison is with other arrests and citations in the same file, is not adjusted for where officers were assigned, and is not a finding of unequal treatment; the lead says so. Two released files can cover overlapping dates; each table is counted alone.


## 21. Release Timing

`src-tauri/src/analysis/detectors/release_timing.rs`, stored as `RELEASE_TIMING_CONCENTRATION` findings. Every rule below has a test in that file run against a fresh database.

### A. Which rows are read
Public rows of `documents` that have a `download_url` and belong to a portal whose `source_kind` is `NEXT_REQUEST`, joined to their request for its agency. `documents.created_at` does not mean one thing: it is the portal's upload time of the file (`document_scan.upload_date`), the request's opening time when the portal gave no upload date, the request's closing time for a closure notice, the moment the application first saw a local, pasted or open-data file, or the container's time for a file extracted from an archive. Only the first is an upload time. Closure notices and extracted files have no download route and files from other source kinds are not on a records portal, so none of them is read. A row that is read is left out, and counted as left out, when its time equals its request's opening time, has no time of day or no offset, is written in UTC with no local offset (`Z` or `-00:00`), or is exactly midnight (how a date without a time is stored). A date is never read as midnight. `request_release_events.released_at` is not used: it is per request, to the minute, and its Pacific offset is assumed by the ingester rather than recorded by the portal.

### B. What is counted
The unit is a release: the files uploaded to one request with less than 60 minutes between one file and the next, timed at its first file. The weekday, date and hour are taken from the stamp's own offset, so a change of daylight time needs no rule and nothing is converted through UTC. A release is late when it began at or after 3:00 p.m. on a working day whose next day is a Saturday, Sunday or California judicial holiday (`court_calendar`, 1990 to 2099; a release outside those years is left out). An agency is the pair of agency id and portal. For each agency: its releases, its late releases, the dates they fall on, and the same counts for every other release on the portal, including requests with no agency. Several late releases by the agency on one date count once in the comparison. A finding needs 20 releases so counted, 40 other releases, late releases on three dates, a share at least twice the others' share and at least ten percentage points above it; ratio and difference are tested by cross-multiplication. The finding states the counts, both shares, the first and last release date, and the files left out for the agency and the portal. It cites up to five files of the late releases. Severity is Info and confidence is not assessed.

For the files of the late releases (the 50 most recent) the internal modification time is read from the file at `documents.local_path`: PDF Info `ModDate`, else XMP `ModifyDate`, else XMP `MetadataDate`; OOXML `dcterms:modified`. The finding counts files modified less than a day, one to six days and seven or more days before upload, and each citation states its file's figure. A modification time later than the upload time is counted and labelled as a contradiction between the file and the portal's clock, not as an age. A file with no readable internal time is "not recorded". A PDF time written without a zone is read as UTC, so an age can be off by the zone difference.

`held_back_response` and `regenerated_response` now time a file by the same upload time when it has one, and by the request's `determination_at` as before when it does not; the explanation states which was used. Their candidate sets and thresholds are unchanged.

### C. What a finding does and does not claim
It is a count of upload times against the same portal's other uploads. It does not say why they fall where they do: staff often finish and post work at the end of a week or before a holiday, and requests that fall due then are released then; the finding says so. The portal records when a file was uploaded and not when a file uploaded earlier was made public. The holiday list is California's; days proclaimed for one occasion are not modeled, and an agency in another state keeps a different calendar. The 3 p.m. start is the last two hours of an 8-to-5 day; on the collection in ROADMAP "Cross-source accountability build" 18% of releases began at or after it. No request draft is offered.
