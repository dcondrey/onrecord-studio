import fs from 'node:fs';
import ts from 'typescript';
import { fixture as base } from './discovery-fixture.mjs';
const modules = new Map();
function compile(name) {
  if (modules.has(name)) return modules.get(name);
  const source = fs.readFileSync(new URL(`../src/utils/${name}.ts`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } }).outputText;
  const module = { exports: {} };
  new Function('module', 'exports', 'require', compiled)(module, module.exports, (path) => compile(path.replace('./', '')));
  modules.set(name, module.exports);
  return module.exports;
}
const { automaticProfileQuery, automaticFollowups } = compile('automaticDiscovery');
const schema = JSON.parse(JSON.stringify(base.schema).replaceAll('"group"', '"city"'));
const profile = { columns: ['column_name', 'source_rows', 'missing_rows', 'distinct_values'], rows: [['year', 28, 0, 4], ['city', 28, 1, 4], ['phase', 28, 0, 2], ['amount', 28, 1, 13]], rowCount: 4, truncated: false, elapsedMs: 0, provenance: ['analysis_fixture@fixture'] };
export const fixture = { csv: base.csv.replace('group,year', 'city,year'), schema, profile, queries: [automaticProfileQuery(schema, schema.datasets[0]), ...automaticFollowups(schema, schema.datasets[0], profile)] };
if (process.argv.includes('--write')) fs.writeFileSync(new URL('../src-tauri/tests/fixtures/automatic-discovery.json', import.meta.url), `${JSON.stringify(fixture, null, 2)}\n`);
