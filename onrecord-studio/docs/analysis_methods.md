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

## 12. Plain-Language Layer
`src/content/plainLanguage.ts` gives every `FindingKind` and every `VerificationState` three short statements for a non-specialist: what was observed, what it does not show, and what to do next. The finding card and inbox row show them behind an "In plain words" disclosure (`PlainLanguageDisclosure`), collapsed by default.

### A. What it does and does not claim
* It explains a finding; it never classifies one. The basis sentence under the text is derived from `isLegalClaimKind`, the same source as the basis chip, so a pattern signal cannot be upgraded and a counted comparison cannot be softened. Confidence and citations are unchanged and remain visible.
* The tables are `Record`s over `FindingKind` and `VerificationState`, so a new variant fails `tsc` until it has text. `scripts/check-help-coverage.mjs` fails on the same omission, and on a key with an empty field, without a build.
* `src/tests/plainLanguage.test.tsx` rejects accusatory wording for every kind, rejects wording of violation or breach for a kind or state that is not a legal claim, and requires the `LatencyOutlier` explanation to say it is a comparison with peers.
* An evidence gap is an untested route. It is never explained as a finding and never as evidence of compliance or non-compliance.
