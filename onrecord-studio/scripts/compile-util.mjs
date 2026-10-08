import fs from 'node:fs';
import ts from 'typescript';

const modules = new Map();

/** Loads `src/utils/<name>.ts` and its relative `./` imports, so a fixture generator runs the app's own query builders. */
export function compileUtil(name) {
  if (modules.has(name)) return modules.get(name);
  const source = fs.readFileSync(new URL(`../src/utils/${name}.ts`, import.meta.url), 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const module = { exports: {} };
  new Function('module', 'exports', 'require', compiled)(module, module.exports, (path) => compileUtil(path.replace('./', '')));
  modules.set(name, module.exports);
  return module.exports;
}
