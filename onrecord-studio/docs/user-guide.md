# OnRecord Studio user guide

OnRecord Studio collects public records you point it at, reads them on your Mac, looks for patterns
and counted deadline failures, and keeps every claim tied to a passage in a source document. It
never files a request, sends an appeal, or contacts an agency for you. The in-app Help
(Cmd+Shift+/) covers every view, finding kind and setting in more detail; this guide is the short
path from installation to a first exported result.

## Installation

Requirements: macOS 13 or later, Apple silicon or Intel.

- **Mac App Store build**: install from the App Store. It is a one-time paid app verified by Apple
  with no account, trial or license key. Account and work-email features are not part of it, and it
  updates through the App Store.
- **Direct download build**: open the DMG, drag OnRecord Studio to Applications, and launch it. The
  app is signed with a Developer ID and notarized, so Gatekeeper opens it without a warning. On first
  launch you sign in with Google, Apple, LinkedIn or GitHub in a system sign-in window; a free trial
  starts once per account, and a purchased license is checked against the account service.

### Optional OCR

Scanned PDFs and images are only searchable when two command-line tools are installed; everything
else is built in. With [Homebrew](https://brew.sh):

```sh
brew install tesseract poppler
```

The sidebar footer shows **Local OCR: Image + PDF** once both are found. Hover it for the probe
details. Tesseract's `eng` language pack, which Homebrew installs by default, is required.

## First launch

The app creates its workspace under `~/Library/Application Support/com.onrecord.studio/`: the
core database, the corpus index, downloaded documents, embedding model files, and a `reports/`
folder for exports. Nothing is created anywhere else.

Work in this order:

1. **Add a source** from the header box: a public URL, local files, or pasted text. Only public
   URLs are fetched; no portal login or password is ever asked for.
2. **Wait for the sync.** At most five sources sync at once. The Activity view shows queues and
   errors. When a sync finishes the app runs every detector and opens a brief of anything new.
3. **Read the Findings inbox** on the San Diego view. Each row says whether it is a *pattern
   signal* or a *counted legal claim*, and whether its confidence was measured or is unassessed.
4. **Open the evidence** behind a finding, read the source document, and check its provenance.
5. **Save what you reviewed** to the Briefing view and export an outline, or export an evidence
   packet or verifiable report from the inbox.

## The nine views

San Diego (findings, drafts, research, case workbench), Patterns (cross-document connections and
the entity graph), Analysis (semantic search and read-only SQL over imported tables), Evidence
(request search and document reading), Activity (ingestion queues and errors), Briefing
(public-comment outline), Story desk (findings ranked by observable factors), Meetings (council
calendar from collected agendas) and Compare (agencies ranked on measured latencies). Switch with
the sidebar, the command palette (Cmd+K) or Cmd+1 to Cmd+9.

## Reading a finding

- **Legal claim: counted violation** means a statutory rule was compared against recorded dates and
  the count came out against the agency. **Pattern signal** means the records show something worth
  review; it is not a legal conclusion.
- **N% measured** is a confidence derived from what the detector observed. **Unassessed** means
  no confidence was measured; it is not zero.
- **Why we believe this** opens the explanation and numbered citations: portal, document,
  character range and quoted text. Dismiss removes a finding permanently.
- **Evidence gaps** list checks the app could not run because the records are not collected. A gap
  is not a finding and not evidence of compliance.

## Public-records requests and deadlines

**My requests** (header icon) tracks requests you filed elsewhere. You enter the portal's request
number, the jurisdiction, and the dates of each event; the statutory deadline is counted only from
dates you confirm. Notification emails (.eml or .msg) can be imported to suggest events, but a
suggestion is not counted until you confirm it. Requests carrying a statutory latency flag in
Evidence offer an appeal draft. Drafts are text for you to submit; the app sends nothing.

## Search

The Evidence search box looks in request metadata and extracted text. Qualifiers `agency:`,
`portal:`, `status:` and `flag:` narrow results, and plain-language mode turns a sentence into those
filters. Results are capped at 500. A missing match does not show the sources lack the
information. Semantic search in Analysis needs the embedding model, downloaded once on
**Initialize model**; until then search is exact-word only.

## Exports

- **Verifiable report** (Findings inbox): one HTML file that checks its own signature in the
  reader's browser. The signature shows the file is unchanged, not who signed it or that its contents
  are true.
- **Evidence packet** (Findings inbox): a Markdown file and a JSON file with each claim's basis,
  sources, quoted excerpts, confidence, and what it does not show, plus a content hash.
- **Briefing outline**: a Markdown file with evidence locators, source URLs and file hashes, saved
  through the download dialog.
- **Case chain-of-custody package** (Casework) and **appeal draft** (Evidence): signed packages
  that the verifier above the workbench tabs checks for integrity.

- **Research snapshot** (San Diego view, *Collection, coverage and storage*): a read-only DuckDB
  file of requests, documents, deadline evaluations and findings, with a signed manifest naming the
  exact corpus state, for Python, R or SQL work that can be cited and re-run. See
  [research-snapshot.md](research-snapshot.md).

Reports, packets and snapshots are written to the `reports/` folder of the workspace directory above.

## Privacy

Documents, extracted text, indexes, findings, tables, drafts, tracked requests and case files stay
on your Mac. Some features contact other servers, and the Help topic *Data handling* lists each
one. In short:

- The direct build's sign-in, trial, license and checkout requests go to the WritersLogic account
  service; it receives your email address and plan, never documents or findings. Payment happens
  at Stripe in your browser.
- The sources you add, and the built-in list of public publishers, are fetched from the internet.
- Update checks (direct build only) send the app version, OS target and architecture.
- Autonomous research is the only feature that uses an AI model, and it is off until you enable it.
- There is no analytics or advertising service.

## AI configuration

Autonomous research drafts dossiers from records you already collected. Turn it on under
**Research settings** on the San Diego view. It uses a key found in your environment or `~/.env`
(`OPENAI_API_KEY` or `OPENROUTER_API_KEY`); there is no key field in the app, and usage is billed
to your key. Choose **Local Ollama** to keep model calls on your Mac. The default budget is 24
model calls per day. Untick *Autonomous research enabled* to stop all model calls.

## Backup and data location

Quit the app, then copy `~/Library/Application Support/com.onrecord.studio/` as a unit. The
databases and downloaded documents are one dataset; restoring only part of it mixes schema versions.
Removing a source keeps its documents; *Delete source and its documents* destroys them and cannot be
undone from inside the app.

## Updates

The direct build checks for a signed update at launch and about every 24 hours and shows a notice
in the header. **Install and restart** downloads, verifies the signature against a key built into
the app, installs and relaunches. **Later** hides the notice for that version until the next
launch. Your license, account and local data are not affected. The App Store build updates through
the App Store.

## Troubleshooting

- **Local OCR shows Unavailable or Image only**: install Tesseract and Poppler as above and relaunch.
  The footer tooltip names what the probe could not find.
- **A locked screen** (sign in, trial ended, license needs verification): **Verify license again**
  rechecks with the account service. Local evidence is never deleted by a lock.
- **This Mac's clock looks wrong**: the clock was set earlier than a time the app already saw.
  Correct the system clock and verify again.
- **Search failed**: the last successful results stay on screen with a banner. Retry after the sync
  finishes.
- **Update failed**: the notice offers **Retry install**. A failed install leaves the current
  version running.
- **A panel says it did not complete**: a query behind it failed. It is not an empty result; check
  the Activity view for the error.
