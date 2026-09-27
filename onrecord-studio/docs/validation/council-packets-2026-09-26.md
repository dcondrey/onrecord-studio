# Council packet ingestion, September 26, 2026

The City Council's OnBase portal links to meeting shells that load agendas and
supporting records through JavaScript. The previous HTML crawler recognized the
agenda route but discarded `javascript:loadAgendaItem(...)` references. A bounded
crawl could therefore spend its page allowance on meeting shells without finding
the staff reports, contracts and amendments needed to investigate an agenda item.

The native client now resolves a meeting's explicitly declared agenda route during
that meeting's inspection. It reads literal item references without executing
JavaScript, prioritizes homelessness, services and spending items, and expands up
to eight attachment indexes per meeting. Each request still uses the existing
host pacing, retry, robots and request budget. Attachment links must match the
selected meeting and item on the exact City tenant. Agenda text retains the
agenda URL; attachments retain their individual publication URLs.

Other item links remain available to the ordinary bounded crawl. Bootstrap
navigation menus are now recognized as navigation so past-search and contact
links do not displace meeting records. This remains a bounded collection, not a
claim that every attachment from every Council meeting has been acquired.
An unavailable or redirected optional item index leaves its retry link intact;
it does not discard the agenda or other successfully discovered attachments.

## Concrete source checked

[September 28 special meeting, meeting 7192](https://sandiego.hylandcloud.com/211agendaonlinecouncil/Meetings/ViewMeeting?doctype=1&id=7192&site=council)
includes the Riverbed Encampment Resolution Funding amendment, item 605. Its
[published item index](https://sandiego.hylandcloud.com/211agendaonlinecouncil/Meetings/ViewMeetingAgendaItem?meetingId=7192&itemId=265583&isSection=false&type=agenda)
exposes seven supporting records: staff report, proposed resolution R-2027-55,
first grant amendment, underlying grant agreement, EOC Program Evaluation,
Land Use and Housing Committee green sheet, and approval log. These provide
specific documents for checking what changes, the original obligations, and the
committee's prior consideration. Discovery alone establishes none of their
substantive claims.

## Reproducible checks

`cargo test --manifest-path src-tauri/Cargo.toml --lib council_`
checks literal parsing, deduplication, research priority, item identity,
cross-host/cross-item rejection and navigation filtering.
The partial-failure test also keeps successful attachments and the failed item's
original follow link after an index error or unexpected redirect.

`cargo test --manifest-path src-tauri/Cargo.toml --test council_packet_live -- --ignored --nocapture`
reads the real meeting, agenda and bounded item indexes with host concurrency
set to one. It asserts that Riverbed supporting document URLs are discovered and
saves a compact JSON record under the system temporary directory. It does not
use model credentials, download the PDFs, or alter the installed app database.

The live native check passed in 4.82 seconds, discovering 69 download links across
the agenda and its eight expanded item indexes, including all seven Riverbed
supporting records. The compact result is [council-packet-links.json](council-packet-links.json).
