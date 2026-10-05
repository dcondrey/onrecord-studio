# Meeting bodies: where agendas are published

Checked 2026-10-04 through a summarising fetcher (no raw HTML or headers inspected). "Verified"
means an example agenda was opened; otherwise only the index page was fetched. Paths are relative
to `https://www.sandiego.gov` unless a host is given. No single feed covers these bodies; each
index is polled separately. sandiego.gov PDFs carry an HTTP `Last-Modified` header (measured on
the 2026-10-06 Council agenda: Fri 2026-10-02 21:31 GMT); OnBase pages return the request time.

Re-checked 2026-10-05 with plain `curl` (headers and HTML read): the Homelessness Revenue Account
Citizens Oversight Committee, Privacy Advisory Board (257 PDF links) and Planning Commission
indexes link their PDFs inside `<main>` under `/sites/default/files/`; `/ethics/meetings` links one
page per year back to 2001, so the registry uses the 2026 page; `/sites/default/files/agenda.pdf`
(Civil Service) answered with `Last-Modified` 2026-09-21. sandiego.gov HTML pages, like OnBase
pages, send `Last-Modified` 0 to 2 seconds before `Date`, so only file responses carry a file time.
The app's registry of these bodies is `src/content/meetingBodies.json`.

Roster of City bodies: `/boards-and-commissions/list?page=0..4` (server-rendered, 10 per page).
`/citywide-agendas-minutes` is stale. `boards.sandiego.gov` is client-rendered and lists no agendas.

## sandiego.gov page with PDF links

PDFs sit at `/sites/default/files/YYYY-MM/<name>.pdf`; names are not uniform, so read the index.

| Body | Index | Verified | Notes |
|---|---|---|---|
| Homelessness Revenue Account Citizens Oversight Committee | `/homelessness-strategies-and-solutions/hracoc` | yes | one agenda (2026-08-28), no minutes yet |
| Commission on Police Practices | `/cpp/meetingsagendas` | yes | attachments inside the PDF |
| Human Relations Commission | `/human-relations/meetings` | yes | minutes months late |
| Privacy Advisory Board | `/pab/meetings` | yes | surveillance policies and impact reports attached |
| Consolidated Plan Advisory Board | `/cdbg/cpab` | yes | |
| Planning Commission | `/planning-commission/documents/agenda` | yes | staff reports at `/planning-commission/documents/pcreports`, minutes at `/documents/minutes` |
| Parks and Recreation Board | `/parkandrecboard/meetings` | yes | |
| Balboa Park Committee | `/park-and-recreation/general-info/boards/bpc` | yes | |
| Mission Bay Park Committee | `/park-and-recreation/general-info/boards/mbpc` | yes | per-item attachments |
| Independent Rates Oversight Committee | `/public-utilities/independent-rates-oversight-committee/agendas-minutes` | yes | newest minutes listed 2023-01-17 |
| Civil Service Commission | `/empopp/about/civilser` | yes | fixed URL `/sites/default/files/agenda.pdf`, overwritten each meeting; minutes not indexed |
| Ethics Commission | `/ethics/meetings/<YYYY>meetings` | yes | |
| Community Reinvestment Review Advisory Committee | `/crrac` | yes | nothing in 2026 |
| Accessibility Advisory Board | `/adacompliance/aab` | no | |
| Airports Advisory Committee | `/airports/aac/meetingdocs` | no | |
| Board of Building Appeals and Advisors | `/development-services/board-building-appeals-and-advisors` | no | |
| Citizens Equal Opportunity Commission | `/eoc/ceoc` | no | |
| Climate Advisory Board | `/planning/public-hearings-meetings/climate-advisory-board` | no | |
| Commission for Arts and Culture | `/economic-development/cultural-affairs/meetings` | no | packets, no minutes |
| Commission on the Status of Women | `/status-of-women` | no | weak index |
| Community Forest Advisory Board | `/planning/boardcomm/cfab` | no | |
| Defined Contribution Plans Trustee Board | `/riskmanagement/dcptb` | no | |
| Franchise Compliance Review Committee | `/general-services/energy/sdge-franchise/crc` | no | convened every two years |
| Funds Commission | `/funds-commission/meetings` | no | |
| Commission on Gang Prevention and Intervention | `/gangcommission` | no | |
| Historical Resources Board | `/development-services/public-hearings-meetings-notices/historical-resources-board/agendas` | no | |
| International Affairs Board | `/iab/meetings` | no | |
| La Jolla Shores Planned District Advisory Board | `/planning/community-plans/la-jolla/pdoab` | no | |
| Board of Library Commissioners | `/public-library/about-the-library/commissioners` | no | |
| Mobility Board | `/mobility-board` | no | |
| Municipal Golf Committee | `/park-and-recreation/general-info/boards/mgc` | no | |
| Otay Mesa EIFD Public Financing Authority | `/city-clerk/officialdocs/legisdocs/omeifd` | no | |
| Resiliency Advisory Board | `/resiliency-advisory-board` | no | |
| Senior Affairs Advisory Board | `/park-and-recreation/general-info/boards/saab` | no | |
| Small Business Advisory Board | `/economic-development/business/starting/advisory` | no | |
| Youth Commission | `/youth-commission/meetings` | no | looks dormant |
| Community planning groups (about 40) | `/planning/community/profiles/<slug>/agendas` | Uptown only | Community Planners Committee: `/planning/community/cpc/agendas` |

