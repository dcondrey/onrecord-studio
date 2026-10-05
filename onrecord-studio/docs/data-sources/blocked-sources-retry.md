# Sources that refused the first fetch: second attempt

Retried 2026-10-04 with a plain HTTP client (no browser imitation). Where a publisher's edge still
refuses, the routes left are a person's browser, the Internet Archive's copy, or a records request.

## Now reachable

| Source | How | What was seen |
|---|---|---|
| County fictitious business names | The County's own download is `https://arcc-acclaim.sdcounty.ca.gov/Fbn/DownloadCsvFile` (page: `/FBN/FbnDownload`). The host refuses non-browser clients (Akamai 403), so the file was taken from the Internet Archive: `http://web.archive.org/web/20250604050356id_/https://arcc-acclaim.sdcounty.ca.gov/Fbn/DownloadCsvFile`. Archive captures: 2021-11-28, 2024-06-27, 2024-08-25, 2025-06-04 (full), 2025-06-13 (partial) | Line 1 is the County's disclosure statement; header `FilingNumber, BusinessName, FilingDate, ExpirationDate`. 2025-06-04 copy: 341,699 rows, 267,186 filing numbers, filed 2016-01-04 to 2025-06-02 (the County serves about nine years). One filing can list several business names. No owner name or address: those are on the filing itself, searched one at a time on the same site. Copy kept at `/Volumes/A/onrecord-eval/sources/sd-county-fbn-archived-2025-06-04.csv` |
| Port of San Diego agendas | `https://pub-portofsandiego.escribemeetings.com/` returns HTML (330 KB) with `Meeting.aspx?Id=<guid>` links; calendar methods `MeetingsCalendarView.aspx/GetCalendarMeetings` and `/PastMeetings` | meeting links seen; item and attachment layout not yet read |
| Homeless student enrollment (CDE) | `https://www3.cde.ca.gov/demo-downloads/homeless/hse2425.txt` (14.4 MB, tab-separated) | header: Academic Year, Aggregate Level, County Code, District Code, School Code, County Name, District Name, School Name, Charter School, DASS, Reporting Category, Cumulative Enrollment, Homeless Student Enrollment, Temporarily Doubled Up, Temporary Shelters, Hotels/Motels, Temporarily Unsheltered, Missing/Unknown, and percentages |
| USAspending | `POST https://api.usaspending.gov/api/v2/search/spending_by_award/` with `recipient_search_text: ["<UEI>"]`, `award_type_codes: ["02","03","04","05"]` | returns awards for the Housing Commission (e.g. ROSS252021, 272,250 dollars, listing 14.870) |
| OpenJustice files (CA DOJ) | `https://data-openjustice.doj.ca.gov/sites/default/files/dataset/2026-07/DeathInCustody_2005-2025.xlsx` | file served (range request returned the workbook type) |
| Transparent California | `https://transparentcalifornia.com/salaries/2024/san-diego/` returns HTML | page served; no CSV link in the HTML, export is behind a form |

## Still refused

| Source | Response | Route left |
|---|---|---|
| County Recorder site (FBN search by name, official records index) | Akamai "Access Denied" to non-browser clients | browser; the archived CSV above for the index |
| Sheriff pages and published documents (`sdsheriff.gov`) | Akamai "Access Denied", including direct document URLs | browser, or the state's agency-level files (OpenJustice) for the same subjects |
| County Water Authority (`sdcwa.org`) | CloudFront "Request blocked" | browser |
| State Controller public pay (`gcc.sco.ca.gov`, `publicpay.ca.gov`) | Cloudflare challenge page | browser download of the yearly city file; Transparent California as a secondary copy |
| Superior Court index (`courtindex.sdcourt.ca.gov`) | Cloudflare challenge page | browser; no bulk data exists |
| HUD national inventory workbook (`huduser.gov/.../2007-2025-HIC-Counts-by-CoC.xlsx`) | 202 with an empty body | HUD's per-CoC PDFs on `files.hudexchange.info`, which serve |
| MuckRock API | 401 "Authentication credentials were not provided" | a free MuckRock account token |
| Judicial Council court statistics | dashboard URL not found; `courts.ca.gov/news-reference/research-data/court-statistics` links only the yearly report PDF | PDF tables by county |
