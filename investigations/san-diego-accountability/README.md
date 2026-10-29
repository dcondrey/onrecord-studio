# San Diego homelessness accountability investigation (2026)

The reports, evidence tables and a small set of sample sources from the investigation that
OnRecord Studio was built around: San Diego's homelessness spending, encampment abatement,
shelter capacity, police overtime and contract performance, read from the City's public records,
open data and public-records portal releases between September and October 2026.

The full working corpus (about 24 GB of portal releases, extracted text, intermediate analysis
and build artefacts) is not kept in this repository. What remains here is what a reader needs to
check the published findings, plus enough original material to try the application.

## Published editions

| Edition | Where |
|---|---|
| *How Help Is Offered Matters: San Diego Homelessness Accountability*, SDHA-2026-01, version 1.2 | Zenodo, https://doi.org/10.5281/zenodo.23405739 |
| Report, Council brief and selected evidence (2026-10-15) | https://github.com/dcondrey/onrecord/releases/tag/sd-accountability-2026-10-15 |
| Version 1.3 (evidence cutoff 2026-10-18) | `reports/` in this directory; not deposited separately |

The source data behind the reports is published as nine public datasets under
`Writerslogic/onrecord-*` on Hugging Face and is refreshed weekly by
[onrecord-dataset-updater](https://github.com/dcondrey/onrecord-dataset-updater). See
[`docs/data-packs.md`](../../onrecord-studio/docs/data-packs.md) for the pack design.

## Contents

### `reports/`

| File | What it is |
|---|---|
| `San-Diego-Homelessness-Accountability.pdf` | Main report, version 1.3, 92 pages |
| `Council-Decision-Brief.pdf` | Two-page decision brief for Council |
| `Follow-the-Charge.pdf`, `Funded-Handoff.pdf`, `Outcome-Numbers.pdf` | Standalone reading editions of three report series |
| `San-Diego-Council-Evidence-Update.pdf`, `Council-Decision-Memo-Updated.pdf` | 2026-10-16 evidence update joining portal releases to the earlier claims |
| `San-Diego-Evidence-Brief.pdf`, `San-Diego-Public-Briefing.pdf` | 2026-10-12 evidence brief and 13-slide public briefing |
| `Unsent-Records-Request-Drafts.pdf` | Follow-up records requests drafted during the work; none were sent |

The main report, Council brief and the three standalone editions carry C2PA Content Credentials.
The certificates and verification record are in `evidence/Content-Credentials/`.

### `evidence/`

| File | What it is |
|---|---|
| `Evidence-Ledger.xlsx` | Findings, limits, public source links, model inputs and the FY2025 invoice ledger |
| `Evidence-Claims.json`, `Evidence-Claims.csv` | Every claim with its evidence class, source identifiers and page locators |
| `Completed-Abatement-Register.csv`, `Completed-Abatement-Ranking.csv` | Encampment abatement register reconstructed from released logs |
| `Research-Coverage.json` | Search scope, access failures, exclusions and pagination corrections |
| `source-catalog.json`, `source-manifest.json`, `Source-Index.html` | Cited sources with public URLs, retrieval dates and SHA-256 digests |
| `Citation-Links.json`, `Delivery-Manifest.json`, `Citation.txt`, `License.txt` | Release metadata for version 1.3 |
| `Evidence-Workbook.xlsx`, `impound-log-screen.csv`, `police-budget-and-actuals.csv` | Workbook and tables behind the 2026-10-12 brief |

Every claim carries one evidence class: verified original, reproduced measurement, agency
assessment or allegation, inference from linkage, inference from absence, or process reading.
Negative results and unverified screens were kept out of the claims.

### `samples/`

Five public City documents cited by the reports, kept as demo input for the application:

| File | Document |
|---|---|
| `BK01.pdf` | FY2026 third-quarter budget monitoring report |
| `BK05.pdf` | Performance audit of Police Department overtime (OCA 24-08) |
| `BK06.pdf` | Police training bulletin on overtime reporting, March 2025 |
| `BK07.pdf` | FY2026 mid-year budget monitoring report, including the Police appropriation memo |
| `BK09.pdf` | Charter section 39 report for the period ending June 2026 |

## Personal information

The audit population is institutional: officials, vendors, contract amounts, agreement numbers,
facilities and departments. The evidence package omits personal names, item descriptions and
contact details from released logs, and republishes no private information even where an agency
release failed to redact it. Portal releases are linked by source URL rather than copied.

## Licence

Original report text, figures and analysis are licensed CC BY 4.0 (see `evidence/License.txt`).
Cited public records keep their own status; no rights are asserted in public-domain material.
