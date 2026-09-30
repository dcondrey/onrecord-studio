import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
// loadEnvFile never overrides a variable the shell already set.
const localEnvironment = join(root, '.env.direct.local');
if (existsSync(localEnvironment)) process.loadEnvFile(localEnvironment);

const checkOnly = process.argv.includes('--check');
const dryRun = process.argv.includes('--dry-run');

const steps = [
  ['npm', ['run', 'prepare:direct']],
  [
    'npx',
    [
      'tauri', 'build', '--ci', '--bundles', 'app', '--target', 'universal-apple-darwin',
      '--features', 'direct-license', '--config', 'src-tauri/tauri.direct.conf.json',
    ],
  ],
  ['npm', ['run', 'notarize:direct']],
  ['node', ['scripts/package-direct.mjs']],
];

if (checkOnly || dryRun) {
  // Validation without credentials: no build, no notarization, nothing written.
  const validations = [
    ['node', ['scripts/prepare-direct.mjs', '--check']],
    ['node', ['scripts/notarize-direct.mjs', '--check']],
    ['node', ['scripts/package-direct.mjs', checkOnly ? '--check' : '--dry-run']],
  ];
  if (dryRun) {
    process.stdout.write(`Plan:\n${steps.map(([command, args]) => `  ${command} ${args.join(' ')}`).join('\n')}\n`);
  }
  for (const [command, args] of validations) {
    const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', env: process.env });
    if (result.status !== 0) process.exit(result.status ?? 1);
  }
  process.exit(0);
}

for (const [command, args] of steps) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', env: process.env });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
