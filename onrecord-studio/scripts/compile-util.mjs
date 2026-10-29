import fs from 'node:fs';
import ts from 'typescript';

const modules = new Map();

/** Loads the app's query builders and their relative TypeScript imports. */
export function compileUtil(name) {
  return compileFile(new URL(`../src/utils/${name}.ts`, import.meta.url));
}

function compileFile(url) {
  if (modules.has(url.href)) return modules.get(url.href);
  const source = fs.readFileSync(url, 'utf8');
  const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText;
  const module = { exports: {} };
  modules.set(url.href, module.exports);
  try {
    new Function('module', 'exports', 'require', compiled)(module, module.exports, (path) => {
      if (!path.startsWith('./') && !path.startsWith('../')) throw new Error(`Expected a relative TypeScript import: ${path}`);
      return compileFile(new URL(`${path}.ts`, url));
    });
  } catch (error) {
    modules.delete(url.href);
    throw error;
  }
  modules.set(url.href, module.exports);
  return module.exports;
}
