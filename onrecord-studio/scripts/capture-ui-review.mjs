// Captures every workspace view and major panel of the real <App/> for UI review, driven by a
// stubbed IPC bridge (scripts/ui-review-fixtures.mjs). `--data=real` answers every surface the San
// Diego packs hold from scripts/ui-review-fixtures-real.mjs (built by build-real-ui-fixtures.mjs);
// the default serves invented data throughout. Browser only: no native application, network, or
// model is involved.
//
//   node scripts/capture-ui-review.mjs [outputDirectory] [--data=real] [--only=view,view] [--concurrency=3]
//     [--widths=1024,1280,1680] [--states=populated,empty,loading,error] [--browser=chrome|bundled]
//   node scripts/capture-ui-review.mjs <dir> --only=chart-gallery   (every chart primitive, harness-only)
//
// --states limits every view to those states; --states=top keeps the full state set for the
// top-level workspace views and only populated for the rest (the CI subset).
//
// Output: <view>__<state>__<theme>__<width>.png plus index.json (steps, console errors, failed
// requests, stub gaps, SQL the real fixtures could not answer, axe-core violations per capture and
// per view, and a keyboard-focus-order record per view). Exits non-zero only when the harness
// itself fails (server, browser, app shell unreachable), never on findings.
//
// Playwright resolves from PLAYWRIGHT_MODULE, then the project's devDependency, then the Homebrew
// copy. The browser is CHROME_BIN when set, else installed Google Chrome on macOS, else Playwright's
// bundled Chromium (`npx playwright install chromium`); --browser=bundled forces the last.
/* global document, window, HTMLElement, CSS, innerHeight, innerWidth, location, requestAnimationFrame -- page.evaluate bodies run in the browser */
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { FIXTURE_NOW } from './ui-review-fixtures.mjs';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const argv = process.argv.slice(2);
const flag = (name) => argv.find((arg) => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? null;
const positional = argv.filter((arg) => !arg.startsWith('--'));
const gitHead = (() => {
  try {
    return execFileSync('git', ['rev-parse', '--short', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
  } catch {
    return 'unknown';
  }
})();
const label = flag('label') ?? `${gitHead}-${new Date().toISOString().slice(0, 16).replace(/[-:T]/g, '')}`;
const outputDirectory = path.resolve(positional[0] ?? path.join('/Volumes/A/onrecord-ui-review', label));
const only = flag('only')?.split(',').filter(Boolean) ?? null;
const concurrency = Math.max(1, Number(flag('concurrency') ?? 3));
const data = flag('data') ?? 'invented';
if (!['invented', 'real'].includes(data)) throw new Error(`--data must be "invented" or "real", not "${data}"`);
const real = data === 'real' ? await import('./ui-review-fixtures-real.mjs') : null;
const HOMEBREW_PLAYWRIGHT = '/opt/homebrew/lib/node_modules/@playwright/mcp/node_modules/playwright/index.mjs';
const MAC_CHROME = '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
const browserChoice = flag('browser') ?? 'chrome';
if (!['chrome', 'bundled'].includes(browserChoice)) throw new Error(`--browser must be "chrome" or "bundled", not "${browserChoice}"`);
const chromeBin = browserChoice === 'bundled' ? null
  : process.env.CHROME_BIN ?? (process.platform === 'darwin' && existsSync(MAC_CHROME) ? MAC_CHROME : null);

const THEMES = ['light', 'dark'];
const ALL_SIZES = [[1024, 700], [1280, 800], [1680, 1050]];
const widths = flag('widths')?.split(',').filter(Boolean).map(Number) ?? null;
const SIZES = widths ? ALL_SIZES.filter(([width]) => widths.includes(width)) : ALL_SIZES;
if (widths && SIZES.length !== widths.length) throw new Error(`--widths takes ${ALL_SIZES.map(([width]) => width).join(', ')}, not "${flag('widths')}"`);
const STATE_NAMES = ['populated', 'empty', 'loading', 'error'];
const statesFlag = flag('states');
const stateFilter = statesFlag === null || statesFlag === 'top' ? null : statesFlag.split(',').filter(Boolean);
if (stateFilter?.some((state) => !STATE_NAMES.includes(state))) throw new Error(`--states takes "top" or ${STATE_NAMES.join(', ')}, not "${statesFlag}"`);
// Real data gives automatic discovery ~90 queries to finish before a capture.
const SETTLE_TIMEOUT_MS = real ? 60_000 : 15_000;
const QUIET_MS = 350;

const stem = `__ui_review_${process.pid}_${Date.now()}`;
const entryFile = path.join(root, `${stem}.tsx`);
const htmlFile = path.join(root, `${stem}.html`);
const galleryEntryFile = path.join(root, `${stem}-gallery.tsx`);
const galleryHtmlFile = path.join(root, `${stem}-gallery.html`);

// The entry installs the IPC stub before the app's modules evaluate, then boots src/main.tsx
// unchanged so the capture is of the production component tree.
// Automatic discovery paces its queries one per 250 ms tick (~90 with real data, over 20 s per page
// load); the entry drops that pause, so a page reaches the same settled state in a fraction of the
// time. Its other delays (idle, retry, hidden) are left alone.
const entry = `const pacedSetTimeout = window.setTimeout.bind(window);
window.setTimeout = (callback, delay, ...rest) => pacedSetTimeout(callback, delay === 250 && new Error().stack?.includes('/src/utils/discoveryController') ? 0 : delay, ...rest);
import { createHandlers, injectedError } from './scripts/ui-review-fixtures.mjs';
const scenario = JSON.parse(new URLSearchParams(location.search).get('scenario') || '{}');
const handlers = createHandlers(scenario, scenario.data === 'real' ? await import('./scripts/ui-review-fixtures-real.mjs') : null);
const review = { calls: [], gaps: [], handlerErrors: [], sqlMisses: [], injected: [], hung: [], plugins: [], inflight: 0, lastActivity: performance.now() };
window.__uiReview = review;
const callbacks = new Map();
let nextId = 1;
const pluginAnswers = { 'plugin:app|version': '1.0.0', 'plugin:app|name': 'OnRecord Studio', 'plugin:app|tauri_version': '2.0.0' };
window.isTauri = true;
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
window.__TAURI_INTERNALS__ = {
  metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
  transformCallback(callback) { const id = nextId++; callbacks.set(id, callback); return id; },
  unregisterCallback(id) { callbacks.delete(id); },
  convertFileSrc(filePath) { return filePath; },
  async invoke(command, args) {
    if (command.startsWith('plugin:')) {
      review.plugins.push(command);
      if (command === 'plugin:event|listen') return nextId++;
      return pluginAnswers[command] ?? null;
    }
    review.calls.push(command);
    if ((scenario.hang || []).includes(command)) {
      review.hung.push(command);
      return new Promise(() => {});
    }
    review.inflight += 1;
    try {
      await new Promise((resolve) => setTimeout(resolve, 0));
      if ((scenario.fail || []).includes(command)) {
        review.injected.push(command);
        throw injectedError(command);
      }
      const handler = handlers[command];
      if (!handler) {
        review.gaps.push(command);
        throw { errorType: 'UI_REVIEW_STUB_GAP', message: 'The UI review harness has no fixture for ' + command };
      }
      let result;
      try {
        result = handler(args || {});
      } catch (error) {
        if (error?.errorType === 'UI_REVIEW_SQL_MISS') review.sqlMisses.push(args.sql);
        else review.handlerErrors.push(command + ': ' + String(error));
        throw error;
      }
      return result instanceof ArrayBuffer ? result.slice(0) : structuredClone(result);
    } finally {
      review.inflight -= 1;
      review.lastActivity = performance.now();
    }
  },
};
const { useAppStore } = await import('./src/store/useAppStore');
const { useProfileLensStore } = await import('./src/store/useProfileLensStore');
review.store = useAppStore;
review.lens = useProfileLensStore;
await import('./src/main');
`;
// The chart gallery (src/components/charts/ChartGallery.tsx) is harness-only: it renders without
// the app shell, so it never appears in the app's navigation or bundle. The stub keeps settle(),
// drain() and applyTheme() working without a store or IPC.
const galleryEntry = `const scenario = JSON.parse(new URLSearchParams(location.search).get('scenario') || '{}');
const review = { calls: [], gaps: [], handlerErrors: [], sqlMisses: [], injected: [], hung: [], plugins: [], inflight: 0, lastActivity: performance.now() };
review.store = { getState: () => ({ setTheme: (theme) => { document.documentElement.dataset.theme = theme; } }) };
window.__uiReview = review;
document.documentElement.dataset.theme = scenario.theme || 'light';
document.documentElement.dataset.motion = 'reduce';
document.documentElement.dataset.contrast = 'standard';
await import('./src/index.css');
const { createElement } = await import('react');
const { createRoot } = await import('react-dom/client');
const { default: ChartGallery } = await import('./src/components/charts/ChartGallery');
for (const element of [document.documentElement, document.body, document.getElementById('root')]) {
  element.style.height = 'auto';
  element.style.overflow = 'visible';
}
createRoot(document.getElementById('root')).render(createElement(ChartGallery));
`;
const html = `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0"><title>OnRecord Studio</title><link rel="icon" href="data:,"></head><body><div id="root"></div><script type="module" src="/${stem}.tsx"></script></body></html>`;
const galleryHtml = html.replace(`/${stem}.tsx`, `/${stem}-gallery.tsx`);

/** Runs before any page script on every navigation: storage the store reads at module evaluation. */
function initScript(fixtureNow) {
  const scenario = JSON.parse(new URLSearchParams(location.search).get('scenario') || '{}');
  try {
    localStorage.clear();
    localStorage.setItem('onrecord-theme', scenario.theme || 'light');
    localStorage.setItem('onrecord-motion', 'reduce');
    localStorage.setItem('onrecord-contrast', 'system');
    localStorage.setItem('onrecord-last-opened-at', new Date(fixtureNow - 2 * 86_400_000).toISOString());
    if (!scenario.firstRun) localStorage.setItem('onrecord-first-run-data', 'settled');
  } catch {
    // Storage unavailable: the app falls back to its defaults.
  }
  window.isTauri = true;
}

// ---- View catalogue -------------------------------------------------------------------------

const NAV_LABELS = { overview: 'San Diego', patterns: 'Patterns', analysis: 'Analysis', evidence: 'Evidence', activity: 'Activity', briefing: 'Briefing', story: 'Story desk', meetings: 'Meetings', compare: 'Compare' };

/** Steps run against a page; each records what it did in `log`. */
const nav = (view) => async (page, log) => {
  const button = page.getByRole('navigation', { name: 'Primary navigation' }).getByRole('button', { name: new RegExp(`^\\s*${NAV_LABELS[view]}\\b`) });
  try {
    await button.click({ timeout: 2_000 });
    log.push(`click sidebar "${NAV_LABELS[view]}"`);
  } catch (error) {
    await page.evaluate((target) => window.__uiReview.store.getState().setWorkspaceView(target), view);
    const reason = String(error?.message ?? error).split('\n').find((line) => /intercepts|not visible|disabled|outside/.test(line))?.trim() ?? 'timed out';
    log.push(`store.setWorkspaceView('${view}') (sidebar button not clickable: ${reason})`);
  }
};
const click = (description, locate) => async (page, log) => {
  await locate(page).filter({ visible: true }).first().click({ timeout: 8_000 });
  log.push(`click ${description}`);
};
const type = (description, locate, text) => async (page, log) => {
  await locate(page).filter({ visible: true }).first().fill(text, { timeout: 8_000 });
  log.push(`type "${text}" into ${description}`);
};
const store = (description, fn, arg) => async (page, log) => {
  await page.evaluate(fn, arg);
  log.push(description);
};
const select = (description, locate, value) => async (page, log) => {
  await locate(page).filter({ visible: true }).first().selectOption(value, { timeout: 8_000 });
  log.push(`select "${value}" in ${description}`);
};
const event = (name, detail) => store(`dispatch window event '${name}'`, ([eventName, eventDetail]) => window.dispatchEvent(new CustomEvent(eventName, { detail: eventDetail })), [name, detail]);
const inspect = (target) => store(`store.inspect(${JSON.stringify(target)})`, (t) => window.__uiReview.store.getState().inspect(t), target);

// The overview's perspectives (Leads, Following, Analytics, Casework, Tools) are tabs in the main pane.
const overviewTab = (name) => click(`tab "${name}"`, (page) => page.getByRole('main').getByRole('tab', { name, exact: true }));
const casework = [nav('overview'), overviewTab('Casework')];
/** Opens the Watching pane when it is folded (its toggle's name starts with "Watching"). */
const openWatching = async (page, log) => {
  const toggle = page.getByRole('button', { name: /^Watching/ }).filter({ visible: true }).first();
  if ((await toggle.getAttribute('aria-expanded', { timeout: 8_000 })) === 'false') {
    await toggle.click({ timeout: 8_000 });
    log.push('click "Watching" (unfold)');
  }
};
const press = (key) => async (page, log) => {
  await page.keyboard.press(key);
  log.push(`press ${key}`);
};

const spendingTab = [
  ...casework,
  click('tab "Spending"', (page) => page.getByRole('main').getByRole('tab', { name: 'Spending', exact: true })),
];
const spendingChains = click('"Spending chains"', (page) => page.getByRole('button', { name: 'Spending chains', exact: true }));
const budgetComparison = [
  nav('analysis'),
  type('the SQL editor', (page) => page.getByRole('textbox', { name: 'Analytical SQL' }), 'SELECT * FROM budget_comparison -- fund 100000, FY25: actual_minus_budget'),
  click('"Run"', (page) => page.getByRole('button', { name: 'Run', exact: true })),
];
const measure = click('"Measure"', (page) => page.getByRole('button', { name: 'Measure', exact: true }));

const selectRequest = click('evidence result "Encampment abatement notices…"', (page) => page.getByRole('main').getByText(/Encampment abatement notices posted along Imperial/));
const openDocument = click('document "ESD_Response_Letter_26-0412.pdf"', (page) => page.getByRole('button', { name: /ESD_Response_Letter_26-0412\.pdf/ }));

const sqlEditor = (page) => page.getByLabel('Analytical SQL');

/**
 * `states` lists the variants beyond populated. `anchor`, when set, is scrolled to the top of the
 * viewport before every screenshot, since each width reflows the page under a fixed scroll offset. `primary` are the commands a loading state hangs
 * and an error state fails: the ones whose answer the view's main content waits on.
 */
const VIEWS = [
  { view: 'overview', steps: [nav('overview')], primary: ['list_saved_portals'], states: ['empty', 'loading', 'error'], discover: true },
  { view: 'story', steps: [nav('story')], primary: ['list_story_leads'], states: ['empty', 'loading', 'error'], discover: true },
  { view: 'patterns', steps: [nav('patterns')], primary: ['query_corpus_patterns', 'get_corpus_index_status'], states: ['empty', 'loading', 'error'], discover: true },
  { view: 'analysis', steps: [nav('analysis')], primary: ['get_analytical_schema', 'get_embedding_model_status', 'get_embedding_index_status'], states: ['empty', 'loading', 'error'], discover: true },
  { view: 'evidence', steps: [nav('evidence')], primary: ['query_audit_records_page', 'count_audit_records'], states: ['empty', 'loading', 'error'] },
  { view: 'evidence-timeline', steps: [nav('evidence'), click('evidence mode "Timeline"', (page) => page.getByRole('main').getByRole('button', { name: 'Timeline', exact: true }))], primary: ['list_claimed_requests', 'get_casework_state'], states: ['empty', 'loading', 'error'] },
  { view: 'activity', steps: [nav('activity')], primary: ['list_portal_runtimes'], states: ['empty', 'loading', 'error'] },
  { view: 'briefing', steps: [nav('briefing')], primary: ['list_findings'], states: ['empty', 'loading', 'error'] },
  { view: 'meetings', steps: [nav('meetings')], primary: ['get_meeting_calendar'], states: ['empty', 'loading', 'error'] },
  { view: 'compare', steps: [nav('compare')], primary: ['get_agency_performance_profiles'], states: ['empty', 'loading', 'error'] },

  { view: 'evidence-request-detail', steps: [nav('evidence'), selectRequest], primary: ['list_request_documents', 'list_request_legal_findings'], states: ['loading', 'error'] },
  { view: 'evidence-document-viewer', steps: [nav('evidence'), selectRequest, openDocument], primary: ['get_document_text_chunk'], states: ['loading', 'error'] },
  { view: 'evidence-authority-library', steps: [nav('evidence'), selectRequest, click('"Authority library"', (page) => page.getByRole('button', { name: 'Authority library' }))], primary: ['list_legal_authorities'], states: ['error'] },
  { view: 'overview-response-analytics', steps: [nav('overview'), overviewTab('Analytics')], primary: ['get_agency_performance_profiles', 'get_cross_agency_correlations'], states: ['error'] },
  { view: 'overview-casework-tools', steps: casework, primary: ['get_casework_state', 'get_research_status'], states: ['empty', 'error'], discover: true },
  // Discovery does not look inside a drawer or a tab, so surfaces nested there are named here.
  { view: 'overview-casework-tools-drawer-assess-usefulness', steps: [...casework, click('tab "Evidence review"', (page) => page.getByRole('main').getByRole('tab', { name: 'Evidence review', exact: true })), click('drawer "Assess usefulness"', (page) => page.getByRole('main').locator('details > summary', { hasText: 'Assess usefulness' }))], primary: [], states: [] },
  { view: 'patterns-tab-ledger-history', steps: [nav('patterns'), click('drawer "Resolve entities"', (page) => page.getByRole('main').locator('details.app-panel > summary', { hasText: 'Resolve entities' })), click('tab "Ledger history"', (page) => page.getByRole('main').getByRole('tab', { name: 'Ledger history', exact: true }))], primary: [], states: [] },
  { view: 'patterns-tab-import-and-export', steps: [nav('patterns'), click('drawer "Resolve entities"', (page) => page.getByRole('main').locator('details.app-panel > summary', { hasText: 'Resolve entities' })), click('tab "Import and export"', (page) => page.getByRole('main').getByRole('tab', { name: 'Import and export', exact: true }))], primary: [], states: [] },
  { view: 'overview-casework-case', steps: [...casework, async (page, log) => {
    await page.getByRole('combobox', { name: 'Open saved case' }).filter({ visible: true }).first().selectOption('case-storage', { timeout: 8_000 });
    log.push('select saved case "Storage contract ceiling"');
    await page.getByText('Request determination clocks').first().scrollIntoViewIfNeeded({ timeout: 8_000 });
    log.push('scroll to "Request determination clocks"');
  }], primary: ['get_casework_state'], states: [] },
  { view: 'overview-subject-dossier', steps: [nav('overview'), store("store.openSubjectDossier({ kind: 'entity', value: 'e-org-harborview' })", () => window.__uiReview.store.getState().openSubjectDossier({ kind: 'entity', value: 'e-org-harborview' }))], primary: ['get_corpus_entity_neighborhood'], states: ['error'] },
  { view: 'overview-post-ingest-brief', steps: [nav('overview'), store('store.setPostIngestBriefOpen(true)', () => window.__uiReview.store.getState().setPostIngestBriefOpen(true))], primary: [], states: [] },
  { view: 'overview-sidebar-collapsed', steps: [nav('overview'), store('store.toggleSidebarCollapsed()', () => window.__uiReview.store.getState().toggleSidebarCollapsed())], primary: [], states: [] },

  { view: 'inspector-finding', steps: [nav('overview'), inspect({ kind: 'finding', id: 'f-contract-ceiling' })], primary: ['build_lead_dossier'], states: ['loading', 'error'] },
  { view: 'inspector-entity', steps: [nav('overview'), inspect({ kind: 'entity', entityId: 'e-org-harborview', label: 'Harborview Shelter Partners' })], primary: ['get_corpus_entity_neighborhood'], states: ['error'] },
  { view: 'inspector-document', steps: [nav('overview'), inspect({ kind: 'document', portalKey: 'sandiego', documentId: 9002, requestId: 260412, characterStart: 610, characterEnd: 644, label: 'ESD_Response_Letter_26-0412.pdf' })], primary: ['get_document_text_chunk'], states: ['error'] },
  { view: 'inspector-agency', steps: [nav('overview'), inspect({ kind: 'agency', portalKey: 'sandiego', agencyId: 102, label: 'Environmental Services Department' })], primary: ['list_findings'], states: ['error'] },
  { view: 'inspector-request', steps: [nav('overview'), inspect({ kind: 'request', portalKey: 'sandiego', requestId: 260412, label: 'Request 26-0412' })], primary: ['list_request_documents'], states: ['error'] },
  { view: 'inspector-case', steps: [nav('overview'), inspect({ kind: 'case', caseId: 'case-storage', label: 'Storage contract ceiling' })], primary: ['get_casework_state'], states: ['error'] },

  { view: 'command-palette', steps: [nav('overview'), event('onrecord:open-command-palette', null)], primary: [], states: [] },
  // The passage search never answers, so every capture shows the palette while it searches.
  { view: 'command-palette-pending', scenario: { hang: ['hybrid_search'] }, steps: [nav('overview'), event('onrecord:open-command-palette', null), type('command palette search', (page) => page.getByRole('dialog').getByRole('combobox'), 'harborview')], primary: [], states: [] },
  { view: 'overview-ledger-stage', steps: [nav('overview'), openWatching, click('Watching "Deadline ledger"', (page) => page.getByRole('button', { name: /^Deadline ledger/ }))], primary: [], states: [] },
  // Each of these scrolls one region to its target; the shell assertion after every step proves
  // none of them scrolled the document.
  { view: 'overview-ledger-in-analytics', steps: [nav('overview'), openWatching, click('Watching "Deadline ledger"', (page) => page.getByRole('button', { name: /^Deadline ledger/ })), click('"Open in response analytics"', (page) => page.getByRole('button', { name: 'Open in response analytics' }))], primary: [], states: [] },
  { view: 'overview-stage-draft', steps: [nav('overview'), click('stage "Draft records request"', (page) => page.getByRole('main').getByRole('button', { name: 'Draft records request', exact: true }))], primary: [], states: [] },
  { view: 'analysis-budget-from-palette', steps: [nav('overview'), event('onrecord:open-command-palette', null), type('command palette search', (page) => page.getByRole('dialog').getByRole('combobox'), 'budget versus'), click('option "Analysis: Budget versus actuals"', (page) => page.getByRole('option', { name: /Analysis: Budget versus actuals/ }))], primary: [], states: [] },
  { view: 'overview-lead-3', steps: [nav('overview'), press('j'), press('j')], primary: [], states: [] },
  { view: 'overview-watching-collapsed', steps: [nav('overview')], widths: [1024], primary: [], states: [] },
  { view: 'story-desk-workbench', steps: [nav('story')], primary: [], states: [] },
  { view: 'command-palette-question', steps: [nav('overview'), event('onrecord:open-command-palette', null), type('command palette search', (page) => page.getByRole('dialog').getByRole('combobox').or(page.getByRole('dialog').getByRole('textbox')), 'findings for Environmental Services'), async (page, log) => { await page.keyboard.press('Enter'); log.push('press Enter'); }], primary: ['run_intent_query'], states: ['error'] },
  { view: 'help-center', steps: [nav('overview'), event('onrecord:open-help', {})], primary: [], states: [] },
  { view: 'my-requests', steps: [nav('overview'), click('"Open my requests"', (page) => page.getByRole('button', { name: 'Open my requests' }))], primary: ['list_claimed_requests'], states: ['empty', 'error'] },
  { view: 'my-requests-detail', steps: [nav('overview'), click('"Open my requests"', (page) => page.getByRole('button', { name: 'Open my requests' })), click('claimed request "26-0533"', (page) => page.getByRole('button', { name: /26-0533/ }))], primary: ['get_request_deadline'], states: [] },
  { view: 'profile-settings', steps: [nav('overview'), click('account menu trigger', (page) => page.getByRole('banner').getByRole('button', { name: 'Account' })), click('"Profile settings"', (page) => page.getByRole('button', { name: 'Profile settings' }))], primary: ['get_profile'], states: ['empty', 'error'] },
  { view: 'header-add-source-dialog', steps: [nav('overview'), click('header "Add source"', (page) => page.getByRole('banner').getByRole('button', { name: 'Add source' }))], primary: [], states: [] },
  { view: 'header-source-actions-menu', steps: [nav('overview'), click('header "Source actions"', (page) => page.getByRole('banner').getByRole('button', { name: 'Source actions' }))], primary: [], states: [] },
  { view: 'self-review', steps: [nav('overview'), store('profileLens.setSelfReviewOpen(true)', () => window.__uiReview.lens.getState().setSelfReviewOpen(true))], primary: ['list_findings'], states: ['empty'] },
  { view: 'account-menu', steps: [nav('overview'), click('account menu trigger', (page) => page.getByRole('button', { name: /reporter@example\.org|Account/ }))], primary: [], states: [] },
  { view: 'case-switcher', steps: [nav('overview'), click('case switcher trigger', (page) => page.getByRole('banner').getByRole('button', { name: /case/i }))], primary: ['get_casework_state'], states: ['empty'] },

  { view: 'collection-plan', scenario: { collectionPlan: true }, steps: [nav('overview')], primary: [], states: [] },
  { view: 'first-run', scenario: { firstRun: true, mode: 'empty' }, steps: [nav('overview')], primary: [], states: [] },
  { view: 'license-trial', scenario: { license: 'trial' }, steps: [nav('overview')], primary: [], states: [] },
  { view: 'license-trial-expired', scenario: { license: 'trialExpired' }, steps: [], gate: true, primary: [], states: [] },
  { view: 'license-sign-in-required', scenario: { license: 'signInRequired' }, steps: [], gate: true, primary: [], states: [] },
  { view: 'license-device-limit', scenario: { license: 'deviceLimit' }, steps: [], gate: true, primary: [], states: [] },
  // Row-level results only appear after a reader runs SQL; these two show the real tables' rows.
  ...(real ? [
    { view: 'analysis-sql-preview', steps: [nav('analysis'), type('SQL editor', (page) => page.getByLabel('Analytical SQL'), real.PREVIEW_SQL), click('"Run"', (page) => page.getByRole('button', { name: 'Run', exact: true }))], anchor: sqlEditor, primary: ['run_analytical_sql'], states: ['error'] },
    { view: 'analysis-budget-comparison', steps: [nav('analysis'),
      select('"Budget snapshot"', (page) => page.getByLabel('Budget snapshot'), real.BUDGET_COMPARISON.budgetSnapshotId),
      select('"Actuals snapshot"', (page) => page.getByLabel('Actuals snapshot'), real.BUDGET_COMPARISON.actualsSnapshotId),
      type('"Comparison fiscal year"', (page) => page.getByLabel('Comparison fiscal year'), real.BUDGET_COMPARISON.fiscalYear),
      click('"Compare budget and actuals"', (page) => page.getByRole('button', { name: 'Compare budget and actuals' }))], anchor: sqlEditor, primary: [], states: [] },
  ] : []),

  { view: 'chart-gallery', gallery: true, steps: [], primary: [], states: [] },

  // Figures below the fold: each also gets an element capture at full height (`sections`).
  { view: 'compare-dispositions', steps: [nav('compare')], sections: ['section[aria-label="Dispositions by department"]'], primary: [], states: [] },
  { view: 'overview-following', steps: [nav('overview'), overviewTab('Following')], sections: ['section[aria-label="Following"]'], primary: [], states: [] },
  // The More tools drawers live on the Tools tab; discovery opens each from there.
  { view: 'overview-tools', steps: [nav('overview'), overviewTab('Tools')], primary: [], states: [], discover: true },
  { view: 'story-ranked-leads', steps: [nav('story'), click('tab "Factor table"', (page) => page.getByRole('main').getByRole('tab', { name: 'Factor table', exact: true }))], sections: ['figure:has(h3:text-is("Ranked leads"))'], primary: [], states: [] },
  {
    view: 'organization-record',
    steps: [...casework, click('tab "Organization filings"', (page) => page.getByRole('main').getByRole('tab', { name: 'Organization filings', exact: true })), click('portfolio row "Harborview Shelter Partners"', (page) => page.getByRole('main').getByRole('button', { name: 'Harborview Shelter Partners', exact: true }))],
    sections: ['section[aria-label="Organization filings"]'],
    primary: ['get_organisation_filings'],
    states: [],
  },
  // Populated figures (scenario.figures): `anchor` is scrolled to the top of the viewport before each capture.
  { view: 'figures-reconcile', scenario: { figures: { payments: true } }, steps: [...spendingTab], anchor: (page) => page.getByRole('heading', { name: 'Actual against revised authority, by department' }), primary: [], states: [] },
  { view: 'figures-contracts', scenario: { figures: { payments: true } }, steps: [...spendingTab, spendingChains], anchor: (page) => page.getByRole('heading', { name: 'Contract authority and posted payments' }), primary: [], states: [] },
  { view: 'figures-contracts-no-payments', scenario: { figures: { payments: false } }, steps: [...spendingTab, spendingChains], anchor: (page) => page.getByRole('heading', { name: 'Contract authority and posted payments' }), primary: [], states: [] },
  { view: 'figures-budget-dots', scenario: { figures: {} }, steps: budgetComparison, anchor: (page) => page.getByRole('heading', { name: /^Actuals minus budget, by account line/ }), primary: [], states: [] },
  { view: 'figures-budget-dots-truncated', scenario: { figures: { truncated: true } }, steps: budgetComparison, anchor: (page) => page.getByRole('heading', { name: /^Actuals minus budget, by account line/ }), primary: [], states: [] },
  { view: 'figures-ordinance-time', scenario: { figures: {} }, steps: [nav('analysis'), measure], anchor: (page) => page.getByRole('heading', { name: /^Time of day/ }), primary: [], states: [] },
  { view: 'figures-ordinance-distance', scenario: { figures: {} }, steps: [nav('analysis'), measure], anchor: (page) => page.getByRole('heading', { name: /^Distance to the nearer/ }), primary: [], states: [] },
];

const TOP_LEVEL = new Set(Object.keys(NAV_LABELS).concat('evidence-timeline'));
/** The states captured for a definition under --states. */
const statesFor = (definition) => ['populated', ...definition.states].filter((state) =>
  statesFlag === 'top' ? state === 'populated' || TOP_LEVEL.has(definition.view) : !stateFilter || stateFilter.includes(state));

// ---- Page helpers ----------------------------------------------------------------------------

async function settle(page) {
  let settled = true;
  try {
    await page.waitForFunction((quiet) => {
      const review = window.__uiReview;
      return review && review.inflight === 0 && performance.now() - review.lastActivity > quiet;
    }, QUIET_MS, { timeout: SETTLE_TIMEOUT_MS, polling: 100 });
  } catch {
    settled = false;
  }
  await page.evaluate(async () => {
    await document.fonts.ready;
    // A theme switch or resize starts colour and geometry transitions; capturing before they end
    // records a mixed frame (old geometry beside new labels, half-faded text) that axe then scores.
    const finite = document.getAnimations().filter((animation) => animation.effect?.getTiming().iterations !== Infinity);
    await Promise.race([Promise.allSettled(finite.map((animation) => animation.finished)), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
  return settled;
}

async function drain(page) {
  return page.evaluate(() => {
    const review = window.__uiReview;
    if (!review) return { gaps: [], handlerErrors: [], sqlMisses: [], injected: [], hung: [] };
    const taken = { gaps: [...new Set(review.gaps)], handlerErrors: [...review.handlerErrors], sqlMisses: [...new Set(review.sqlMisses)], injected: [...new Set(review.injected)], hung: [...new Set(review.hung)] };
    review.gaps.length = 0;
    review.handlerErrors.length = 0;
    review.sqlMisses.length = 0;
    review.injected.length = 0;
    return taken;
  });
}

async function shellReady(page, gate, gallery) {
  if (gallery) {
    await page.locator('[data-gallery-section]').first().waitFor({ timeout: 30_000 });
    return;
  }
  if (gate) {
    await page.locator('#root *').first().waitFor({ timeout: 30_000 });
    return;
  }
  // A dialog that opens at launch (the collection plan) makes the shell inert and aria-hidden; it is
  // still rendered, which is all readiness needs.
  await page.getByRole('navigation', { name: 'Primary navigation', includeHidden: true }).waitFor({ timeout: 30_000 });
}

async function applyTheme(page, theme) {
  await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
  await page.evaluate((value) => window.__uiReview?.store?.getState().setTheme(value), theme);
}

/** Keyboard focus order, which axe does not test: Tab from the top, naming each stop heuristically. */
async function focusOrder(page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    window.scrollTo(0, 0);
  });
  const stops = [];
  let first = null;
  for (let index = 0; index < 60; index++) {
    await page.keyboard.press('Tab');
    const stop = await page.evaluate(() => {
      const element = document.activeElement;
      if (!element || element === document.body) return null;
      const text = (value) => (value ?? '').replace(/\s+/g, ' ').trim();
      const labelledBy = element.getAttribute('aria-labelledby');
      const forLabel = element.id ? document.querySelector(`label[for="${CSS.escape(element.id)}"]`) : null;
      const wrapping = element.closest('label');
      const name = text(element.getAttribute('aria-label'))
        || (labelledBy ? text(labelledBy.split(/\s+/).map((id) => document.getElementById(id)?.textContent ?? '').join(' ')) : '')
        || text(forLabel?.textContent)
        || (wrapping && wrapping !== element ? text(wrapping.textContent) : '')
        || text(element instanceof HTMLElement ? element.innerText : element.textContent)
        || text(element.getAttribute('title'))
        || text(element.getAttribute('placeholder'))
        || text(element.getAttribute('alt'));
      const rect = element.getBoundingClientRect();
      const key = `${element.tagName}|${element.id}|${Math.round(rect.x)},${Math.round(rect.y)}|${name}`;
      return { key, role: element.getAttribute('role') ?? element.tagName.toLowerCase(), name: name.slice(0, 100), visible: rect.width > 0 && rect.height > 0 && rect.bottom > 0 && rect.right > 0 && rect.top < innerHeight && rect.left < innerWidth };
    });
    if (!stop) continue;
    if (first === stop.key) break;
    first ??= stop.key;
    stops.push({ role: stop.role, name: stop.name, emptyName: stop.name === '', offscreen: !stop.visible });
  }
  const unnamed = await page.evaluate(() => {
    const text = (value) => (value ?? '').replace(/\s+/g, ' ').trim();
    const named = (element) => text(element.getAttribute('aria-label')) || element.getAttribute('aria-labelledby')
      || text(element instanceof HTMLElement ? element.innerText : element.textContent) || text(element.getAttribute('title'))
      || (element.id && document.querySelector(`label[for="${CSS.escape(element.id)}"]`)) || element.closest('label');
    const candidates = [...document.querySelectorAll('button, a[href], input:not([type=hidden]), select, textarea, [role=button], [role=tab], [role=link]')];
    return candidates.filter((element) => element.getClientRects().length > 0 && !named(element)).slice(0, 30).map((element) => ({
      tag: element.tagName.toLowerCase(), type: element.getAttribute('type'), className: (element.getAttribute('class') ?? '').slice(0, 80),
    }));
  });
  const images = await page.evaluate(() => [...document.querySelectorAll('img:not([alt]), svg[role=img]:not([aria-label]):not([aria-labelledby])')].filter((element) => element.getClientRects().length > 0).length);
  return {
    method: 'keyboard-focus-order (heuristic accessible names)',
    stops,
    stopCount: stops.length,
    emptyNameStops: stops.filter((stop) => stop.emptyName).length,
    offscreenStops: stops.filter((stop) => stop.offscreen).length,
    unnamedInteractive: unnamed,
    imagesWithoutText: images,
  };
}

async function discoverSubviews(page) {
  return page.evaluate(() => {
    const scope = document.querySelector('main') ?? document.body;
    const tabs = [...scope.querySelectorAll('[role=tab]')].filter((tab) => tab.getAttribute('aria-selected') !== 'true' && tab.getClientRects().length > 0)
      .map((tab) => ({ kind: 'tab', name: (tab.textContent ?? '').replace(/\s+/g, ' ').trim() })).filter((tab) => tab.name);
    const drawers = [...scope.querySelectorAll('details.app-panel > summary, details[data-ui-drawer] > summary')].map((summary) => ({ kind: 'drawer', name: (summary.textContent ?? '').replace(/\s+/g, ' ').trim() }));
    return [...tabs, ...drawers];
  });
}

const AXE_TARGETS_KEPT = 20;

/**
 * axe-core over the whole document as it stands; the script is injected once per page load.
 * Serious and critical violations keep every target: check-ui-review.mjs gates on them per target,
 * and a truncated list would turn one new node into a false "new" plus a false "disappeared".
 */
async function axeScan(page) {
  try {
    return await page.evaluate(async (kept) => {
      const results = await window.axe.run(document, { resultTypes: ['violations'] });
      return {
        violations: results.violations.map((violation) => ({
          id: violation.id, impact: violation.impact ?? null, help: violation.help, helpUrl: violation.helpUrl, count: violation.nodes.length,
          targets: (['serious', 'critical'].includes(violation.impact) ? violation.nodes : violation.nodes.slice(0, kept))
            .map((node) => node.target.map((part) => (Array.isArray(part) ? part.join(' >>> ') : part)).join(' ')),
        })),
      };
    }, AXE_TARGETS_KEPT);
  } catch (error) {
    return { error: String(error?.message ?? error).split('\n')[0] };
  }
}

const slug = (value) => value.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40);

// ---- Run --------------------------------------------------------------------------------------

async function loadPlaywright() {
  if (process.env.PLAYWRIGHT_MODULE) return import(pathToFileURL(process.env.PLAYWRIGHT_MODULE).href);
  try {
    return await import('playwright');
  } catch (error) {
    if (error?.code !== 'ERR_MODULE_NOT_FOUND' || !existsSync(HOMEBREW_PLAYWRIGHT)) throw error;
    return import(pathToFileURL(HOMEBREW_PLAYWRIGHT).href);
  }
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

let vite;
let browser;
let cacheDirectory;
const index = {
  /** Steps after which document.scrollingElement.scrollTop was not 0; any entry fails the run. */
  shellScrolled: [],
  label,
  generatedAt: new Date().toISOString(),
  gitHead,
  fixtureNow: new Date(FIXTURE_NOW).toISOString(),
  themes: THEMES,
  sizes: SIZES.map(([width, height]) => ({ width, height })),
  data,
  states: statesFlag ?? 'all',
  only,
  browser: null,
  accessibilityMethod: 'axe-core violations per capture, plus keyboard-focus-order per view',
  axeAvailable: false,
  axeVersion: null,
  views: [],
  captures: [],
  accessibility: {},
  axe: {},
  sqlMisses: [],
  unreachable: [],
};
let harnessFailure = null;

try {
  if (chromeBin && !existsSync(chromeBin)) throw new Error(`Chrome not found at ${chromeBin}; set CHROME_BIN or pass --browser=bundled.`);
  const axeFile = path.join(root, 'node_modules/axe-core/axe.min.js');
  index.axeAvailable = existsSync(axeFile);
  if (!index.axeAvailable) throw new Error(`axe-core not found at ${axeFile}; run npm ci.`);
  const axeSource = await readFile(axeFile, 'utf8');
  index.axeVersion = JSON.parse(await readFile(path.join(root, 'node_modules/axe-core/package.json'), 'utf8')).version;
  const { chromium } = await loadPlaywright();
  const { createServer } = await import('vite');
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(entryFile, entry);
  await writeFile(htmlFile, html);
  await writeFile(galleryEntryFile, galleryEntry);
  await writeFile(galleryHtmlFile, galleryHtml);
  // A private dependency cache: node_modules may be shared with other checkouts' dev servers.
  cacheDirectory = await mkdtemp(path.join(os.tmpdir(), 'onrecord-ui-review-vite-'));
  const port = await freePort();
  vite = await createServer({
    root,
    configFile: path.join(root, 'vite.config.ts'),
    cacheDir: cacheDirectory,
    logLevel: 'warn',
    clearScreen: false,
    server: { host: '127.0.0.1', port, strictPort: true, hmr: false, watch: null },
    optimizeDeps: { entries: [path.basename(htmlFile), path.basename(galleryHtmlFile)] },
  });
  await vite.listen();
  const origin = `http://127.0.0.1:${port}`;
  browser = await chromium.launch({ ...(chromeBin ? { executablePath: chromeBin } : {}), headless: true });
  index.browser = `${chromeBin ?? 'bundled chromium'} ${browser.version()}`;

  const urlFor = (scenario, gallery) => `${origin}/${stem}${gallery ? '-gallery' : ''}.html?scenario=${encodeURIComponent(JSON.stringify(scenario))}`;

  /** One page load: run the steps, then capture both themes at every size. */
  async function captureState(context, definition, state) {
    const baseScenario = { mode: 'full', ...(real ? { data } : {}), ...(definition.scenario ?? {}) };
    const scenario = state === 'empty' ? { ...baseScenario, mode: 'empty' }
      : state === 'loading' ? { ...baseScenario, hang: definition.primary }
        : state === 'error' ? { ...baseScenario, fail: definition.primary }
          : baseScenario;
    const page = await context.newPage();
    const consoleErrors = [];
    const failedRequests = [];
    const pageErrors = [];
    page.on('console', (message) => {
      if (message.type() === 'error' || message.type() === 'warning') consoleErrors.push({ type: message.type(), text: message.text().slice(0, 600) });
    });
    page.on('pageerror', (error) => pageErrors.push(String(error.stack ?? error).slice(0, 800)));
    page.on('requestfailed', (request) => failedRequests.push({ url: request.url(), failure: request.failure()?.errorText ?? null }));
    page.on('response', (response) => {
      if (response.status() >= 400) failedRequests.push({ url: response.url(), status: response.status() });
    });
    const take = () => {
      const taken = { consoleErrors: [...consoleErrors], pageErrors: [...pageErrors], failedRequests: [...failedRequests] };
      consoleErrors.length = 0;
      pageErrors.length = 0;
      failedRequests.length = 0;
      return taken;
    };
    const steps = [`open ${state === 'populated' ? 'app' : `app with ${state} stub`} (scenario ${JSON.stringify(scenario)})`];
    try {
      await page.setViewportSize({ width: 1280, height: 800 });
      await page.clock.setFixedTime(new Date(FIXTURE_NOW));
      await page.emulateMedia({ colorScheme: 'light', reducedMotion: 'reduce' });
      await page.goto(urlFor({ ...scenario, theme: 'light' }, definition.gallery), { timeout: 120_000 });
      await shellReady(page, definition.gate, definition.gallery);
      await settle(page);
      for (const step of definition.steps) {
        await step(page, steps);
        await settle(page);
        // The shell never scrolls: a step that moved the document carried the title bar off screen.
        const shellTop = await page.evaluate(() => document.scrollingElement?.scrollTop ?? 0);
        if (shellTop !== 0) index.shellScrolled.push({ view: definition.view, state, after: steps.at(-1), scrollTop: shellTop });
      }
    } catch (error) {
      const reason = String(error?.message ?? error).split('\n')[0];
      index.unreachable.push({ view: definition.view, state, steps, reason, ...take(), ...(await drain(page).catch(() => ({}))) });
      if (!definition.gate && state === 'populated' && definition.view === 'overview') {
        throw new Error(`The app shell never rendered: ${reason}`, { cause: error });
      }
      await page.close();
      return null;
    }
    const preamble = { ...take(), ...(await drain(page)) };
    const axeInjected = await page.addScriptTag({ content: axeSource }).then(() => true, () => false);
    const records = [];
    for (const theme of THEMES) {
      await applyTheme(page, theme);
      for (const [width, height] of SIZES.filter(([size]) => !definition.widths || definition.widths.includes(size))) {
        await page.setViewportSize({ width, height });
        const settled = await settle(page);
        if (definition.anchor) await definition.anchor(page).first().evaluate((element) => element.scrollIntoView({ block: 'start' })).catch(() => {});
        const file = `${definition.view}__${state}__${theme}__${width}.png`;
        await page.screenshot({ path: path.join(outputDirectory, file) });
        for (const selector of definition.sections ?? []) {
          const element = page.locator(selector).first();
          if (!(await element.count())) continue;
          // The app scrolls inside its own panes, which paint only what the viewport shows: grow the
          // viewport to the element's height for its capture, at the same width.
          const box = await element.boundingBox();
          if (box && box.height > height) {
            await page.setViewportSize({ width, height: Math.min(6000, Math.ceil(box.height) + 160) });
            await settle(page);
          }
          await element.screenshot({ path: path.join(outputDirectory, `${definition.view}__${state}__${theme}__${width}__${slug(selector)}.png`) });
          await page.setViewportSize({ width, height });
        }
        if (definition.gallery) {
          // One capture per gallery section at full resolution: a whole-page capture is downscaled
          // when viewed and hides tick-level defects.
          const sections = page.locator('[data-gallery-section]');
          for (let index = 0; index < await sections.count(); index++) {
            const section = sections.nth(index);
            const name = slug((await section.getAttribute('data-gallery-section')) ?? String(index));
            await section.screenshot({ path: path.join(outputDirectory, `${definition.view}__${state}__${theme}__${width}__${String(index + 1).padStart(2, '0')}-${name}.png`) });
          }
        }
        const axe = axeInjected ? await axeScan(page) : { error: 'axe-core could not be injected into the page' };
        const observed = { ...take(), ...(await drain(page)) };
        records.push({
          file, view: definition.view, state, theme, width, height, steps,
          scenario, settled, axe,
          // Everything seen while loading and running the steps belongs to the first capture.
          ...(records.length === 0 ? merge(preamble, observed) : observed),
        });
      }
    }
    index.captures.push(...records);
    return page;
  }

  const discovered = new Set();
  const merge = (a, b) => Object.fromEntries(Object.keys({ ...a, ...b }).map((key) => [key, [...(a[key] ?? []), ...(b[key] ?? [])]]));

  async function runDefinition(context, definition) {
    const record = { view: definition.view, steps: null, states: [] };
    index.views.push(record);
    for (const state of statesFor(definition)) {
      const page = await captureState(context, definition, state);
      if (!page) continue;
      record.states.push(state);
      if (state === 'populated') {
        record.steps = index.captures.find((capture) => capture.view === definition.view)?.steps ?? null;
        await applyTheme(page, 'light');
        await page.setViewportSize({ width: 1280, height: 800 });
        await settle(page);
        index.accessibility[definition.view] = await focusOrder(page);
        if (definition.discover) {
          for (const subview of await discoverSubviews(page)) {
            // Overview's drawers render under its sub-views too; capture each drawer once.
            const key = `${subview.kind}:${subview.name}`;
            if (subview.kind === 'drawer' && discovered.has(key)) continue;
            discovered.add(key);
            const name = `${definition.view}-${subview.kind}-${slug(subview.name)}`;
            if (only && !only.some((filter) => name.startsWith(filter))) continue;
            const locator = subview.kind === 'tab'
              ? (p) => p.getByRole('main').getByRole('tab', { name: subview.name, exact: true })
              : (p) => p.getByRole('main').locator('details.app-panel > summary, details[data-ui-drawer] > summary', { hasText: subview.name });
            queue.push({ view: name, scenario: definition.scenario, steps: [...definition.steps, click(`${subview.kind} "${subview.name}"`, locator)], primary: [], states: [], discovered: true });
          }
        }
      }
      await page.close();
    }
  }

  const queue = VIEWS.filter((definition) => !only || only.some((filter) => definition.view === filter || definition.view.startsWith(`${filter}-`) || definition.view.startsWith(filter)));
  const workers = Array.from({ length: concurrency }, async () => {
    const context = await browser.newContext({ deviceScaleFactor: 1, colorScheme: 'light', reducedMotion: 'reduce' });
    await context.addInitScript(initScript, FIXTURE_NOW);
    try {
      while (queue.length > 0) {
        const definition = queue.shift();
        const started = Date.now();
        await runDefinition(context, definition);
        console.log(`${definition.view}: ${((Date.now() - started) / 1000).toFixed(1)}s`);
      }
    } finally {
      await context.close();
    }
  });
  // Workers pull from a queue that discovery extends, so wait until every worker has drained it.
  await Promise.all(workers);
  if (index.captures.length === 0) throw new Error('No captures were taken.');
} catch (error) {
  harnessFailure = error;
} finally {
  await browser?.close().catch(() => {});
  await vite?.close().catch(() => {});
  await Promise.all([entryFile, htmlFile, galleryEntryFile, galleryHtmlFile].map((file) => rm(file, { force: true })));
  if (cacheDirectory) await rm(cacheDirectory, { recursive: true, force: true });
}

index.finishedAt = new Date().toISOString();
if (index.shellScrolled.length > 0 && !harnessFailure) {
  harnessFailure = new Error(`The app shell scrolled: ${index.shellScrolled.map((entry) => `${entry.view} (${entry.state}) after "${entry.after}": scrollTop ${entry.scrollTop}`).join('; ')}`);
}
// Per view, one entry per rule and target node across all its states, themes and widths, so the six
// captures of a state do not count one defect six times. `nodes` is the number of distinct targets
// (bounded by the AXE_TARGETS_KEPT kept per capture); `instances` sums axe's own node counts.
const IMPACTS = ['critical', 'serious', 'moderate', 'minor'];
for (const capture of index.captures) {
  const view = (index.axe[capture.view] ??= { scannedCaptures: 0, failedScans: 0, rules: {} });
  if (capture.axe?.error) { view.failedScans += 1; continue; }
  view.scannedCaptures += 1;
  for (const violation of capture.axe?.violations ?? []) {
    const rule = (view.rules[violation.id] ??= { id: violation.id, impact: violation.impact, help: violation.help, helpUrl: violation.helpUrl, captures: 0, instances: 0, states: [], targets: [] });
    rule.captures += 1;
    rule.instances += violation.count;
    if (!rule.states.includes(capture.state)) rule.states.push(capture.state);
    for (const target of violation.targets) if (!rule.targets.includes(target)) rule.targets.push(target);
  }
}
for (const view of Object.values(index.axe)) {
  view.rules = Object.values(view.rules).map((rule) => ({ ...rule, nodes: rule.targets.length })).sort((a, b) => IMPACTS.indexOf(a.impact) - IMPACTS.indexOf(b.impact) || b.nodes - a.nodes);
  view.nodesByImpact = Object.fromEntries(IMPACTS.map((impact) => [impact, view.rules.filter((rule) => rule.impact === impact).reduce((sum, rule) => sum + rule.nodes, 0)]));
}
const topRules = {};
for (const [viewName, view] of Object.entries(index.axe)) {
  for (const rule of view.rules) {
    const entry = (topRules[rule.id] ??= { id: rule.id, impact: rule.impact, help: rule.help, nodes: 0, views: [] });
    entry.nodes += rule.nodes;
    entry.views.push(viewName);
  }
}
index.axeTop = Object.values(topRules).sort((a, b) => b.nodes - a.nodes);
const misses = new Map();
for (const capture of index.captures) for (const sql of capture.sqlMisses ?? []) misses.set(sql, [...new Set([...(misses.get(sql) ?? []), capture.view])]);
index.sqlMisses = [...misses].map(([sql, views]) => ({ sql, views }));
index.summary = {
  views: index.views.length,
  captures: index.captures.length,
  unreachable: index.unreachable.length,
  capturesWithConsoleErrors: index.captures.filter((capture) => capture.consoleErrors?.some((entry) => entry.type === 'error') || capture.pageErrors?.length).length,
  stubGaps: [...new Set(index.captures.flatMap((capture) => capture.gaps ?? []))].sort(),
  unsettledCaptures: index.captures.filter((capture) => !capture.settled).length,
  sqlMisses: index.sqlMisses.length,
  axeScannedCaptures: index.captures.filter((capture) => capture.axe && !capture.axe.error).length,
  axeFailedScans: index.captures.filter((capture) => capture.axe?.error).length,
};
if (index.captures.length > 0 || harnessFailure) {
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(path.join(outputDirectory, 'index.json'), `${JSON.stringify({ ...index, harnessFailure: harnessFailure ? String(harnessFailure.stack ?? harnessFailure) : null }, null, 2)}\n`);
}
if (Object.keys(index.axe).length) {
  const rows = Object.entries(index.axe).map(([view, summary]) => [view, ...IMPACTS.slice(0, 3).map((impact) => String(summary.nodesByImpact[impact]))]);
  const width = Math.max(4, ...rows.map(([view]) => view.length));
  console.log(`\naxe-core ${index.axeVersion}: distinct violating nodes per view (rule x target, across states, themes and widths)`);
  console.log(`${'view'.padEnd(width)}  critical  serious  moderate`);
  for (const [view, critical, serious, moderate] of rows) console.log(`${view.padEnd(width)}  ${critical.padStart(8)}  ${serious.padStart(7)}  ${moderate.padStart(8)}`);
  console.log(`\ntop rules: ${index.axeTop.slice(0, 8).map((rule) => `${rule.id} (${rule.impact}, ${rule.nodes} nodes, ${rule.views.length} views)`).join('; ')}`);
}
console.log(`${index.captures.length} captures of ${index.views.length} views (${data} data), ${index.unreachable.length} unreachable, ${index.summary.unsettledCaptures} unsettled, ${index.sqlMisses.length} SQL misses, ${index.summary.axeFailedScans} failed axe scans -> ${outputDirectory}`);
if (harnessFailure) {
  console.error(harnessFailure);
  process.exit(1);
}
