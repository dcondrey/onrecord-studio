// Captures every workspace view and major panel of the real <App/> for UI review, driven by a
// stubbed IPC bridge serving invented San Diego data (scripts/ui-review-fixtures.mjs). Browser only:
// no native application, network, or model is involved.
//
//   node scripts/capture-ui-review.mjs [outputDirectory] [--only=view,view] [--concurrency=3]
//
// Output: <view>__<state>__<theme>__<width>.png plus index.json (steps, console errors, failed
// requests, stub gaps, and a keyboard-focus-order accessibility record per view). Exits non-zero
// only when the harness itself fails (server, browser, app shell unreachable), never on findings.
//
// PLAYWRIGHT_MODULE and CHROME_BIN override the Playwright build and Chrome executable.
/* global document, window, HTMLElement, CSS, innerHeight, innerWidth, location, requestAnimationFrame -- page.evaluate bodies run in the browser */
import { execFileSync } from 'node:child_process';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
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
const playwrightModule = process.env.PLAYWRIGHT_MODULE ?? '/opt/homebrew/lib/node_modules/@playwright/mcp/node_modules/playwright/index.mjs';
const chromeBin = process.env.CHROME_BIN ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';

const THEMES = ['light', 'dark'];
const SIZES = [[1024, 700], [1280, 800], [1680, 1050]];
const SETTLE_TIMEOUT_MS = 15_000;
const QUIET_MS = 350;

const stem = `__ui_review_${process.pid}_${Date.now()}`;
const entryFile = path.join(root, `${stem}.tsx`);
const htmlFile = path.join(root, `${stem}.html`);

// The entry installs the IPC stub before the app's modules evaluate, then boots src/main.tsx
// unchanged so the capture is of the production component tree.
const entry = `import { createHandlers, injectedError } from './scripts/ui-review-fixtures.mjs';
const scenario = JSON.parse(new URLSearchParams(location.search).get('scenario') || '{}');
const handlers = createHandlers(scenario);
const review = { calls: [], gaps: [], handlerErrors: [], injected: [], hung: [], plugins: [], inflight: 0, lastActivity: performance.now() };
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
        review.handlerErrors.push(command + ': ' + String(error));
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
const html = `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0"><title>OnRecord Studio</title><link rel="icon" href="data:,"></head><body><div id="root"></div><script type="module" src="/${stem}.tsx"></script></body></html>`;

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
const event = (name, detail) => store(`dispatch window event '${name}'`, ([eventName, eventDetail]) => window.dispatchEvent(new CustomEvent(eventName, { detail: eventDetail })), [name, detail]);
const inspect = (target) => store(`store.inspect(${JSON.stringify(target)})`, (t) => window.__uiReview.store.getState().inspect(t), target);

const selectRequest = click('evidence result "Encampment abatement notices…"', (page) => page.getByRole('main').getByText(/Encampment abatement notices posted along Imperial/));
const openDocument = click('document "ESD_Response_Letter_26-0412.pdf"', (page) => page.getByRole('button', { name: /ESD_Response_Letter_26-0412\.pdf/ }));

/**
 * `states` lists the variants beyond populated. `primary` are the commands a loading state hangs
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
  { view: 'overview-response-analytics', steps: [nav('overview'), click('"Show response analytics"', (page) => page.getByRole('button', { name: 'Show response analytics' }))], primary: ['get_agency_performance_profiles', 'get_cross_agency_correlations'], states: ['error'] },
  { view: 'overview-casework-tools', steps: [nav('overview'), click('"Open casework and research tools"', (page) => page.getByRole('button', { name: 'Open casework and research tools' }))], primary: ['get_casework_state', 'get_research_status'], states: ['empty', 'error'], discover: true },
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
];

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
    await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  });
  return settled;
}

async function drain(page) {
  return page.evaluate(() => {
    const review = window.__uiReview;
    if (!review) return { gaps: [], handlerErrors: [], injected: [], hung: [] };
    const taken = { gaps: [...new Set(review.gaps)], handlerErrors: [...review.handlerErrors], injected: [...new Set(review.injected)], hung: [...new Set(review.hung)] };
    review.gaps.length = 0;
    review.handlerErrors.length = 0;
    review.injected.length = 0;
    return taken;
  });
}

async function shellReady(page, gate) {
  if (gate) {
    await page.locator('#root *').first().waitFor({ timeout: 30_000 });
    return;
  }
  await page.getByRole('navigation', { name: 'Primary navigation' }).waitFor({ timeout: 30_000 });
}

async function applyTheme(page, theme) {
  await page.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
  await page.evaluate((value) => window.__uiReview?.store?.getState().setTheme(value), theme);
}

/** Keyboard focus order (axe-core is not installed): Tab from the top, naming each stop heuristically. */
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
    method: 'keyboard-focus-order (heuristic accessible names; axe-core not available)',
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
    const drawers = [...scope.querySelectorAll('details.app-panel > summary')].map((summary) => ({ kind: 'drawer', name: (summary.textContent ?? '').replace(/\s+/g, ' ').trim() }));
    return [...tabs, ...drawers];
  });
}

