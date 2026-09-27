# Financial investigation depth — September 27, 2026

The earlier financial workbench could explain individual differences, but it did not test whether one account's apparent shortfall was offset elsewhere or whether recent growth concealed a longer implementation problem. The new production analysis makes those connections before ranking isolated variances.

## Findings reproduced from the archived City query rows

The [native replay artifact](financial-depth-replay.json) contains the derived findings, arithmetic, exact original result rows, executed SQL and original source references. These are new deterministic hypotheses from preserved evidence, not a fresh download or a conclusion about delivered services.

| Connection | Exact result | Useful next test |
|---|---|---|
| FY2023 Homelessness center `1716000011`, accounts `512059` and `512066` | One account is $10,563,304 below adopted; the other is $8,767,029 above. The opposite movement offsets 82.9951% of the larger difference. The two-account net is −$1,796,275. | Obtain journal entries, transfers, revised appropriations and contract/program identifiers for both accounts. Determine whether this is an accounting shift, different services, or timing. |
| FY2023 Parks center `1714131301`, accounts `512172` and `512059` | Differences of −$6,948,545 and +$5,810,833 leave a two-account net of −$1,137,712. | Reconcile the agency-service agreements with professional-service invoices before treating either isolated difference as a service reduction or excess spending. |
| Homelessness General Fund history, FY2021–FY2023 | Actual spending rebounded by $13,104,424 in FY2023 but remained $19,858,922, or 46.4220%, below FY2021. | Trace expiring grants, temporary emergency services and moved programs to their receiving ledgers; compare service capacity on a consistent basis. A latest-year increase does not establish restoration. |
| Commission on Police Practices, FY2022–FY2023 | Adopted funding increased $1,223,218 while actual spending increased $300,402. Only 24.5583% of the added allocation appears as additional actual spending. | Obtain quarterly authorized/filled positions, hiring and launch milestones, revised funding, commitments and work completed. Test implementation capacity and later catch-up. |

The first two findings concern specific account pairs. They do not describe the entire department or establish that money transferred. The history findings compare the same named agency, department, fund, currency and funding duration; organizational changes and one-time funding remain explicit alternative explanations. Fiscal-year labels do not prove that a ledger is final.

## Production behavior

- Opening Spending and outcomes automatically investigates a valid mapped budget/actual default. The default prefers a homelessness department when present. Saved source mappings trigger recalculation against the current scope; the investigator can still change the scope explicitly.
- Two bounded history queries independently aggregate the selected fiscal year and its two predecessors. Account-detail truncation cannot silently become a departmental total. Missing amounts, missing years, incomplete snapshots and incompatible jurisdictions or funding durations prevent unsupported comparisons.
- Offset candidates use the same cost center and three-digit account family, require opposite movements with at least 70% overlap and materiality of at least 10% of the pair's adopted amount, and retain one candidate per group. The panel discloses these discovery filters.
- Capacity and rebound tests have explicit thresholds and require consecutive years. They produce explanations and records to acquire, not statistical or legal conclusions.
- Each finding carries exact calculations, citations, competing explanations and a specific next test. Findings and calculations appear above the account tables; targeted records also appear in Missing evidence.
- The existing autonomous context includes these deterministic leads and query rows. The researcher must execute the SQL again before treating the lead as supported evidence. The final masked JSON, including its SQL dictionary, stays below 20 KB. Optional omissions are counted explicitly, and a retained lead keeps every supporting row and query. Context is stable between unchanged runs.

## Separate primary-source acquisition lead

The City's [FY2023 year-end performance report](https://www.sandiego.gov/sites/default/files/fy23-year-end-performance-report.pdf), page 9 and Attachment 7, reports a settlement recovery for Convention Center portable-shower overcharges in FY2020–FY2021; Attachment 7 identifies approximately $942,000 received. Useful follow-up records are the settlement agreement, case identifier, shower contract and amendments, disputed invoices, overcharge calculation, and recovery posting.

Attachment 7 also reports $7.6 million in homelessness grant invoices deferred into FY2024. This is important timing counterevidence, but those grant amounts cannot be subtracted from the General Fund comparisons above. These narrative records require acquisition and reconciliation; they are not represented as facts established by the archived ledger replay.

## Verification

The financial suite now contains 16 native tests. The existing real-source replay verifies all four derived connections above and preserves its previous missing-counterpart assertions. Additional tests reject mixed agencies, funds, centers, account families, currencies, funding durations, missing periods and partial sources. A storage integration test runs the independent history SQL, replays cited rows, checks autonomous context and verifies that partial data withholds the deeper findings.

All 16 native financial tests pass, including the real archived replay and final serialized-context bound. TypeScript, targeted ESLint and all six financial frontend tests pass. The automatic-startup scope-change test fails when the report continuation guard is removed, and passes after restoration. The earlier scenario-input and native source-identity mutation proofs remain applicable.
