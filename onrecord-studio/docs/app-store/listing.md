# OnRecord Studio: Mac App Store listing

All the details to enter in App Store Connect for the paid Mac App Store build
(`--features app-store`). Values were checked against the code on 2026-10-01. **OWNER** marks a
value only you can supply or decide. Copy each field from the fenced block under it. The character
counts were measured by script.

Contents:

1. Submission blockers
2. Before you can attach a build
3. App Information
4. Pricing and Availability
5. App Privacy
6. Version 1.0 page (text)
7. Screenshots and icon
8. App Review Information
9. Age Rating
10. Accessibility
11. Build, export compliance and release

---

## 1. Submission blockers

Fix or confirm each of these before you press Submit for Review. Items marked "fixed on branch" are
done on the `claude/accessibility` branch (worktree `/Volumes/A/onrecord-studio-appstore`) and
reach the build once that branch is merged.

| # | Issue | Why it matters | Status |
|---|---|---|---|
| 1 | DuckDB downloaded two extensions (native code) from extensions.duckdb.org at runtime: **json**, for `json_extract_string` in `storage/maintenance.rs`, and **icu**, for the `SET TimeZone = 'UTC'` that every core, corpus and vector connection runs on open. The `bundled` feature compiles neither, and auto-install and auto-load are on by default. Evidence: `~/.duckdb/extensions/v1.5.5/osx_arm64/{json,icu}.duckdb_extension`. | App Review 2.5.2 forbids downloading executable code. Library validation in a signed, hardened build also refuses to load an unsigned extension. With ICU blocked, the core database fails to open ("Failed to configure DuckDB"). **This affects the Developer ID beta too.** It has worked only on machines that already had the extension cached. | Fixed on branch (commit `3e7efac`): `json` is compiled in (`duckdb` features `bundled`, `json`). ICU can't be compiled from the published crate, so the app no longer needs it: the `SET TimeZone = 'UTC'` statements are removed (with no ICU loaded the session zone is UTC anyway), and `now()` is cast to `TIMESTAMP` before interval arithmetic. Every app connection opens through `storage::open_duckdb` with auto-install and auto-load off. Two regression tests cover this. All 1,179 Rust tests pass with no extension cache in use. |
| 2 | In-app Help described Stripe checkout, per-seat pricing, the trial, sign-in, the self-updater and a "beta" roadmap in every build. | 3.1.1 and 3.1.3: pointing users to another way to pay. 2.2: beta content. | Fixed on branch (commit `b4e1c58`): Help topics and sections carry a `channel`, the App Store build is built with `ONRECORD_DISTRIBUTION=app-store` and shows only content true of it, and a test fails if App Store Help mentions any of those. |
| 3 | The AI research help told users to put an API key in `~/.env`. In the sandbox that resolves to the app container, and shell variables are not inherited. | A reviewer following Help would find the feature dead (2.1). | Fixed on branch: the App Store text names the container path. The app itself is unchanged. |
| 4 | The version is `1.0.0-beta.1` in three files. | App Store versions must be numeric, and a "beta" build is rejected under 2.2. | OWNER: drop the suffix for the App Store build only (see section 11). |
| 5 | Source-citation links use `<a target="_blank">` (19 places), and the opener permission allows only Stripe, writerslogic.com and GitHub. | If the webview ignores these clicks, links like "View publisher's original file" are dead buttons (2.1). | **Unverified.** Click one in the signed App Store build. If nothing opens, route anchor clicks through `openExternalUrl` and widen the opener scope to `https://*`. |
| 6 | OCR ran Homebrew `tesseract` and `pdftoppm` as child processes, which the sandbox is unlikely to allow. | Scanned documents would get no text in the paid build. | Fixed on branch (commit `7ac0a67`): on macOS, OCR runs in-process with Apple Vision, and PDF pages are rendered with PDFKit. Both are system frameworks, so no child process and no install are involved. **Unverified in the sandbox:** check that the sidebar's "Local OCR" line reads Image + PDF in the signed App Store build. |
| 7 | There is no `PrivacyInfo.xcprivacy` privacy manifest. | Apple enforces privacy manifests for iOS-family uploads. Enforcement for macOS uploads is not confirmed. | Watch for an ITMS-91053 email after upload. If one arrives, add a manifest declaring the required-reason APIs it names. |
| 8 | The main view, sidebar subtitle and the starter queue are written for San Diego ("San Diego research workspace", "Investigate homelessness spending…"). | A worldwide listing that opens on another city's topics confuses buyers. It is not a rule violation. | OWNER decision: keep it as a San Diego edition, or make the region generic. Either way, the screenshots must match. |

