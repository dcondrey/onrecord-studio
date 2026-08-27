import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const rustRegistry = readFileSync(join(root, 'src-tauri/src/lib.rs'), 'utf8');
const typescriptBridge = readFileSync(join(root, 'src/ipc.ts'), 'utf8');

const rustCommands = new Set(
  [...rustRegistry.matchAll(/commands::[a-z_]+::([a-z_]+)/g)].map((match) => match[1]),
);
const mapBody = typescriptBridge.match(
  /export interface NativeCommandMap \{(?<body>[\s\S]*?)^\}/m,
)?.groups?.body;
if (!mapBody) throw new Error('Could not locate NativeCommandMap in src/ipc.ts.');
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

// The telemetry event union is hand-maintained on the TypeScript side, so a Rust variant added
// without its counterpart is invisible to `tsc` and reaches the UI as an unhandled event type.
const queueSource = readFileSync(join(root, 'src-tauri/src/ingestion/queue.rs'), 'utf8');
const progressBody = queueSource.match(
  /pub enum IngestionProgress \{(?<body>[\s\S]*?)^\}/m,
)?.groups?.body;
if (!progressBody) {
  throw new Error('Could not locate IngestionProgress in src-tauri/src/ingestion/queue.rs.');
}
const rustEvents = new Set(
  [...progressBody.matchAll(/^ {4}([A-Z][A-Za-z0-9]*)\s*\{/gm)].map((match) =>
    match[1].charAt(0).toLowerCase() + match[1].slice(1),
  ),
);

const investigationTypes = readFileSync(join(root, 'src/types/investigation.ts'), 'utf8');
const typescriptEvents = new Set(
  [...investigationTypes.matchAll(/\|\s*\{\s*type: '([a-zA-Z0-9]+)'/g)].map((match) => match[1]),
);

const missingEventsInTypescript = difference(rustEvents, typescriptEvents);
const missingEventsInRust = difference(typescriptEvents, rustEvents);
if (missingEventsInTypescript.length || missingEventsInRust.length) {
  throw new Error([
    'Rust and TypeScript ingestion telemetry unions differ.',
    `Missing from src/types/investigation.ts: ${missingEventsInTypescript.join(', ') || 'none'}`,
    `Missing from IngestionProgress: ${missingEventsInRust.join(', ') || 'none'}`,
  ].join('\n'));
}

// `VerificationState` is another hand-maintained union. A Rust variant added without its
// TypeScript counterpart type-checks on both sides and reaches the UI as an unhandled label,
// which for this enum means a legal conclusion rendered as an unknown string.
const verificationBody = readFileSync(join(root, 'src-tauri/src/types/audit.rs'), 'utf8').match(
  /pub enum VerificationState \{(?<body>[\s\S]*?)^\}/m,
)?.groups?.body;
if (!verificationBody) {
  throw new Error('Could not locate VerificationState in src-tauri/src/types/audit.rs.');
}
const rustVerificationStates = new Set(
  [...verificationBody.matchAll(/^ {4}([A-Z][A-Za-z0-9]*),$/gm)].map((match) => match[1]),
);
const auditTypes = readFileSync(join(root, 'src/types/audit.ts'), 'utf8');
const verificationUnion = auditTypes.match(
  /export type VerificationState =(?<body>[\s\S]*?);/,
)?.groups?.body;
if (!verificationUnion) {
  throw new Error('Could not locate the VerificationState union in src/types/audit.ts.');
}
const typescriptVerificationStates = new Set(
  [...verificationUnion.matchAll(/'([A-Za-z0-9]+)'/g)].map((match) => match[1]),
);
const missingStatesInTypescript = difference(rustVerificationStates, typescriptVerificationStates);
const missingStatesInRust = difference(typescriptVerificationStates, rustVerificationStates);
if (missingStatesInTypescript.length || missingStatesInRust.length) {
  throw new Error([
    'Rust and TypeScript VerificationState unions differ.',
    `Missing from src/types/audit.ts: ${missingStatesInTypescript.join(', ') || 'none'}`,
    `Missing from the Rust enum: ${missingStatesInRust.join(', ') || 'none'}`,
  ].join('\n'));
}

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
    `${rustEvents.size} telemetry events and ${rustVerificationStates.size} verification states ` +
    'agree across the bridge.',
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