const slug = (value) => value.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 40);

// ---- Run --------------------------------------------------------------------------------------

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
  label,
  generatedAt: new Date().toISOString(),
  gitHead,
  fixtureNow: new Date(FIXTURE_NOW).toISOString(),
  themes: THEMES,
  sizes: SIZES.map(([width, height]) => ({ width, height })),
  accessibilityMethod: 'keyboard-focus-order',
  axeAvailable: false,
  views: [],
  captures: [],
  accessibility: {},
  unreachable: [],
};
let harnessFailure = null;

try {
  if (!existsSync(playwrightModule)) throw new Error(`Playwright not found at ${playwrightModule}; set PLAYWRIGHT_MODULE.`);
  if (!existsSync(chromeBin)) throw new Error(`Chrome not found at ${chromeBin}; set CHROME_BIN.`);
  index.axeAvailable = existsSync(path.join(root, 'node_modules/axe-core/axe.min.js'));
  const { chromium } = await import(pathToFileURL(playwrightModule).href);
  const { createServer } = await import('vite');
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(entryFile, entry);
  await writeFile(htmlFile, html);
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
    optimizeDeps: { entries: [path.basename(htmlFile)] },
  });
  await vite.listen();
  const origin = `http://127.0.0.1:${port}`;
  browser = await chromium.launch({ executablePath: chromeBin, headless: true });

  const urlFor = (scenario) => `${origin}/${stem}.html?scenario=${encodeURIComponent(JSON.stringify(scenario))}`;

  /** One page load: run the steps, then capture both themes at every size. */
  async function captureState(context, definition, state) {
    const baseScenario = { mode: 'full', ...(definition.scenario ?? {}) };
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
      await page.goto(urlFor({ ...scenario, theme: 'light' }), { timeout: 120_000 });
      await shellReady(page, definition.gate);
      await settle(page);
      for (const step of definition.steps) {
        await step(page, steps);
        await settle(page);
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
    const records = [];
    for (const theme of THEMES) {
      await applyTheme(page, theme);
      for (const [width, height] of SIZES) {
        await page.setViewportSize({ width, height });
        const settled = await settle(page);
        const file = `${definition.view}__${state}__${theme}__${width}.png`;
        await page.screenshot({ path: path.join(outputDirectory, file) });
        const observed = { ...take(), ...(await drain(page)) };
        records.push({
          file, view: definition.view, state, theme, width, height, steps,
          scenario, settled,
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
    for (const state of ['populated', ...definition.states]) {
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
              : (p) => p.getByRole('main').locator('details.app-panel > summary', { hasText: subview.name });
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
  await Promise.all([entryFile, htmlFile].map((file) => rm(file, { force: true })));
  if (cacheDirectory) await rm(cacheDirectory, { recursive: true, force: true });
}

index.finishedAt = new Date().toISOString();
index.summary = {
  views: index.views.length,
  captures: index.captures.length,
  unreachable: index.unreachable.length,
  capturesWithConsoleErrors: index.captures.filter((capture) => capture.consoleErrors?.some((entry) => entry.type === 'error') || capture.pageErrors?.length).length,
  stubGaps: [...new Set(index.captures.flatMap((capture) => capture.gaps ?? []))].sort(),
};
if (index.captures.length > 0 || harnessFailure) {
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(path.join(outputDirectory, 'index.json'), `${JSON.stringify({ ...index, harnessFailure: harnessFailure ? String(harnessFailure.stack ?? harnessFailure) : null }, null, 2)}\n`);
}
console.log(`${index.captures.length} captures of ${index.views.length} views, ${index.unreachable.length} unreachable -> ${outputDirectory}`);
if (harnessFailure) {
  console.error(harnessFailure);
  process.exit(1);
}