## 2. Before you can attach a build

The Developer ID work done today does not cover the App Store. At developer.apple.com
(team U3PZN7P3E5) you need:

1. **Certificates:** an **Apple Distribution** certificate and a **Mac Installer Distribution**
   certificate. Make each from a new CSR, which I can generate.
2. **Profile:** Profiles, then + under Distribution, then **Mac App Store Connect**. Choose App ID
   `com.onrecord.studio` and the Apple Distribution certificate.
3. **Agreements:** accept the **Paid Apps Agreement** in App Store Connect (Business), with
   banking and tax forms filled in. A paid app can't be submitted without it.
4. **App record:** in App Store Connect, go to Apps, then +, then New App:
   - Platform: **macOS**
   - Name: `OnRecord Studio`
   - Primary language: **English (U.S.)**
   - Bundle ID: `com.onrecord.studio`
   - SKU: `onrecord-studio-mac` (any unique string; it's never shown to customers)
   - User access: Full Access
5. **Upload:** an App Store Connect API key with the App Manager role for uploading.
   `~/.appstoreconnect/private_keys/` already holds `AuthKey_K7S97U6PZ6.p8` and
   `AuthKey_TQSD79UAG8.p8`. You also need that key's **Issuer ID**, from App Store Connect, then
   Users and Access, then Integrations.

## 3. App Information

| Field | Value |
|---|---|
| Name (30 max) | see block below |
| Subtitle (30 max) | see block below |
| Bundle ID | `com.onrecord.studio` |
| SKU | `onrecord-studio-mac` |
| Primary language | English (U.S.) |
| Primary category | **Productivity** (matches `tauri.appstore.conf.json`) |
| Secondary category | **Reference** |
| Content Rights | **Yes**, it contains, shows or accesses third-party content, and **Yes**, you have the rights to use it. The app retrieves and displays public records that users choose to collect. |
| License Agreement | Apple's standard EULA |
| Privacy Policy URL | `https://writerslogic.com/privacy/#onrecord-studio` |
| User Privacy Choices URL | leave empty |

Name (15/30):

```
OnRecord Studio
```

Subtitle (29/30):

```
Public records, on the record
```

> **OWNER, before submitting:** section 6a of the privacy page describes account, trial, license,
> device and seat records without saying they apply only to the direct download, and it doesn't
> mention the App Store. Add one sentence: the Mac App Store edition has no account, and its
> purchase is verified by Apple. Otherwise the policy contradicts the App Privacy answers below.

## 4. Pricing and Availability

| Field | Value |
|---|---|
| Price | **OWNER.** A one-time paid app. The code enforces a paid `AppTransaction` and has no in-app purchases or subscriptions, so pick a single price tier. The direct build's $99/seat/year has no App Store equivalent; a one-time price is a separate decision. |
| Availability | **OWNER.** All countries, or the United States only. The deadline model covers only the California Public Records Act, though the rest of the app is jurisdiction-neutral. |
| Pre-orders | Off |
| Distribution | Public |
| Tax category | App Store software (the default) |
| EU Digital Services Act | **OWNER.** To sell in the EU you must declare trader status: as an individual trader, your address, phone and email are shown publicly. Declare "not a trader" only if that is true. Otherwise exclude EU storefronts. |
| Apple silicon only | Nothing to set. The build is arm64-only (`--target aarch64-apple-darwin`) and the store hides it from Intel Macs. |

## 5. App Privacy

**Privacy Policy URL:** `https://writerslogic.com/privacy/#onrecord-studio`

**Data collection: answer "Yes, we collect data"** and declare exactly one type:

| Data type | Used for | Linked to the user | Used for tracking |
|---|---|---|---|
| **User Content, then Other User Content** | App Functionality | No | No |

Why this is the accurate answer:

- **Nothing reaches the developer.**
  - The App Store build has no account, analytics, crash reporter or update check. The WritersLogic
    account and Supabase code is compiled only into the direct build (`licensing.rs:11-20`).
  - The fetcher's User-Agent carries only the app version.
- **The one case where content leaves the Mac:** if the user turns on Autonomous research (off by
  default) and supplies their own OpenAI or OpenRouter key.
  - Excerpts of collected records, query results and case questions go to that provider
    (`research/model.rs`), with personal information masked on a best-effort basis.
  - This is an integration you wrote. Declaring it is the conservative reading of Apple's
    definition, which counts data sent to a third party you integrated.
- **What doesn't count as collection:**
  - Fetching public URLs the user enters, and the one-time semantic-model download from
    huggingface.co, send no user data beyond the request itself.
  - Purchase verification goes to Apple through StoreKit.

If you decide the user's own API key makes the provider the user's relationship rather than yours,
"Data Not Collected" is defensible. The table above is the safer answer.

## 6. Version 1.0 page (text)

Promotional Text (154/170; it can be changed at any time without review):

```
Collect public records, read them on your Mac, and keep every finding tied to the exact passage it came from. No account, no cloud copy of your documents.
```

Description (2861/4000):

```
OnRecord Studio turns the public records you collect into findings you can defend. Point it at a public records portal, an open-data catalog, a website, or files on your Mac. It downloads and reads the documents locally, searches them, looks for patterns, and ties every claim it shows you to a quoted passage in a source document.

BUILT FOR PEOPLE WHO HAVE TO SHOW THEIR WORK
Reporters, researchers, attorneys, advocates and civic watchdogs use public records to hold institutions accountable. A conclusion is only as good as its citation, so OnRecord Studio never shows a finding without the document, character range and quoted text behind it.

COLLECT
- Add a public URL, local files or pasted text. Only public addresses are fetched; the app never asks for a portal login or password.
- Reads PDFs, Word, Excel, email (.eml and .msg), HTML, RTF, plain text and ZIP archives.
- Imports open-data tables so you can query them alongside the documents.

READ AND SEARCH
- Full-text search across request metadata and extracted document text, with filters for agency, portal, status and flags.
- A document viewer that opens straight to the cited passage.
- Optional semantic search and read-only SQL over imported tables in the Analysis view.

FIND PATTERNS
- A findings inbox ranks what the detectors surfaced, such as reused denial wording, gaps in numbered document series, and agencies missing their own stated response times.
- Each finding says plainly whether it is a pattern worth review or a counted legal claim, and whether its confidence was measured or is unassessed.
- An entity graph and cross-document connections show who and what recurs across the record.

TRACK YOUR OWN REQUESTS
- Track the public records requests you filed elsewhere and count California Public Records Act deadlines from dates you confirm.
- Import notification emails to suggest timeline events; nothing is counted until you confirm it.
- Draft appeal text for you to send yourself. The app never files, sends or contacts an agency on your behalf.

EXPORT EVIDENCE OTHERS CAN CHECK
- Verifiable HTML reports that check their own signature in the reader's browser.
- Evidence packets in Markdown and JSON listing each claim's sources, quoted excerpts, confidence and what it does not show.
- Briefing outlines with evidence locators, source addresses and file hashes.
- Read-only research snapshots for Python, R or SQL.

PRIVATE BY DESIGN
Your documents, extracted text, indexes, findings, drafts and case files stay on your Mac. There is no account, no analytics and no advertising. The app goes online only to fetch the sources you add, to download the optional semantic-search model once, and for an optional AI research feature that is off until you turn it on.

One purchase. No subscription and no sign-in.

Requires a Mac with Apple silicon and macOS 13 or later.
```

Keywords (98/100; separated by commas without spaces, and repeating no word from the name or subtitle):

```
FOIA,CPRA,records request,sunshine,open data,journalism,investigative,evidence,agency,transparency
```

| Field | Value |
|---|---|
| Support URL | `https://github.com/writerslogic/support` (the Help "Support" button opens it; the opener permission already allows it) |
| Marketing URL | `https://writerslogic.com/onrecord/` |
| Version | `1.0.0` |
| Copyright | `2026 WritersLogic` (the bundle says "Copyright © 2026 WritersLogic"; Apple adds the ©). **OWNER:** this holder must match the seller shown on the store. If WritersLogic is not your legal seller name, use your own legal name. |
| What's New | Not shown for a first version |

## 7. Screenshots and icon

**Screenshots: OWNER, with help from me.** You will capture these in the running app once the UI
work in main has settled. Requirements, from Apple's screenshot specifications:

- 1 to 10 images, 16:10, at exactly **2880×1800**, 2560×1600, 1440×900 or 1280×800. Use PNG or
  JPEG with no transparency.
- They must show the actual app. Captions and a background frame are allowed.
- Use invented or genuinely public data. Real records appear publicly in the store.

Procedure:

1. I build the app and launch it, then set the window to exactly 1440×900 points with
   `osascript`, so a Retina capture is 2880×1800.
2. You capture a window: Cmd-Shift-4, then Space, then Option-click the window to leave out the
   shadow. Save the captures to `docs/app-store/screenshots/raw/`.
3. I check each image's dimensions and alpha, and trim or pad to 2880×1800 where needed.

Suggested shots, in order (the first two appear in search results):

1. **Findings inbox** with a legal claim and pattern signals visible. "Every finding tied to the
   passage it came from."
2. **Evidence** with a document open at a cited passage. "Read the source, not a summary."
3. **Patterns / entity graph.** "See who and what recurs across the record."
4. **My requests** with a deadline countdown. "Track deadlines from dates you confirm."
5. **Analysis** with a SQL result over an imported table. "Query open data alongside documents."
6. **Verifiable report** export dialog. "Exports anyone can check."

A capture harness renders the real views with invented records, so the shots don't need real
data. It lives outside the repo at `~/.config/onrecord-studio/app-store-tools/`:

- `__sample_answers.ts`: typed sample data for 45 IPC commands.
- `a11y-audit.mjs`: axe-core audit, with optional screenshots.
- `sweep.mjs`: keyboard and button sweep.

To use it, copy `__sample_answers.ts` to `src/` in a worktree. The scripts import
`./src/__sample_answers` and need `axe-core` installed next to them
(`npm install --prefix <tools dir> axe-core@4.13.0`).

**App icon (done).** App Store Connect takes the Mac icon from the build's `icon.icns`; there is no
separate upload. The icon is the Meta-generated design, adjusted for macOS:

- The magenta-tinted shadow fringe was neutralised.
- It was fitted to Apple's macOS grid: an 824 px body centred on a 1024 canvas.
- The master is `docs/app-store/icon/app-icon-1024.png`.
- Every size in `src-tauri/icons/`, from 16 to 1024 including `icon.icns`, was regenerated from
  it with `npx tauri icon docs/app-store/icon/app-icon-1024.png -o src-tauri/icons`. Rerun that
  command after any change to the master.

The previous edge-to-edge icon was removed.

## 8. App Review Information

| Field | Value |
|---|---|
| Sign-in required | **No.** Untick "Sign-in required". |
| Contact first name | **OWNER** |
| Contact last name | **OWNER** |
| Contact phone | **OWNER** |
| Contact email | **OWNER** (use an address that is read daily) |
| Attachment | Optional: a small public PDF so the reviewer can try a local import without network access |

Notes (paste as is; under the 4000 limit):

```
OnRecord Studio is a paid, one-time-purchase app. No account, sign-in or demo credentials are needed, and there are no in-app purchases.

What it does: it collects public records the user points it at (public web addresses, open-data catalogs or local files), extracts and indexes them on this Mac, runs pattern checks, and ties every finding to a quoted passage in a source document. It never files requests or contacts any agency.

To try it (about 3 minutes, needs network access for step 1):
1. In the source bar at the top of the window, open the source picker and choose any entry under "San Diego sources" (for example "Regional WaterGIS"), then press Add. Alternatively, use the Add files button in the same bar and pick any PDF, Word or text file.
2. The Activity view (Cmd-5) shows the collection. When it finishes, the app opens a brief of new findings.
3. Open Evidence (Cmd-4), search for a word from the document, and open a result to read the extracted text.
4. On the first view (Cmd-1), the Findings inbox lists what the detectors surfaced. "Why we believe this" shows the citations.
5. Help (Cmd-Shift-/) documents every view.

Network use: only the addresses the user adds, an optional one-time model download for semantic search (Analysis > Initialize model, from huggingface.co), and an optional AI research feature that is off by default and uses the user's own OpenAI or OpenRouter key. There is no analytics or advertising.

Scanned documents are read on this Mac with Apple's Vision framework (PDF pages rendered with PDFKit). Nothing is installed and nothing is sent anywhere.
```

## 9. Age Rating

Answer every question **None** or **No**. Expected result: **4+**.

| Question | Answer | Basis |
|---|---|---|
| Violence, sexual content, profanity, horror, drugs, gambling (all frequency questions) | None | The app ships no such content. It shows public records the user chooses to collect. |
| Medical or treatment information | None | |
| Unrestricted web access | **No** | There is no in-app browser, iframe or second webview. The app fetches documents from addresses the user enters and shows extracted text, and links go to the system browser. |
| User-generated content | No | Users exchange no content through any service. Case packets move only through a folder the user picks. |
| Messaging and chat | No | |
| Advertising | No | |
| Contests, gambling, loot boxes | No | |
| Parental controls / age assurance | No | |

If Apple's questionnaire treats displaying fetched public web documents as unrestricted web access,
answer Yes and accept the 18+ rating it forces. That would be a reviewer's call, not something the
code can settle.

## 10. Accessibility (App Store accessibility nutrition labels)

On a Mac, Apple offers nine labels. Claim a label only if a user who relies on that feature can
complete every common task: launch, add a source, search, open a finding's evidence, read the
document and export. Apple can remove a label it finds false.

| Label | Claim? | Evidence |
|---|---|---|
| Dark Interface | **Yes** | Full dark appearance that follows the system (View, then Appearance, has System/Light/Dark). Every view was audited in both appearances. |
| Sufficient Contrast | **Yes** | Every UI colour is one of 14 semantic roles, defined once for each mode in `index.css` (`claude/palette`). `src/tests/contrast.test.ts` checks every text role against every surface: 4.5:1 in light and dark, and 7:1 for body text with Increase Contrast on. Control borders must reach 3:1. The same test fails if a component uses a raw palette colour. axe-core finds 0 contrast violations across all nine views in light, dark and both increased-contrast modes. |
| Reduced Motion | **Yes** | Follows the macOS Reduce Motion setting and has its own View-menu override. With it on, every animation and transition is disabled (`index.css`, `data-motion="reduce"`). |
| Differentiate Without Color Alone | **Yes** | Severity, legal claim versus pattern, and confidence are each stated in text ("Legal claim: counted violation", "95% measured", "Unassessed"), not only by colour. |
| VoiceOver | **After the hands-on pass** | Automated evidence (commit `07923a8`): axe-core finds 0 WCAG A/AA violations across all nine views in light and dark, with sample data loaded. Tab traversal reaches every control in every view (35–111 stops per view) with a visible focus ring. Dialogs are labelled and modal and return focus when closed. Landmarks are named, and the current view is announced. A sweep clicked every enabled button on each view's first screen (about 500 buttons) against sample data and found none that did nothing. One button was covered by another panel and couldn't be clicked; that is fixed. The sweep did not reach dialogs, expanded findings, the command palette, Help, the source dialogs or the document viewer, and "did something" is not the same as "did the right thing". The hands-on pass covers those. What remains is the spoken task pass below. |
| Voice Control | **After the hands-on pass** | axe `label-content-name-mismatch` is clean after the fixes, so each control's spoken name contains its visible label. Icon-only header buttons have spoken names ("Enable audio alerts", "Open local profile", "Open help for …", "Open my requests"). |
| Larger Text | Not offered on macOS | Apple doesn't offer this label for Mac apps. |
| Captions | Leave blank | The app has no video. |
| Audio Descriptions | Leave blank | The app has no video. |

Hands-on pass (about 20 minutes in the built app; tick each box before claiming the label):

- [ ] VoiceOver on (Cmd-F5). Using VO keys and Tab only: add a source from the header, wait for
  the sync, open the Findings inbox, expand "Why we believe this", open the cited document, find
  the quoted passage, export a verifiable report. Every control should announce a name and role.
- [ ] VoiceOver: open and close the command palette (Cmd-K) and Help (Cmd-Shift-/). Focus should
  return to where it was.
- [ ] Voice Control on (System Settings, Accessibility, Voice Control). Say "Show names", then do
  the same tasks by voice ("Click Add", "Click Refresh findings", "Click Dismiss").
- [ ] Full keyboard access (System Settings, Keyboard, Keyboard navigation): every view is reachable
  with Cmd-1 to Cmd-9, and every button with Tab.

## 11. Build, export compliance and release

| Field | Value |
|---|---|
| Export compliance | `ONRECORD_EXPORT_COMPLIANCE=exempt` sets `ITSAppUsesNonExemptEncryption = NO`. In App Store Connect, that means: yes, it uses encryption, and every use is exempt. |
| Encryption in the app | TLS through rustls (HTTPS); Ed25519 signing of exports and SHA-256 hashing (integrity and authentication); reading PDFs that use standard PDF encryption. There is no proprietary algorithm and no encryption of stored user data. **OWNER:** confirm you agree it's exempt; the legal answer is yours. |
| Version | Change `1.0.0-beta.1` to `1.0.0` in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` for the App Store build. `prepare-app-store.mjs --check` requires all three to match. The direct beta keeps its own version. |
| Build | `npm run build:app-store`, then `npm run validate:app-store-bundle`, then `npm run package:app-store`, then `xcrun altool --upload-app` (see `docs/release.md`). |
| Version release | **Manually release this version**, so you can check the live listing first. |
| TestFlight | Install the processed build through TestFlight and run the section 10 checklist and blockers 5 and 6 against it before submitting. |
