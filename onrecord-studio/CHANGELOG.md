# Changelog

Notable, user-visible changes. The in-app Help topic *What's next* lists planned enhancements;
`docs/ROADMAP.md` holds the engineering record.

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