## OnBase Agenda Online

- City Council: `sandiego.hylandcloud.com/211agendaonlinecouncil` (site=council).
- Council committees, including Audit: `sandiego.hylandcloud.com/211agendaonlinecomm` (site=comm).
  No non-council board is in it. Meeting-type ids for `mtids` were not obtained. Verified
  2026-10-05: the index is server-rendered with `Meetings/ViewMeeting?id=&doctype=1&site=comm`
  links, and a meeting page declares `/211agendaonlinecomm/Documents/ViewAgenda?meetingId=…`, the
  same template as the Council application. Item links were not seen (the agenda read had none).
- SDCERS Board of Administration and its committees:
  `board.sdcers.gov/onbaseagendaonline/Meetings/Search?dropid=4&mtids=`. Same URL grammar as the
  City apps is inferred, not verified.

Grammar: `Meetings/Search?dropid=4&mtids=` lists meetings; `Documents/ViewAgenda?meetingId=&doctype=&site=`
is the server-rendered agenda (doctype 1 agenda, 3 results summary for Council, 2 actions for
committees); `Meetings/ViewMeetingAgendaItem?meetingId=&itemId=&isSection=false&type=agenda` lists
attachments; `Documents/DownloadFile/<name>.pdf?...publishId=` returns the bytes and needs the
publishId from the item page. Revisions reuse the meeting id and appear only as title text and
inline "(Rev. …)" or "(Added …)" markers.

## Agency sites

- San Diego Housing Commission board: `https://sdhc.org/governance-legislative-affairs/sdhc-board-of-commissioners/meetings/`
  (bare host; `www.` returns 403). Staff reports are separate links. Upload times from
  `https://sdhc.org/wp-json/wp/v2/media?search=<term>&_fields=date_gmt,modified_gmt,link,source_url`.
- Housing Authority: `https://sdhc.org/governance-legislative-affairs/housing-authority-of-the-city-of-san-diego/meeting/`.
  Verified 2026-10-05: both SDHC pages list agendas in table rows as extensionless attachment
  pages (`https://sdhc.org/<date>-…-agenda/`), with no `<main>` element.
- San Diego County Regional Airport Authority: `https://www.san.org/Airport-Authority/Meetings-Agendas`;
  `https://www.san.org/wp-json/wp/v2/media?search=Board-Meeting-Agenda`; robots asks for a 10-second crawl delay.
- Convention Center Corporation: `https://www.visitsandiego.com/board-of-directors`; PDFs on `sdcc.canto.com` with opaque URLs.
- Port of San Diego: eSCRIBE at `https://pub-portofsandiego.escribemeetings.com/` (returned 403 to
  the fetcher; unverified). Legistar client `portofsandiego` holds nothing after 2023-11-14.

## Settlements and closed session

- Council resolutions index: `/city-clerk/officialdocs/council-resolutions-ordinances?page=N`
  (Year, Date, Doc Num, Title; 25 per page). Settlements over 50,000 dollars are resolutions titled
  "settlement"; PDFs at `docs.sandiego.gov/council_reso_ordinance/rao<YYYY>/R-<number>.pdf` are
  scanned (amounts need tolerant reading). The index date is not the date of final passage.
- Closed session reports: `/city-clerk/councilcomm/closedsess` (case names and claim file numbers, no amounts).

## Not found

Managed Competition Independent Review Board (only 2008 documents); Horton Plaza Theatres
Foundation (`hptf.us`, no agenda links seen); San Diego County Water Authority (`sdcwa.org`
returned 403 on every route; meetings page reported at `/about-us/board-of-directors/meetings/`).
