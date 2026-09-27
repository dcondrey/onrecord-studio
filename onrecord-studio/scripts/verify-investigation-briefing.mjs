// Browser/component validation only: no native application, model, collection or write commands.
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { readFile, writeFile, rm, mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const [reportArgument, outputArgument] = process.argv.slice(2);
if (!reportArgument || !outputArgument || !process.env.PLAYWRIGHT_MODULE || !process.env.CHROME_BIN) {
  throw new Error("Usage: PLAYWRIGHT_MODULE=<installed module path> CHROME_BIN=<browser executable> node scripts/verify-investigation-briefing.mjs <accepted report.json> <output prefix>");
}
const reportPath = path.resolve(root, reportArgument);
const outputPrefix = path.resolve(root, outputArgument);
const raw = await readFile(reportPath, "utf8");
const report = JSON.parse(raw);
if (!report.id || !report.synthesis?.findings?.length || !report.evidence?.length) {
  throw new Error("Choose an accepted saved dossier containing actual findings and evidence.");
}
const { chromium } = await import(pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href);
const stem = `__briefing_review_${process.pid}_${Date.now()}`;
const temporaryFiles = [`.${stem}.json`, `${stem}.tsx`, `${stem}.html`].map((name) => path.join(root, name));
const port = Number(process.env.BRIEFING_REVIEW_PORT ?? 5197);
const origin = `http://127.0.0.1:${port}`;
let server;
let browser;
let serverOutput = "";
try {
  await mkdir(path.dirname(outputPrefix), { recursive: true });
  await writeFile(temporaryFiles[0], raw);
  await writeFile(temporaryFiles[1], `import { createRoot } from 'react-dom/client';
import OverviewDashboard from './src/components/dashboard/OverviewDashboard';
import { useAppStore } from './src/store/useAppStore';
import './src/index.css';
import report from './.${stem}.json';
window.isTauri = true;
window.__TAURI_INTERNALS__ = { invoke: async (command) => {
  if (command !== 'get_research_status') throw new Error('Read-only browser harness rejects: ' + command);
  return { policy: { enabled: true, autoCollect: true, provider: 'openrouter', model: 'openai/gpt-5.4', intervalHours: 6, dailyCallLimit: 24, researchPasses: 4, discoverSources: true },
    credentialAvailable: true, phase: 'Browser review of a saved dossier — live research status is not connected', lastError: null,
    lastFinishedAt: null, callsToday: 0, reports: [report], sources: [], actionStates: {} };
} };
document.documentElement.dataset.theme = 'dark';
document.documentElement.dataset.motion = 'reduced';
useAppStore.setState({ workspaceInitializationStatus: 'ready', workspaceInitializationError: null });
createRoot(document.getElementById('root')).render(<OverviewDashboard />);
`);
  await writeFile(temporaryFiles[2], `<!doctype html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1.0"><title>Saved dossier browser review</title></head><body><div id="root"></div><script type="module" src="/${stem}.tsx"></script></body></html>`);
  server = spawn(process.execPath, [path.join(root, "node_modules/vite/bin/vite.js"), "--host", "127.0.0.1", "--port", String(port), "--strictPort"], { cwd: root, env: { ...process.env, NO_COLOR: "1" }, stdio: ["ignore", "pipe", "pipe"] });
  server.stdout.on("data", (chunk) => { serverOutput += chunk.toString(); });
  server.stderr.on("data", (chunk) => { serverOutput += chunk.toString(); });
  let ready = false;
  for (let attempt = 0; attempt < 75; attempt++) {
    if (server.exitCode !== null) throw new Error(`Review server exited: ${serverOutput}`);
    if (serverOutput.includes(origin)) { ready = true; break; }
    await new Promise((resolve) => setTimeout(resolve, 200));
  }
  if (!ready) throw new Error(`Review server did not become ready: ${serverOutput}`);
  browser = await chromium.launch({ executablePath: process.env.CHROME_BIN, headless: true });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 }, deviceScaleFactor: 1 });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  const url = `${origin}/${stem}.html`;
  await page.goto(url);
  await page.getByRole("heading", { name: "Evidence worth following" }).waitFor();
  const latestSavedLabel = await page.getByText(/^Latest saved dossier:/).innerText();
  const lead = page.locator("article[aria-label]").first();
  const findingTitle = await lead.getAttribute("aria-label");
  const savedFinding = report.synthesis.findings.find((finding) => finding.title === findingTitle);
  if (!savedFinding) throw new Error("Displayed finding does not belong to the supplied dossier");
  const displayedQuote = await lead.locator("blockquote:visible > p").first().textContent();
  if (!savedFinding.citations.some((citation) => displayedQuote === `“${citation.quote}”`)) throw new Error("Displayed quote diverged from the saved citation");
  const sourceHref = await lead.getByRole("link", { name: "Open original source", exact: true }).first().getAttribute("href");
  if (!report.evidence.some((evidence) => evidence.sourceUrl === sourceHref)) throw new Error("Original-source link is not in the saved evidence");
  const action = page.getByRole("button", { name: "Review targeted request" });
  const desktop = { finding: await lead.boundingBox(), action: await action.boundingBox(), horizontalOverflow: await page.evaluate(() => globalThis.document.documentElement.scrollWidth > globalThis.innerWidth) };
  if (desktop.horizontalOverflow) throw new Error("Desktop has horizontal overflow");
  await page.screenshot({ path: `${outputPrefix}-desktop.png`, fullPage: false });
  await action.click();
  const firstRequest = page.getByRole("article").first();
  const requestTitle = await firstRequest.getByRole("heading").innerText();
  const savedRequest = report.synthesis.requests.find((request) => request.title === requestTitle);
  if (!savedRequest) throw new Error("Opened request does not belong to the supplied dossier");
  const draft = firstRequest.locator("pre").first();
  if (!(await draft.isVisible()) || (await draft.innerText()) !== savedRequest.draft) throw new Error("Prepared request is not open with its original draft");
  const requestPosition = await firstRequest.boundingBox();
  await page.screenshot({ path: `${outputPrefix}-request.png`, fullPage: false });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(url);
  await action.waitFor();
  const mobile = { finding: await lead.boundingBox(), action: await action.boundingBox(), horizontalOverflow: await page.evaluate(() => globalThis.document.documentElement.scrollWidth > globalThis.innerWidth) };
  if (mobile.horizontalOverflow) throw new Error("Mobile has horizontal overflow");
  await page.screenshot({ path: `${outputPrefix}-mobile.png`, fullPage: false });
  await action.click();
  const mobileRequestTitle = await page.getByRole("article").first().getByRole("heading").innerText();
  if (mobileRequestTitle !== requestTitle) throw new Error("Mobile request context diverged");
  if (errors.length) throw new Error(errors.join("\n"));
  const result = {
    validation: "Actual dashboard components with a supplied saved dossier and a read-only mocked IPC status bridge; browser/component review, not native app end-to-end validation. No live research state or credentials are connected.",
    dossier: { path: path.relative(root, reportPath), sha256: createHash("sha256").update(raw).digest("hex"), id: report.id, createdAt: report.createdAt },
    latestSavedLabel, findingTitle, displayedQuote, sourceHref, requestTitle, requestPosition, mobileRequestTitle, desktop, mobile, errors,
  };
  await writeFile(`${outputPrefix}.json`, `${JSON.stringify(result, null, 2)}\n`);
  console.log(JSON.stringify(result, null, 2));
} finally {
  await browser?.close();
  if (server && server.exitCode === null) {
    const exited = new Promise((resolve) => server.once("exit", resolve));
    server.kill("SIGTERM");
    const timeout = setTimeout(() => server.kill("SIGKILL"), 3000);
    await exited;
    clearTimeout(timeout);
  }
  await Promise.all(temporaryFiles.map((file) => rm(file, { force: true })));
}
