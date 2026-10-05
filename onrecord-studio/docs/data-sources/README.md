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

## Analyses run against the data (2026-10-03)

Ordinance text: SDMC Ch. 6 Art. 3 Div. 4 (https://docs.sandiego.gov/municode/MuniCodeChapter06/Ch06Art03Division04.pdf).
63.0404(a) public property generally; (c) prohibited areas "at all times regardless of shelter":
(c)(1) within two blocks of a K-12 school and (c)(2) of a shelter, (c)(4) transit hubs/trolley platforms
or tracks, (c)(5) City-Manager-designated parks — each only where signs are posted; (c)(3) open space,
waterways and banks. 63.0405(b): no criminal citation under 63.0404(a) between 9:00 p.m. and 5:30 a.m.
or when no shelter is reasonably available. 63.0406(a): written notice of clean-up at least 24 hours ahead.

- **63.0405(b) night-hours rule — no violation found.** SDPD "ArrestsCites" log released in request
  25-9260 (document 55556769, 153,862 rows, 2023-06-28 to 2025-06-27, columns activityType, activityDate
  with time, violationSection, address, beat): 29 citations and 5 arrests under 63.0404(A), none between
  21:00 and 05:30. One night-time citation is coded "63.0404" without a subsection. (c) accounts for 359
  citations/arrests in the same log.
  A later release extends this: "CPRA26-6187_Arrests_2025-June2026_SDPD.xlsx" (sandiego.nextrequest.com
  document 67231643, SHA-256 f81590df…c66; 118,550 charge rows, 76,291 CITATION and 42,259 ARREST,
  2025-01-01 00:02 to 2026-06-22 23:48, same columns plus Command). 63.0404 activities: 14 (A), 251 (C),
  3 coded "63.0404" alone, and none between 21:00 and 05:30; one (C) arrest is recorded at exactly 05:30
  (activity 2504585401, 1600 El Prado). NIBRS 2025 alone (154 offenses after de-duplicating `nibrs_uniq`):
  (c) 42 within 1,320 ft of a school or MTS/NCTD station, 56 at 1,320–2,640 ft, 45 beyond, 1 not geocoded;
  (a) 3 / 4 / 3. The app's report and an independent script agree on every count.
- **63.0404(c) locations — inconclusive without designations.** 374 geocoded NIBRS (c) incidents
  (2023-08 to 2025): 130 within 1,320 ft of a school (SANDAG Schools layer) or MTS station (GTFS
  location_type 1); 105 beyond 2,640 ft of both, concentrated in Balboa Park (El Prado, Park Blvd),
  Mission Bay (Ingraham St, Sea World Dr) and Golden Hill — plausibly (c)(3)/(c)(5). Testing them needs
  the City Manager's (c)(5) park designations and the ordinance sign locations (City Auditor OCA-26-07
  Appendix B lists sign locations as a City dataset) — request both.
- In the app: Analysis → Analytical Import → "Fetch and import" downloads NIBRS offenses, calls for
  service (2015 on) or Get It Done encampment reports for a year, SANDAG schools, or SANDAG transit
  stops. Delimited files are streamed (up to 1 GB and 20 million cells; calls for service 2024, the
  largest year, is 970,649 rows x 14 columns). Get It Done closed-year files are kept as a subset by
  choice: `service_name = 'Encampment'`, or "closed reports (those seen open)", the rows whose
  `service_request_id` appears in an imported open copy; the publisher file's SHA-256 is stored in the
  snapshot's validation summary. "Encampment ordinance observations" runs both checks above over
  whatever is imported.
- NIBRS `occured_on` is a date only; `approved_on` is report approval time, not incident time, so
  time-of-day tests must use the ArrestsCites log.
