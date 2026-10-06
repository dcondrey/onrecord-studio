# Changelog

Notable, user-visible changes. The in-app Help topic *What's next* lists planned enhancements;
`docs/ROADMAP.md` holds the engineering record.

## 1.0.0 (2026-10-06)

### Added

- City datasets fetched by name and year: SDPD calls for service, NIBRS offenses, arrests and
  citations, Get It Done reports, operating budget and actuals, the emergency shelter inventory,
  schools and transit stops. Leads from them cite every table and the SQL behind them.
- Leads about homelessness enforcement and services: encampment ordinance observations, hours
  between a posted abatement notice and the abatement, reports around abatements, stops, arrests
  and property seizures of people perceived as unhoused, spending records, and shelter capacity
  set beside budget and Encampment reports by year.
- Organisation records: Form 990 returns, single audits and recorded payments by EIN, people across
  returns, a checked-organisations portfolio, and links to held documents and requests.
- Meeting bodies: a registry of City bodies, agenda posting times and differences between agenda
  copies.
- Record gaps behind each lead, with a drafted request to fill one; a follow-up letter for a request
  overdue with no response.
- Hand-added files are labelled and compared with collected documents instead of being read as
  agency releases; behested payment reports are counted from the disclosure portal's export.

### Changed

- Table imports read the file once and keep every row's original cells; a cell that does not
  convert is kept as written and counted, not dropped with its row. Imports are faster and use less
  memory, XLSX worksheets are streamed, and a worksheet whose cells are stored out of row order is
  refused with a way forward.
- On first launch the core and corpus databases are rewritten once to compressed storage, and
  automatic tables are re-imported under the new import rules; large libraries take longer to open
  that once.

### Fixed

- An organisation's name now matches vendor values written without its apostrophe.
- A re-imported table's rows are no longer reported again, unreviewed, by area monitoring.
- Redaction findings come only from explicit marks; blank form fields are not redactions.
- Rate-limited portals no longer stop a collection partway; queued tasks are no longer abandoned.

## 1.0.0-beta.3 (2026-10-02)

### Fixed

- The license check and the 30-day trial were always refused with "The clock guard is
  unavailable": the native Keychain bridge refused one of the three values the check reads and
  writes.
- Eight buttons and one PDF annotation label drew text the same color as their own background,
  including "Verify license again", "Continue to checkout", "Use this Mac and sign out my oldest
  Mac", "Delete account", "Revoke", and "Open checkout in your browser". Every solid button/text
  color pairing in the app was checked against the WCAG contrast minimum in both themes.

## 1.0.0-beta.2 (2026-10-02)

### Added

- Outlook PST mailboxes are ingested as bounded containers.
- OCR uses Apple Vision and PDFKit on macOS.
- Check source re-fetches up to 100 collected documents of an active source and records whether
  each is unchanged, changed, gone or unreachable; watches can report a changed source copy.

### Fixed

- Quoted search phrases require the words consecutively instead of matching them anywhere.
- Sign-in, upgrade and account-deletion error messages were red on red; they are now readable.

### Changed

- New databases are created in a newer storage format that compresses long text. Existing
  databases keep their format.

## 1.0.0-beta.1 (2026-10-01)

First beta. macOS 13 or later, Apple silicon and Intel, as a notarized direct download with
signed updates or as a Mac App Store build.

### Added

- Research snapshot export: a read-only DuckDB file of requests, documents, deadline evaluations
  and findings with a signed manifest carrying the file digest and a corpus-state digest, so a
  result published from Python, R or SQL can be cited and re-run. The standalone verifier checks
  the file against its manifest.
- Shared-folder collaboration: exchange signed case packets through any folder and record which
  contributor keys you have verified. No server is involved.
- Findings inbox: filter by severity and text, sort by rank or recency, load more in pages, and a
  plain statement when the cap hides more.
- Search: watch a search and record its result; capped totals are no longer presented as exact.
- Entity graph: filter by relationship basis and date.
- Manage subscription in the account menu opens Stripe's billing portal for seats, payment method
  and cancellation. Cancelling stops renewal at the period end; the license stays active until then.
- Seats: the purchaser of an organization subscription assigns seats to colleagues by email
  address and revokes them from the account menu. A seat is claimed at the colleague's first
  license check with a provider-confirmed address; no email is sent by the app.
- Support entry point (github.com/writerslogic/support) from the Help header and the Start here
  topic; the app version is shown in Help.
- Help topics for every view, finding kind and command group, including *What's next*; a user
  guide and research-snapshot schema reference under `docs/`.
- Per-fetch acquisition log: every network fetch records its URL, redirects, status, headers and
  digests, so a re-fetch no longer erases what was received before.

### Changed

- CPRA deadlines whose last day falls on a Saturday, Sunday or California judicial holiday now end
  on the next court day, and stored findings were recomputed under that rule.
- OCR tools are found in the usual Homebrew, MacPorts and /usr/local locations even when the app
  is launched from the Finder, and the sidebar names the install command when one is missing.
- Render failures are contained per view instead of blanking the window; destructive and import
  dialogs trap focus and return it on close.

### Fixed

- A purge no longer removes content-addressed files that another row still references, and a
  concurrent sync of identical bytes no longer races it.
- A frontier task interrupted on every attempt is abandoned and recorded instead of replayed at
  each launch.
- PDF loads and Office part reads are bounded against decompression bombs.
- Package verification binds the signer key id to the key that verified the package.
- A clean checkout builds: the application icons are tracked.

### Known limitations

- OCR for scanned PDFs and images needs Tesseract (with `eng`) and Poppler installed.
- Windows and Linux bundles are produced by CI as compile evidence only; they are not distributed.
