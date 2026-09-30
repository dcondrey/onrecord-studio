# Claim validation eval set (City of San Diego, FY 2022–2023)

Real rows from the City of San Diego's operating budget and actuals exports, sliced to the
General Fund (fund 100000) for the Library and Police departments in report years 22 and 23,
with hand-written memo sentences whose expected outcome was verified against those rows.

## Provenance

| file | source | retrieved | full-export SHA-256 |
|---|---|---|---|
| `budget_operating_datasd_fy22-23_gf_library_police.csv` | https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv | 2026-09-30T21:00Z | `4ad8a734cb351a7ccae7939ea9c8076cf7e30a4d4f2a8c10f45852cb042c5fe5` |
| `actuals_operating_datasd_fy22-23_gf_library_police.csv` | https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv | 2026-09-30T21:00Z | `53cfc7488c40c2d3b654c64ba8599bc11a4086fa5efe403371c38eff78dd2143` |

Slice: `WHERE report_fy IN ('22','23') AND fund_number='100000' AND dept_name IN ('Library','Police')`,
columns and values untouched. Because only two departments are present, every case names its
department: a fund-level claim would bind to the slice's partial total, not the City's.

## Verified totals (whole dollars)

Expenditure accounts are `5xxxxx`, revenue accounts `4xxxxx`, the same filters the mapped roles apply.

| year | department | adopted budget (5) | actual (5) | actual revenue (4) |
|---|---|---|---|---|
| 22 | Library | 60,741,326 | 61,226,563 | 1,883,049 |
| 22 | Police | 593,292,386 | 604,197,963 | 45,157,480 |
| 23 | Library | 66,823,341 | 67,586,967 | 3,194,254 |
| 23 | Police | 584,706,166 | 588,906,166 | 47,805,183 |

`cases.json` records, per sentence, the expected evaluation and the arithmetic behind it. The
harness is `claim_validation_eval_set` in `tests/detector_benchmark.rs`.
