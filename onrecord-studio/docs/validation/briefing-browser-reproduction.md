# Reproduce the saved-dossier browser review

This harness renders the production `OverviewDashboard`, including `ResearchWorkspace` and the investigation briefing. Its only mocked command is the read-only `get_research_status`, supplied with a reviewed saved dossier. Other commands fail. It does not connect to the native database, run research, use API keys, ingest sources, or send requests. The screenshots demonstrate component rendering and navigation, not native end-to-end operation or an independent verification of the dossier's conclusions.

Review and accept the live dossier before choosing it as input. Do not substitute a known failed attempt just to obtain screenshots. The JSON result records the input path, SHA-256, dossier ID and creation time, so the visual record can be traced to the exact reviewed artifact.

Run these commands from the application directory. An existing Playwright installation can be used by setting `PLAYWRIGHT_MODULE` to its `index.mjs`; otherwise install browser automation tools into a temporary directory. This does not change application dependencies.

```sh
briefing_tools=$(mktemp -d)
npm install --prefix "$briefing_tools" playwright-core
export PLAYWRIGHT_MODULE="$briefing_tools/node_modules/playwright-core/index.mjs"
export CHROME_BIN='/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'

# Reviewed live dossier used for the September 27 final browser record.
accepted_report='docs/validation/investigation-depth-live-2026-09-27.json'
node scripts/verify-investigation-briefing.mjs "$accepted_report" docs/validation/investigation-depth-briefing-browser-2026-09-27
```

On another operating system, set `CHROME_BIN` to an installed Chromium-compatible browser. `BRIEFING_REVIEW_PORT` optionally overrides the default localhost port 5197. The script starts Vite itself, uses a fresh headless browser profile, and deletes its temporary HTML, TSX and JSON fixture files when it finishes. No private workspace paths are required in the committed recipe.

The script verifies:

- The default finding belongs to the supplied saved dossier.
- The visible citation matches the saved quotation and its original-source link belongs to the saved evidence.
- The status line shows **Latest saved dossier**, derived from the saved report.
- The targeted request opens with its original prepared draft, and mobile navigation opens that same request.
- Desktop (1440 × 1000) and mobile (390 × 844) have no horizontal overflow or browser runtime errors.

It writes desktop, mobile and targeted-request PNGs plus a JSON check record. Inspect the images as well as the numerical bounds: verify headline readability, the prominence of the next action, relevant source access and the absence of misleading live-status claims. The report's wording and request relevance still require editorial review. The older September 27 briefing screenshots used prior archived dossiers; their input is separate from later live-run validation.

## Accepted live dossier: September 27, 2026

The final review used dossier `82083f08-4af4-4ea3-aa2d-fa66a175d803`, created at `2026-09-27T08:59:43.995881+00:00`. Its JSON SHA-256 is `8345c426951e2542fef0a165a9196c10f03d01c83df1ff01ac15b8873f1b96b4`.

The default finding concerns the comparability of Police costs within the FY2026 and FY2027 homelessness funding reports. Its visible quotation and original-source link matched the saved evidence. The corresponding **FY2026-to-FY2027 homelessness funding scope crosswalk** request opened with its exact prepared draft on desktop and mobile. The **Latest saved dossier** timestamp matched the actual report creation time.

The request button appeared at y=420 on desktop and y=659 on mobile, fully inside the first viewport. The selected request opened at y=271 on desktop. Both viewports had no horizontal overflow, and the browser reported no runtime errors. Visual inspection confirmed readable headings, evidence and draft text; the desktop displayed the main finding, alternative explanation, next test, source quotation and request action together. On mobile, the headline and request action appeared in the initial viewport, with detailed evidence available by scrolling. Temporary harness files and the local Vite server were removed afterward.

- [Desktop screenshot](investigation-depth-briefing-browser-2026-09-27-desktop.png)
- [Mobile screenshot](investigation-depth-briefing-browser-2026-09-27-mobile.png)
- [Targeted request screenshot](investigation-depth-briefing-browser-2026-09-27-request.png)
- [Exact input and browser checks](investigation-depth-briefing-browser-2026-09-27.json)
