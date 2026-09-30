// Captures the Help screenshots from the real React components, driven by a stubbed IPC bridge
// that returns invented sample data and rejects every command it does not know. Browser/component
// capture only: no native application, network, or model is involved.
//
// PLAYWRIGHT_MODULE=/opt/homebrew/lib/node_modules/@playwright/mcp/node_modules/playwright/index.mjs \
// CHROME_BIN="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
// node scripts/capture-help-screenshots.mjs
import { spawn } from 'node:child_process';
import { mkdir, rm, stat, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
if (!process.env.PLAYWRIGHT_MODULE || !process.env.CHROME_BIN) {
  throw new Error('Set PLAYWRIGHT_MODULE (an installed playwright index.mjs) and CHROME_BIN (a Chrome executable).');
}
const { chromium } = await import(pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href);
const outputDirectory = path.join(root, 'src/content/help/screenshots');
const stem = `__help_capture_${process.pid}_${Date.now()}`;
const temporaryFiles = [`${stem}.tsx`, `${stem}.html`].map((name) => path.join(root, name));
const port = Number(process.env.HELP_CAPTURE_PORT ?? 5243);
const origin = `http://127.0.0.1:${port}`;

const now = Date.now();
const iso = (offsetHours) => new Date(now - offsetHours * 3600_000).toISOString();

const entry = `import { createRoot } from 'react-dom/client';
import FindingsInbox from './src/components/dashboard/FindingsInbox';
import BriefingWorkspace from './src/components/dashboard/BriefingWorkspace';
import MyRequestsPanel from './src/components/tracker/MyRequestsPanel';
import { useAppStore } from './src/store/useAppStore';
import './src/index.css';

const iso = (h) => new Date(${now} - h * 3600000).toISOString();
const subject = { portalKey: 'sample-agency', agencyId: null, requestId: null, documentId: 12, entityId: null };
const finding = (id, kind, severity, confidence, headline, explanation, hours) => ({
  id, kind, severity, confidence, subject, headline, explanation,
  citations: [{ portalKey: 'sample-agency', documentId: 12, characterStart: 210, characterEnd: 268, quote: 'Sample quoted passage from a sample response letter.', label: 'Sample response letter' }],
  discoveredAt: iso(hours), lastSeenAt: iso(hours), dismissedAt: null, sourceDetector: 'sample',
});
const findings = [
  finding('f1', 'POLICY_VIOLATION_DETECTED', 'SERIOUS', { kind: 'MEASURED', basisPoints: 9000 }, 'Sample Agency exceeded its own stated 10-day response time on 6 requests', 'Its policy document states 10 calendar days. Six requests to the same agency took longer, each measured from a sourced determination.', 1),
  finding('f2', 'BOILERPLATE_DENIAL', 'NOTABLE', { kind: 'MEASURED', basisPoints: 7800 }, 'The same denial paragraph appears in 9 unrelated responses', 'Near-identical wording was reused across requests on different topics.', 2),
  finding('f3', 'ABSENCE_OF_RECORDS', 'NOTABLE', { kind: 'UNASSESSED' }, 'Memos 14 through 19 are missing from a numbered series', 'The released set numbers memos 1 to 24 and omits a tight run in the middle.', 3),
];
const claim = {
  id: 'c1', portalKey: null, reference: 'SAMPLE-2026-0142', title: 'Sample records request', sourceUrl: null,
  jurisdiction: 'CA_CPRA', claimedAt: iso(200), importedEmails: [],
  events: [{ id: 'e1', kind: 'SUBMITTED', occurredAt: iso(120), basis: 'USER_ENTERED', evidence: null, extensionReason: null, expectedDeterminationAt: null, createdAt: iso(119) }],
  suggestions: [{ id: 's1', kind: 'ACKNOWLEDGED', candidateOccurredAt: iso(110), suggestedAt: iso(100),
    evidence: { messageIdHash: 'h', subjectExcerpt: 'Your request has been received', emailDate: iso(110), excerpt: 'We have received your request and will respond shortly.', sentenceStart: 0, sentenceEnd: 54, matchedSentence: 'We have received your request and will respond shortly.' } }],
};
const deadline = {
  claimId: 'c1', model: 'CALIFORNIA_CPRA', analysis: null, countdown: { kind: 'DAYS_REMAINING', days: 5 },
  determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', violationAsserted: false,
  basisStatement: 'No determination is recorded. The period is still running and is counted from the submission date you entered.',
  notes: [], pendingSuggestionCount: 1,
};
const answers = {
  list_findings: findings,
  list_evidence_gaps: [],
  get_affiliation_disclosure: { status: 'UNSET', sentence: null, affiliations: [], ownPortalKeys: [], ownAgencies: [], ownEntityIds: [] },
  get_profile: { status: 'UNSET', profile: null },
  list_claimed_requests: [claim],
  get_request_deadline: deadline,
  list_saved_portals: [],
};
window.isTauri = true;
window.__TAURI_INTERNALS__ = { invoke: async (command) => {
  if (command in answers) return answers[command];
  throw new Error('Capture harness has no data for ' + command);
} };
document.documentElement.dataset.theme = 'dark';
document.documentElement.dataset.motion = 'reduced';
useAppStore.setState({ workspaceInitializationStatus: 'ready', workspaceInitializationError: null });
const shot = new URLSearchParams(location.search).get('shot');
const root = createRoot(document.getElementById('root'));
if (shot === 'findings') root.render(<div id="capture" className="app-theme p-4" style={{ width: 1000 }}><FindingsInbox /></div>);
if (shot === 'briefing') root.render(<div id="capture" className="app-theme flex" style={{ width: 1200, height: 900 }}><BriefingWorkspace /></div>);
if (shot === 'tracker') root.render(<div id="capture" className="app-theme"><MyRequestsPanel onClose={() => {}} /></div>);
`;
const html = `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0"><title>Help capture</title></head><body><div id="root"></div><script type="module" src="/${stem}.tsx"></script></body></html>`;

let server;
let browser;
let serverOutput = '';
try {
  await mkdir(outputDirectory, { recursive: true });
  await writeFile(temporaryFiles[0], entry);
  await writeFile(temporaryFiles[1], html);
  server = spawn(process.execPath, [path.join(root, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], { cwd: root, env: { ...process.env, NO_COLOR: '1' }, stdio: ['ignore', 'pipe', 'pipe'] });
  server.stdout.on('data', (chunk) => { serverOutput += chunk.toString(); });
  server.stderr.on('data', (chunk) => { serverOutput += chunk.toString(); });
  let ready = false;
  for (let attempt = 0; attempt < 500; attempt++) {
    if (server.exitCode !== null) throw new Error(`Vite exited: ${serverOutput}`);
    if (serverOutput.includes(origin)) { ready = true; break; }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  if (!ready) throw new Error(`Vite not ready: ${serverOutput}`);
  browser = await chromium.launch({ executablePath: process.env.CHROME_BIN, headless: true });
  const errors = [];
  const shots = [
    { name: 'findings', wait: (page) => page.getByText('Findings inbox').first().waitFor(), viewport: { width: 1040, height: 900 }, clip: '#capture' },
    { name: 'tracker', wait: async (page) => { await page.getByRole('button', { name: /SAMPLE-2026-0142/ }).click(); await page.getByText('Suggested from emails').waitFor(); }, viewport: { width: 600, height: 800 }, clip: null },
    { name: 'briefing', wait: (page) => page.getByText('Turn evidence into a defensible public comment.').waitFor(), viewport: { width: 1200, height: 900 }, clip: '#capture' },
  ];
  for (const shot of shots) {
    const page = await browser.newPage({ viewport: shot.viewport, deviceScaleFactor: 1 });
    page.on('pageerror', (error) => errors.push(`${shot.name}: ${error.message}`));
    page.setDefaultTimeout(120_000);
    await page.goto(`${origin}/${stem}.html?shot=${shot.name}`, { timeout: 120_000 });
    await shot.wait(page);
    await page.waitForTimeout(400);
    const file = path.join(outputDirectory, `${shot.name}.jpg`);
    if (shot.clip) await page.locator(shot.clip).screenshot({ path: file, type: 'jpeg', quality: 70 });
    else await page.screenshot({ path: file, type: 'jpeg', quality: 70 });
    console.log(`${shot.name}: ${(await stat(file)).size} bytes`);
    await page.close();
  }
  if (errors.length) throw new Error(errors.join('\n'));
} finally {
  await browser?.close();
  if (server && server.exitCode === null) {
    const exited = new Promise((resolve) => server.once('exit', resolve));
    server.kill('SIGTERM');
    const timeout = setTimeout(() => server.kill('SIGKILL'), 3000);
    await exited;
    clearTimeout(timeout);
  }
  await Promise.all(temporaryFiles.map((file) => rm(file, { force: true })));
}
