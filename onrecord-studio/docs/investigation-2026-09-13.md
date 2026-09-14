**Application investigation — September 13, 2026**

Follow-up: this is the original investigation snapshot. The subsequent [San Diego implementation](san-diego-focus.md) addresses these five defects and records the remaining City NextRequest compatibility gap.

Five defects were reproduced in the current working tree. They affect finding records, selecting the correct department, seeing ingestion results, reading long documents, and reopening evidence. Existing uncommitted implementation changes were preserved. This investigation makes no application-code changes.

Validation: the production frontend build, TypeScript, lint, IPC contract check (40 commands), 104 frontend tests, and 314 Rust tests pass. A separate diagnostic using the real Rust repository and an in-memory DuckDB database fails the expected metadata-search behavior. Browser workflow checks use the actual React application with a synthetic native bridge; they establish frontend behavior, not successful native ingestion. No installed desktop session or live portal ingestion was exercised.

1. **High priority: search omits request titles and descriptions.**

   A native diagnostic saved a request titled “Unicorn procurement” with description “Zephyr contracts” and no attached documents. Unfiltered search returned one record; searching `Unicorn` or `Zephyr` returned zero. The interface advertises searching request metadata, but the search predicate reads only `document_search_tokens` joined through documents. Request metadata never participates. Requests without extracted attachments consequently disappear from every nonempty text search, even when the investigator copies a word directly from the visible title.

   Source: [native search predicate](../src-tauri/src/storage/sqlite.rs), lines 2571–2592; [search interface](../src/components/filters/SearchFilter.tsx), lines 96–109. Repair by indexing and searching request metadata alongside extracted text, with explicit semantics for terms spanning the two. Keep a regression test for metadata-only requests.

2. **High priority: department selection mixes identities across jurisdictions.**

   With two synthetic agency profiles—`portal-a / 7 / Police A` and `portal-b / 7 / Health B`—the All jurisdictions department selector offered only Health B. Selecting it sent `agencyId: 7` and `portalKey: null`. The SQL applies that numeric ID across portals, so unrelated departments sharing the ID can match under one department's label.

   Source: [AgencySelector](../src/components/filters/AgencySelector.tsx), lines 14–38; native search, lines 2630–2647. The all-portals map is keyed only by numeric ID. Preserve `(portalKey, agencyId)` through option generation, selection, and the query.

3. **High priority: ingestion does not refresh the open request's documents.**

   Opened a synthetic request containing one document, made a second document available through the bridge, and delivered extraction, completion, and sync-stop events. Search ran once before ingestion and twice after sync stopped, but `list_request_documents` ran only once. The new document remained absent even after completion. Search itself also remained unchanged during the completion events before sync stopped.

   Source: [MainLayout](../src/components/dashboard/MainLayout.tsx), lines 69–91; [telemetry handling](../src/App.tsx), lines 330–350. The document-list effect depends on request identity, not changes to its documents. Add explicit, bounded refresh notifications for search and affected document lists after ingestion commits, retaining the existing stale-response guards.

4. **Medium priority: advancing a long document can produce a blank viewer with no back control.**

   Loaded a 100,000-character chunk, advanced to its second text page, then clicked Next chunk to load a final 40-character chunk. The backend response contained text, but the rendered text length became zero and Previous text page disappeared. The retained page index slices beyond the new chunk; the final chunk has only one page, hiding pagination. There is also no Previous chunk control.

   Source: [DocumentViewer](../src/components/documents/DocumentViewer.tsx), lines 50–83, 259, and 325–329. Reset or clamp the page index when the chunk offset/content changes; support returning to earlier chunks. Changing chunks within the same document currently bypasses the reset keyed on document identity.

5. **Medium priority: copied evidence links fail to reopen their document.**

   Loaded a fresh page with `#portal=fixture-gov&request=42&document=1&offset=100000`. The application selected request 42 and requested the correct text offset, but stayed on Overview with no document dialog. Switching to Evidence still did not open the document. This was a full navigation, not just an in-page fragment change.

   Source: [MainLayout](../src/components/dashboard/MainLayout.tsx), lines 63–91 and 156–169; [default workspace](../src/store/useAppStore.ts), line 161. Consuming the pending link immediately reruns document-list initialization, which invalidates the opening request and clears the active document. Link initialization also never selects the Evidence workspace. Handle link resolution as a coherent navigation operation independent of the current search result page.

**Launch behavior:** opening the Vite development server directly in Chrome reproduced source-catalog, analytics, OCR, and telemetry errors about missing native `invoke` / `transformCallback`. The browser is rendering the desktop UI without its Rust backend. For the desktop application, use `npm run tauri -- dev` from the nested `onrecord-studio` package, or `npm --prefix onrecord-studio run tauri -- dev` from the repository root. A browser preview should explain this dependency instead of presenting it as a failed local database load.

Recommended repair order: metadata search and department identity first, ingestion refresh next, then document pagination and evidence-link navigation. Preserve these reproductions as behavioral regression tests when implementing the fixes; the existing passing suites do not cover these cases.

Local diagnostic artifacts are retained in `/tmp/onrecord-investigation-workflows.json`, `/tmp/onrecord-investigation-workflows.mjs`, `/tmp/onrecord-investigation-fixture.js`, `/tmp/onrecord-investigation-metadata.rs`, and `/tmp/onrecord-investigation-metadata.log`. The temporary Rust test was removed from the package after reproducing the failure. Test/build logs use the same `/tmp/onrecord-investigation-` prefix.
