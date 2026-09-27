# San Diego investigation validation — September 26, 2026

This is a record of findings checked against published sources during development. It distinguishes independently reproduced calculations from the application's live integration results. Amounts below are investigative leads, not conclusions about unlawful spending or ineffective services.

## Reconciled spending leads

The complete [City operating budget export](https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv) contains 548,811 rows. The complete [actual expenditures export](https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv) contains 663,858 rows. These were independently read using Python CSV and Decimal arithmetic, excluding non-General-Fund rows, nonexpense accounts, and budget rows not explicitly labeled adopted. Their file digests and sizes are in [finance-source-references.json](finance-source-references.json). The original validation downloads were deleted after analysis.

| Comparison | Original adopted budget | Actual spending | Difference |
| --- | ---: | ---: | ---: |
| FY2023 Police overtime | $40,162,474 | $50,818,708 | +$10,656,234 |
| FY2023 entire Police department | $584,706,166 | $588,906,166 | +$4,200,000 |
| FY2022 Police overtime | $30,678,213 | $40,906,497 | +$10,228,284 |
| FY2023 Homelessness Strategies and Solutions | $25,157,607 | $22,920,122 | −$2,237,485 |
| FY2022 Homelessness Strategies and Solutions | $18,071,824 | $9,815,698 | −$8,256,126 |

The overtime difference is substantially larger than the department's net difference. Department totals obscure the distribution of spending. A useful next step is to reconcile adopted and revised appropriations, vacancies and salaries, overtime purpose codes, reimbursements, and transfers within the same fiscal periods. Overtime can reflect vacancies, special events, emergencies, or reimbursable assignments; these exports alone do not settle which explanation applies.

Lower homelessness department spending requires a different test: obtain the project-level ledger, encumbrances, carryforwards, transfers to SDHC or other departments, and matching service deliverables. A difference from original adopted funding does not establish that money was available for another use or that services were withheld.

### A consequential coverage gap

The actuals export ends at FY2023. The budget export includes FY2026, but all 32,596 FY2026 rows have an empty budget-cycle label. A 2026 website update date does not establish 2026 spending coverage, and an empty label cannot establish an adopted appropriation. The latest explicitly adopted year in this export is FY2025.

Request the existing FY2024–FY2026 actual expenditure exports, revision/close status, current budget-cycle documentation, and adopted-to-revised appropriation bridge before making present-day allocation-versus-spending claims. Keep this gap attached to every historical comparison.

## Immediate Council follow-through

The [September 28 special Council agenda](https://www.sandiego.gov/sites/default/files/2026-09/9-28-26-special.pdf), item 605, proposes a first amendment to the County subrecipient agreement for the Riverbed Encampment Resolution Funding Project. It authorizes acceptance, appropriation, and expenditure of $3,666,572 and extends the agreement to June 14, 2027.

A focused action is to review the amendment's scope, amended budget, milestones, existing performance reports, and funding conditions before commenting. Ask how the amendment changes promised services and how permanent placements and sustained housing outcomes will be reported. After the meeting, retrieve the vote, adopted resolution, executed amendment, and subsequent performance reports. Do not describe proposed authorization as money already spent or as a demonstrated outcome.

Check the [City Clerk's current agenda page](https://www.sandiego.gov/city-clerk/officialdocs/legisdocs/brown-act) again before participating because agendas can be revised.

## Application verification

Both live native investigations passed. They used temporary databases and did not replace the installed application's records.

- **Finance:** the native ingestion queue imported all 1,212,669 rows from the two official CSV exports with zero rejected rows. Executed comparisons, two adaptive retrieval/query passes, synthesis, and independent critical review produced six specific findings and six records-request drafts. Both originals were released after verification, freeing 132,929,740 bytes; the imported data remained queryable. See the [readable finance dossier](live-finance-dossier.md), [full JSON evidence and SQL](live-finance-dossier.json), and [comparison results](finance-comparisons.json).
- **Homelessness and Council follow-through:** the queue ingested the FY2024, FY2025, and FY2027 IBA reports, State Auditor report 2023-102.2, and the September 28, 2026 special Council agenda. Two adaptive research passes, synthesis, and independent critical review used four model calls and completed in 425.98 seconds. The result contained five concrete findings, six request drafts, and a date-and-item-verified action for Council Item 605. Three findings cite multiple distinct source records. Every retained quote passed validation; no final component needed to be withheld. All five originals were released, and retained evidence remained searchable. See the [readable investigation and requests](live-research-dossier.md) and [full JSON with source hashes and character locators](live-research-dossier.json).

The homelessness case identified these specific lines of inquiry:

1. The FY2024 and FY2027 homelessness funding tables explicitly omit CleanSD encampment-abatement and Police Neighborhood Policing Division costs. Together with the State Auditor's citywide-accounting finding, this supports a targeted request for the cross-department ledger and reconciliation rules, rather than treating the annual homelessness table as the whole cost.
2. The FY2025 $8.0 million Affordable Housing Fund contribution sits between an earlier documented structural funding deficit and FY2027 service reductions. The case seeks the transfer authority, restrictions, displaced housing uses, and recurring-funding plan; it does not claim the transfer caused later cuts.
3. The Kettner/Vine proposal paired a limited FY2025 budget allocation with an unexecuted lease and a much larger possible annual operating cost. The next test is the decision and obligation trail, not a claim that the hypothetical cost was spent.
4. The FY2027 report documents a $750,000 public-restroom closure line, but the available reports cannot establish whether locations, operating hours, or sanitation services were replaced through another department or grant. The case requests the location-level service and funding records needed to answer that question.
5. The State Auditor's 26% permanent-housing-exit benchmark versus 6% actual result is paired with SDHC's described monitoring process. The case seeks the specific contract and documented renewal/remediation decisions. The supplied excerpt does not identify the provider, so the report does not invent one.

Source-fidelity checks are described in [research-source-validation.md](research-source-validation.md). They do not establish causation, legal noncompliance, or the truth of an interpretation solely because a quotation matches.
