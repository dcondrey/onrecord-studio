import { readdirSync, readFileSync, statSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, relative, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
execFileSync(process.execPath, [join(root, 'scripts/generate-contracts.mjs'), '--check'], { stdio: 'inherit' });
const rustRegistry = readFileSync(join(root, 'src-tauri/src/lib.rs'), 'utf8');
const typescriptBridge = readFileSync(join(root, 'src/generated/nativeCommands.ts'), 'utf8');

const rustCommands = new Set(
  [...rustRegistry.matchAll(/commands::[a-z_]+::([a-z_]+)/g)].map((match) => match[1]),
);
const mapBody = typescriptBridge.match(
  /export interface NativeCommandMap \{(?<body>[\s\S]*?)^\}/m,
)?.groups?.body;
if (!mapBody) throw new Error('Could not locate NativeCommandMap in src/generated/nativeCommands.ts.');
const typescriptCommands = new Set(
  [...mapBody.matchAll(/^ {2}([a-z][a-z0-9_]+):/gm)].map((match) => match[1]),
);

const missingInTypescript = difference(rustCommands, typescriptCommands);
const missingInRust = difference(typescriptCommands, rustCommands);
if (missingInTypescript.length || missingInRust.length) {
  throw new Error([
    'Rust and TypeScript IPC command registries differ.',
    `Missing from NativeCommandMap: ${missingInTypescript.join(', ') || 'none'}`,
    `Missing from the Rust handler: ${missingInRust.join(', ') || 'none'}`,
  ].join('\n'));
}

// The generator's check mode compares byte-for-byte output derived from Rust with the files that
// TypeScript imports. Counts here make the successful audit visible without maintaining a second
// parser or another list that can itself drift.
const telemetryContracts = readFileSync(join(root, 'src/generated/ingestionTelemetry.ts'), 'utf8');
const auditContracts = readFileSync(join(root, 'src/generated/auditContracts.ts'), 'utf8');
const telemetryEventCount = [...telemetryContracts.matchAll(/^ {2}\| \{ type:/gm)].length;
const verificationStateCount = auditContracts
  .match(/export type VerificationState =(?<body>[\s\S]*?);/)?.groups?.body
  ?.match(/^ {2}\| /gm)?.length ?? 0;

const directInvocations = walk(join(root, 'src'))
  .filter((path) => /\.(?:ts|tsx)$/.test(path) && path !== join(root, 'src/ipc.ts'))
  .filter((path) => /\binvoke(?:<[^>]+>)?\s*\(/.test(readFileSync(path, 'utf8')))
  .map((path) => relative(root, path));
if (directInvocations.length) {
  throw new Error(
    `Native commands must be routed through src/ipc.ts. Direct invoke calls found in: ${directInvocations.join(', ')}`,
  );
}

globalThis.console.log(
  `IPC contract verified: ${rustCommands.size} commands are typed and centralized, ` +
    `${telemetryEventCount} telemetry events and ${verificationStateCount} verification states ` +
    'are generated from the native definitions.',
);

function difference(left, right) {
  return [...left].filter((value) => !right.has(value)).sort();
}

function walk(directory) {
  return readdirSync(directory).flatMap((entry) => {
    const path = join(directory, entry);
    return statSync(path).isDirectory() ? walk(path) : [path];
  });
}
