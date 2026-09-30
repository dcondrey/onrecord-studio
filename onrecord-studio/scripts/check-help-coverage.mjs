// Fails when a workspace view, a FindingKind, or a registered Tauri command group has no Help
// topic, or when a topic points at something that no longer exists. The three source lists are
// read from the files that define them, so a new view, kind or command module cannot ship
// without documentation.
//
// Usage: node scripts/check-help-coverage.mjs [--topics-dir <directory of topics-*.json>]
import { readdirSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
const args = process.argv.slice(2);
const dirFlag = args.indexOf('--topics-dir');
const topicsDir = dirFlag >= 0 ? resolve(args[dirFlag + 1] ?? '') : join(root, 'src/content/help');

const read = (path) => readFileSync(join(root, path), 'utf8');

const views = new Set(
  [...(read('src/store/useAppStore.ts').match(/export type WorkspaceView =([^;]+);/)?.[1] ?? '').matchAll(/'([a-z]+)'/g)].map((m) => m[1]),
);
const kinds = new Set(
  [...(read('src/types/findings.ts').match(/export type FindingKind =([^;]+);/)?.[1] ?? '').matchAll(/'([A-Z_]+)'/g)].map((m) => m[1]),
);
const groups = new Set([...read('src-tauri/src/lib.rs').matchAll(/commands::([a-z_]+)::[a-z_]+/g)].map((m) => m[1]));
const shots = new Set([...read('src/components/help/shots.ts').matchAll(/^ {2}(?:'([a-z-]+)'|([a-z]+)):\s*\{/gm)].map((m) => m[1] ?? m[2]));
const figures = new Set(['evidence-ladder', 'provenance-ladder']);

if (views.size === 0 || kinds.size === 0 || groups.size === 0) {
  throw new Error(`Could not parse sources: ${views.size} views, ${kinds.size} finding kinds, ${groups.size} command groups.`);
}

const topics = readdirSync(topicsDir)
  .filter((name) => name.startsWith('topics-') && name.endsWith('.json'))
  .sort()
  .flatMap((name) => JSON.parse(readFileSync(join(topicsDir, name), 'utf8')));

const problems = [];
const ids = new Set();
for (const topic of topics) {
  if (ids.has(topic.id)) problems.push(`duplicate topic id ${topic.id}`);
  ids.add(topic.id);
}
for (const topic of topics) {
  for (const id of topic.related ?? []) if (!ids.has(id)) problems.push(`topic ${topic.id} relates to missing topic ${id}`);
  for (const view of topic.views ?? []) if (!views.has(view)) problems.push(`topic ${topic.id} documents unknown view ${view}`);
  for (const kind of topic.findingKinds ?? []) if (!kinds.has(kind)) problems.push(`topic ${topic.id} documents unknown finding kind ${kind}`);
  for (const group of topic.commandGroups ?? []) if (!groups.has(group)) problems.push(`topic ${topic.id} documents unknown command group ${group}`);
  if (topic.screenshot && !shots.has(topic.screenshot)) problems.push(`topic ${topic.id} names missing screenshot ${topic.screenshot}`);
  if (topic.figure && !figures.has(topic.figure)) problems.push(`topic ${topic.id} names unknown figure ${topic.figure}`);
}

const covered = (field) => new Set(topics.flatMap((topic) => topic[field] ?? []));
for (const [label, expected, field] of [
  ['view', views, 'views'],
  ['FindingKind', kinds, 'findingKinds'],
  ['command group', groups, 'commandGroups'],
]) {
  const have = covered(field);
  for (const item of [...expected].sort()) if (!have.has(item)) problems.push(`${label} ${item} has no help topic`);
}

// Every finding kind and verification state needs plain-language text, read from the same type
// definitions the rest of the app uses. The tables are Records, so tsc already refuses a missing key;
// this catches it without a build and also catches a key whose fields are empty or absent.
const tableBody = (source, name) => {
  const start = source.indexOf(`export const ${name}`);
  const end = source.indexOf('\n};', start);
  return start < 0 || end < 0 ? '' : source.slice(start, end);
};
const plainSource = read('src/content/plainLanguage.ts');
const states = new Set(
  [...(read('src/generated/auditContracts.ts').match(/export type VerificationState =([^;]+);/)?.[1] ?? '').matchAll(/'([A-Za-z]+)'/g)].map((m) => m[1]),
);
if (states.size === 0) throw new Error('Could not parse VerificationState.');
for (const [label, expected, table] of [
  ['FindingKind', kinds, 'PLAIN_FINDING_TEXT'],
  ['VerificationState', states, 'PLAIN_VERIFICATION_TEXT'],
]) {
  const body = tableBody(plainSource, table);
  const keys = [...body.matchAll(/^ {2}([A-Za-z_]+): \{/gm)].map((m) => m[1]);
  for (const item of [...expected].sort()) if (!keys.includes(item)) problems.push(`${label} ${item} has no plain-language text in ${table}`);
  for (const key of keys) if (!expected.has(key)) problems.push(`${table} has text for unknown ${label} ${key}`);
  for (const field of ['observed', 'notShown', 'next']) {
    const filled = [...body.matchAll(new RegExp(`^ {4}${field}: (?:'[^']+'|"[^"]+"|[A-Z_]+),$`, 'gm'))].length;
    if (filled !== keys.length) problems.push(`${table} has ${filled} non-empty ${field} entries for ${keys.length} keys`);
  }
}

if (problems.length) {
  process.stderr.write(`Help coverage check failed:\n${problems.map((p) => `  ${p}`).join('\n')}\n`);
  process.exit(1);
}
process.stdout.write(
  `Help coverage verified: ${topics.length} topics cover ${views.size} views, ${kinds.size} finding kinds and ${groups.size} command groups; plain-language text covers ${kinds.size} kinds and ${states.size} verification states.\n`,
);
