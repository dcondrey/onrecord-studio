# San Diego product focus

Decision: September 13, 2026. Make the City of San Diego workflow dependable before expanding connector coverage. The initial research task is police oversight and Commission on Police Practices meeting preparation, supported by City Council agendas and public-records responses. San Diego County and the Sheriff's Department are separate jurisdictions, not alternate names for City records.

The primary workflow is: choose a local source → acquire its published records → find a relevant record → read and save an excerpt → prepare a briefing with traceable evidence. The home screen now presents this workflow instead of an empty analytics dashboard. Existing saved sources remain accessible; response analytics are available on demand.

## Maintained starting points

| Source | Default coverage | Crawl limits |
| --- | --- | --- |
| [CPP agendas and minutes](https://www.sandiego.gov/cpp/meetingsagendas) | Current calendar and directly linked City documents; earlier years and video are separate. | 1 page, depth 0, 300 documents |
| [SDPD policies and procedures](https://www.sandiego.gov/police/data-transparency/policies-procedures) | Published categories and their pagination, with City document downloads. Document revision dates remain authoritative. | 100 pages, depth 4, 1,000 documents |
| [CPP resources](https://www.sandiego.gov/cpp/reports) | The resource page and directly linked City documents. External reference sites are excluded. | 1 page, depth 0, 300 documents |
| [City Council agendas](https://www.sandiego.gov/city-clerk/officialdocs/legisdocs/brown-act) | Currently posted agendas. The historical Council packet archive is separate. | 1 page, depth 0, 100 documents |

The City [public-records information page](https://www.sandiego.gov/it/public-records) identifies [sandiego.nextrequest.com](https://sandiego.nextrequest.com/) as its public request portal. Automatic import of that portal is not currently supported; download response files there and import them locally. The home screen presents it as a reference website, not a working acquisition preset. None of these sources guarantees access to every record held by the City.

Source cards stage the selected URL in the toolbar; Add starts acquisition. Named document sources apply bounded recipes scoped to the selected section and the City's document directory, with external crawling disabled. Crawl settings show the effective limits and permit an explicit adjustment. A saved source card offers access to its records and an update action.

## Reliability changes

- City document entry points bypass unrelated API discovery probes, preserving the request budget for actual records.
- SDPD's Drupal pagination is distinguished from site navigation. Known category and numeric page parameters retain distinct crawl identities; unrelated query variants continue to collapse. The exception recognizes only the City's policy catalog and is bounded to page indices below 100 plus the source's overall page budget.
- Search includes request title, description, public request identifier, and numeric identifier alongside indexed attachment text. Every plain-text term must match, including terms split between metadata and attachments. Metadata-only records are searchable immediately.
- Department options retain both source and agency identity. Selecting a department from all saved sources scopes the search to that source.
- Ingestion refreshes search and open document lists through coalesced notifications, retaining an already open document.
- Moving between document chunks resets text pagination and provides a Previous chunk action. Evidence links open the correct document offset in the Evidence workspace, independently of the current search result page.
- CPP briefing starters use the maintained calendar and instruct the researcher to confirm the meeting details. No hard-coded meeting is presented as happening tonight.

## Verification

Frontend integration tests exercise source selection and effective limits, department identity collisions, documents appearing during ingestion, long-document navigation, and opening an evidence link in a fresh application. Native regression tests exercise metadata-only search, mixed metadata/document terms, City source routing, and SDPD category/page handling.

September 13 verification: 109 frontend tests and 318 native tests passed, along with the production frontend build, lint, Rust formatting, Clippy with warnings denied, and the 40-command IPC contract check. A browser review exercised the real React layout with a synthetic native bridge. The two live native tests passed: the CPP calendar yielded the September 2, 2026 agenda with 34,763 extracted characters available through search and document loading; SDPD's second procedures page exposed 20 additional documents. An installed desktop UI session and a complete SDPD acquisition were not exercised.

Live checks are deliberately opt-in because public content and availability change:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test san_diego_live -- --ignored --nocapture
```

They use temporary storage and bounded requests. The CPP check follows the current calendar through acquisition, extraction, search, and document text loading. The SDPD check verifies that following pagination exposes additional documents. These are sampled workflows, not a certification that the entire City archive has been acquired. The desktop's installed data is not altered by the checks.

A separate live NextRequest probe failed on September 13. `/client/requests` returns a public index containing `request_date`, a string identifier such as `26-9306`, and `request_text`, while the importer requires numeric identifiers and creation timestamps. The tested `per_page`, `limit`, and `page_size` parameters each returned 100 rows despite requesting one. Supporting this index requires explicit field mapping, verified pagination, and attachment retrieval. Treating the homepage as a successful website crawl would not establish public-request coverage.

## Next acceptance milestones

1. Implement and validate the City's actual NextRequest index and attachment format before enabling automatic portal import.
2. Run a complete bounded SDPD acquisition and report coverage by category, including failed, deferred, and excluded documents.
3. Validate each published meeting packet format, scanned PDFs, and Spanish-language documents with the configured OCR installation.
4. Exercise meeting preparation with a real researcher: find two records, save reviewed excerpts, reopen their evidence links, and export the outline.

County/Sheriff connectors, historical Council archive automation, video transcription, additional jurisdictions, and new general-purpose platform adapters are outside the initial acceptance scope. Add them when a concrete San Diego research task calls for them.
