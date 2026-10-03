# Data sources for the San Diego homelessness and accountability investigations

Researched and fetched 2026-10-03. Detail files: `city-open-data.md`, `homelessness-system.md`,
`records-requests.md` (existing NextRequest releases, and draft CPRA requests for what is not public).

## Verified locally (downloaded and queried)

- **SDMC 63.0404 enforcement is in public NIBRS data.** `https://seshat.datasd.org/police_nibrs/pd_nibrs_{YYYY}_datasd.csv`,
  `code_section` "63.0404(a) SDMC Unauthorized Encampments on Public Property" / "63.0404(c) ... Within
  Prohibited Areas", with `occured_on`, beat, division, block address, latitude/longitude. Incidents: 2023
  (from Aug) 20, 2024 225, 2025 163; almost all (c), 32 (a); all but one geocoded. These are reported
  incidents, not every citation (the City Auditor's OCA-26-07 totals are higher).
- **Get It Done encampment reports**: `https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_closed_{YYYY}_datasd.csv`,
  `service_name = 'Encampment'`, 41,222 closed reports in 2025.
- **Already released on sandiego.nextrequest.com** (confirmed public): 26-772 "Abatements Completed 7.31.23 to
  1.28.26.xlsx"; 25-9260 "PRA 25-9260_ArrestsCites_SDPD.xlsx". More in `records-requests.md`.
- RIPA stop results do not carry 63.0404 (state codes only; CA REG camping codes appear).

## By investigation

| Need | Public now | Must be requested |
|---|---|---|
| 63.0404 enforcement time/place | NIBRS incidents; SDPD arrestee/cite logs via NextRequest (to 2025-07) | Full citation log after 2025-07 |
| Shelter bed availability at enforcement time | none (annual HIC only) | SDHC daily availability emails to SDPD (City says it holds none; SDHC has its own portal) |
| Prohibited-zone audit | NIBRS coordinates; schools, transit stops, beats layers | Shelter locations as data |
| 24-hour notice window | Get It Done encampment reports; abatements completed list | ESD notice-posting timestamps |
| Impounded property | Impound logs as PDFs (to 2025-10) | Intake manifests per abatement |
| HMIS / CES / vouchers | Countywide CSV/XLSX only | Provider- and district-level outcomes |
| Sub-recipient admin vs direct cost | Award PDFs | Contract budgets by line item |
| NPD saturation / overtime | Calls for service (daily, beat-level); budget PDFs | NPD deployment hours; overtime by event |
| CPRA delay and exemptions | NextRequest timelines and closure notices (collected by the app) | — |
