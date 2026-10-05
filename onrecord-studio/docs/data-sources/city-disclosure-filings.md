# City of San Diego disclosure filings (eFile-SD)

Mapped 2026-10-04 with about 53 sequential requests. Every route below was called and returned
200 unless marked inferred. The portal pages (`https://efile.sandiego.gov/public/search/...`) are a
JavaScript shell with no filings in the HTML; the data comes from a JSON API.

Base `https://efile.sandiego.gov/api/v1`, header `x-tenant: csd` (sent on every call; not tested
without it). No key, cookie or CSRF token. Envelope `{"success":"true","data":...}`. No
`openapi.json`. Operator: Pasadena Consulting Group; the same product serves other agencies
(`sandiegocounty.efile.systems`, `sdc.efile.systems`, and others).

## Terms: unresolved

The site's Terms of Service (`GET /help/markdown?doc_id=terms-of-service&doc_type=public`) prohibit
any "deep link", "page-scrape", "robot", "spider" or other automatic device to access, copy or
monitor any portion of the site, or to obtain information "through any means not made purposely
available through the website", and prohibit use "for advertising or commercial purposes". There is
no robots.txt. The export routes below are the ones the site itself offers as download buttons.
No reader is built until the owner decides how to proceed (see docs/ROADMAP.md).

## Routes

| What | Request | Paging | Total seen |
|---|---|---|---|
| Statements of Economic Interests (Form 700) | `POST /public/sei-search` JSON `{page_number, page_size, query, start_date, end_date, filing_types:["fppc700","ec700"], filing_subtypes:[], sort_column}` | `page_number`/`page_size`; response has `total_pages` only | 51,424 |
| Agency reports (Forms 801, 802, 803, 806) | `GET /public/agency-reports-search?query=&start_date=&end_date=&filing_types=fppc801,fppc802,fppc803,fppc806` | none; returns every match | 1,333, earliest 2008-08-18 |
| Campaign filings | `GET /public/campaign-search?page_number=&page_size=&start_date=&end_date=&query=&state_id=&sort_column=` | `total_pages`, `total_count` | 16,146 |
| Campaign bulk export | `GET /public/campaign-bulk-export-info`, then `GET /public/campaign-bulk-export-url?year=&most_recent_only=true` (returns an S3 URL to an .xlsx; not downloaded) | per year, 2018 to 2026 | |
| Search export | `GET /public/export?start_date&end_date&search_string&file_format=csv\|xlsx&document_type=agency_reports\|sei\|campaign_search\|lobbyist_filings\|public&filing_types=` | whole result | |
| Lobbyist filings (EC-601 to 605) | `GET /public/lobbyist-search?start_date&end_date` | none | |
| Lobbyist registrations | `GET /public/lobbyist-search/annual?year=` | none | 102 entities for 2026 |
| Lobbyist quarterly expenditure | `GET /public/quarterly-report/expenditure?year=&quarter=Q2` | none | siblings (`muni-decision`, `new-registration`, `termination`, `activity-expense`, `no-activity`, `non-or-late-filer`) inferred |
| One filing's PDF | `GET /public/document?doc_id=<doc_public>` returns a presigned S3 URL | | |
| One filing's metadata | `GET /public/document/metadata?doc_id=&filing_id=` adds positions (`kind, position, department, organization`) | | |
| Form 700 as HTML | `GET /public/sei/fppc700/<filing_id>` when `show_html` is true | | |

Agency reports by form through 2024: 960 Form 802 (ticket distribution), 278 Form 803 (behested
payments), 36 Form 806 (appointments), 34 Form 801 (gifts to agency). Export columns for agency
reports: Filing Date, Filing Type, Description, Agency, Official.

## Identity and amendments

`filing_id` (UUID) identifies a filing; agency reports have none and use `doc_public`
(`Ext_` + UUID). `e_filing_id` is the printed number and is null for paper filings. An amendment
has `amendment: true`, `amendment_number`, `amends_orig_id` and `amends_prev_id` (strings in SEI
results, objects with `human_id` in campaign and lobbyist results; a campaign original points
`amends_orig_id` at itself). Only the public copy of a filing is served; the API has no flag for
redaction. SEI `filing_date` can be a data-entry date (earliest row is dated 2000 with a 2009
period), and `sort_column=asc` did not reverse campaign results.
