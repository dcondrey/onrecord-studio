// Gates a UI review run (scripts/capture-ui-review.mjs) on accessibility regressions: fails when a
// serious or critical axe-core violation (view, rule, target) appears that ui-review/axe-baseline.json
// does not list. Baseline entries the run covered but no longer saw are printed so the baseline can
// be tightened; they never fail the check.
//
//   node scripts/check-ui-review.mjs <runDirectory|index.json> [--baseline=path] [--write-baseline]
//
// --write-baseline replaces the baseline with the run's violations (any index.json, including one
// downloaded from a CI artifact). The run must be complete either way: a harness failure, a failed
// axe scan, an unsettled capture, an unreachable state, or a baselined view the run never captured fails the check, since
// each would otherwise pass by not looking.
import { existsSync } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const argv = process.argv.slice(2);
const flag = (name) => argv.find((arg) => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? null;
const positional = argv.filter((arg) => !arg.startsWith('--'));
const writeBaseline = argv.includes('--write-baseline');
const baselineFile = path.resolve(flag('baseline') ?? path.join(root, 'ui-review/axe-baseline.json'));
const GATED = ['critical', 'serious'];

if (positional.length !== 1) {
  console.error('usage: node scripts/check-ui-review.mjs <runDirectory|index.json> [--baseline=path] [--write-baseline]');
  process.exit(2);
}
const indexFile = positional[0].endsWith('.json') ? path.resolve(positional[0]) : path.resolve(positional[0], 'index.json');
const index = JSON.parse(await readFile(indexFile, 'utf8'));

const problems = [];
if (index.harnessFailure) problems.push(`the harness failed: ${index.harnessFailure.split('\n')[0]}`);
if (!index.axeAvailable) problems.push('axe-core was not available to the run');
const failedScans = index.captures.filter((capture) => !capture.axe || capture.axe.error);
for (const capture of failedScans) problems.push(`axe scan failed on ${capture.file}: ${capture.axe?.error ?? 'no result'}`);
for (const capture of index.captures.filter((capture) => capture.settled === false)) problems.push(`${capture.file} was scanned before the page settled`);
for (const entry of index.unreachable ?? []) problems.push(`${entry.view} (${entry.state}) was unreachable: ${entry.reason}`);

/** (view, rule, target) -> where it was seen, from each capture's own impact. */
const observed = new Map();
const covered = new Set();
for (const capture of index.captures) {
  if (!capture.axe || capture.axe.error) continue;
  covered.add(`${capture.view}|${capture.state}|${capture.width}`);
  for (const violation of capture.axe.violations) {
    if (!GATED.includes(violation.impact)) continue;
    for (const target of violation.targets) {
      const key = JSON.stringify([capture.view, violation.id, target]);
      const entry = observed.get(key) ?? { view: capture.view, rule: violation.id, impact: violation.impact, help: violation.help, target, states: new Set(), widths: new Set(), files: [] };
      entry.states.add(capture.state);
      entry.widths.add(capture.width);
      entry.files.push(capture.file);
      observed.set(key, entry);
    }
  }
}

if (writeBaseline) {
  if (problems.length) {
    for (const problem of problems) console.error(`incomplete run: ${problem}`);
    console.error('Refusing to write a baseline from an incomplete run.');
    process.exit(1);
  }
  const views = {};
  const sorted = [...observed.values()].sort((a, b) => a.view.localeCompare(b.view) || a.rule.localeCompare(b.rule) || a.target.localeCompare(b.target));
  for (const entry of sorted) {
    const rule = ((views[entry.view] ??= {})[entry.rule] ??= { impact: entry.impact, targets: [] });
    rule.targets.push({ target: entry.target, states: [...entry.states].sort(), widths: [...entry.widths].sort((a, b) => a - b) });
  }
  const capturedViews = [...new Set(index.captures.map((capture) => capture.view))].sort();
  const baseline = {
    note: 'Serious and critical axe-core violations accepted as of this run; regenerate with `npm run ui:check -- <run> --write-baseline` after fixing some.',
    generatedFrom: { label: index.label, gitHead: index.gitHead, generatedAt: index.generatedAt, data: index.data, browser: index.browser ?? null, axeVersion: index.axeVersion, widths: index.sizes.map((size) => size.width), states: index.states ?? 'all' },
    impacts: GATED,
    views: capturedViews,
    violations: views,
  };
  await writeFile(baselineFile, `${JSON.stringify(baseline, null, 2)}\n`);
  const ruleCount = Object.values(views).reduce((sum, rules) => sum + Object.keys(rules).length, 0);
  console.log(`Wrote ${path.relative(process.cwd(), baselineFile)}: ${observed.size} (view, rule, target) entries across ${ruleCount} view-rule pairs in ${Object.keys(views).length} of ${capturedViews.length} views.`);
  process.exit(0);
}

if (!existsSync(baselineFile)) {
  console.error(`No baseline at ${baselineFile}; create one with --write-baseline.`);
  process.exit(1);
}
const baseline = JSON.parse(await readFile(baselineFile, 'utf8'));
const runViews = new Set(index.captures.map((capture) => capture.view));
// A view pinned to widths the run left out (--widths) was never attempted, so it is not missing.
const outOfScope = new Set((index.outOfScope ?? []).map((entry) => entry.view));
if (!index.only) for (const view of baseline.views) if (!runViews.has(view) && !outOfScope.has(view)) problems.push(`baselined view ${view} was not captured`);

const accepted = new Map();
for (const [view, rules] of Object.entries(baseline.violations)) {
  for (const [rule, { impact, targets }] of Object.entries(rules)) {
    for (const entry of targets) accepted.set(JSON.stringify([view, rule, entry.target]), { view, rule, impact, ...entry });
  }
}

const fresh = [...observed.entries()].filter(([key]) => !accepted.has(key)).map(([, entry]) => entry);
// Only what this run could have seen again: the same view at a state and width it captured.
const disappeared = [...accepted.entries()].filter(([key, entry]) => !observed.has(key)
  && entry.states.some((state) => entry.widths.some((width) => covered.has(`${entry.view}|${state}|${width}`)))).map(([, entry]) => entry);

if (disappeared.length) {
  console.log(`${disappeared.length} baselined violation(s) no longer present; tighten the baseline with --write-baseline:`);
  for (const entry of disappeared) console.log(`  - ${entry.view}  ${entry.rule} (${entry.impact})  ${entry.target}`);
}
if (fresh.length) {
  console.error(`${fresh.length} new serious/critical accessibility violation(s) not in ${path.relative(process.cwd(), baselineFile)}:`);
  for (const entry of fresh) {
    console.error(`  + ${entry.view}  ${entry.rule} (${entry.impact})  ${entry.target}`);
    console.error(`      ${entry.help}; seen in ${entry.files.slice(0, 3).join(', ')}${entry.files.length > 3 ? `, +${entry.files.length - 3} more` : ''}`);
  }
}
for (const problem of problems) console.error(`incomplete run: ${problem}`);
console.log(`${index.captures.length} captures, ${observed.size} serious/critical (view, rule, target) entries observed, ${accepted.size} baselined, ${fresh.length} new, ${disappeared.length} gone.`);
process.exit(fresh.length || problems.length ? 1 : 0);
