import { spawnSync } from 'node:child_process';
import { existsSync, lstatSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
const tauriRoot = join(root, 'src-tauri');
const checkOnly = process.argv.includes('--check');
const identifier = 'com.onrecord.studio';
const teamId = 'U3PZN7P3E5';

assert(process.platform === 'darwin', 'Notarization runs on macOS.');
for (const tool of ['notarytool', 'stapler', 'codesign', 'spctl', 'ditto']) {
  assert(spawnSync('xcrun', ['--find', tool]).status === 0 || spawnSync('which', [tool]).status === 0, `${tool} is not available.`);
}
const bundleTarget = configuredBuildTarget();

if (checkOnly) {
  process.stdout.write('Direct notarization tooling and bundle path resolution verified.\n');
  process.exit(0);
}

// A notarytool keychain profile (xcrun notarytool store-credentials) keeps the Apple ID or API key
// out of the repository, the environment, and the process list.
const keychainProfile = requiredEnvironment('APPLE_NOTARY_KEYCHAIN_PROFILE');
const appPath = resolve(
  argumentValue('--app')
    ?? process.env.ONRECORD_APP_BUNDLE_PATH
    ?? join(cargoTargetDirectory(), ...(bundleTarget ? [bundleTarget] : []), 'release', 'bundle', 'macos', 'OnRecord Studio.app'),
);
assert(existsSync(appPath) && lstatSync(appPath).isDirectory(), `App bundle not found: ${appPath}`);

const infoPlist = join(appPath, 'Contents', 'Info.plist');
assert(run('plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-', infoPlist]) === identifier, `Bundle is not ${identifier}.`);
assert(
  existsSync(join(appPath, 'Contents', 'embedded.provisionprofile')),
  'The bundle has no embedded provisioning profile; run prepare:direct before building.',
);

// Refuse to submit anything not signed for distribution by this team with the hardened runtime.
run('codesign', ['--verify', '--deep', '--strict', appPath]);
const details = runCapture('codesign', ['-dvv', appPath]);
assert(/Authority=Developer ID Application:/.test(details), 'The app is not signed with a Developer ID Application identity.');
assert(new RegExp(`TeamIdentifier=${teamId}\\b`).test(details), `The app is not signed by team ${teamId}.`);
assert(/flags=0x[0-9a-f]+\(.*runtime.*\)/.test(details), 'The app is signed without the hardened runtime.');

const workDir = mkdtempSync(join(tmpdir(), 'onrecord-notarize-'));
try {
  const archive = join(workDir, 'OnRecord Studio.zip');
  run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', appPath, archive]);
  const submission = runCapture('xcrun', [
    'notarytool', 'submit', archive, '--keychain-profile', keychainProfile, '--wait', '--output-format', 'json',
  ]);
  const result = JSON.parse(submission);
  assert(result.status === 'Accepted', `Notarization ended as ${result.status}; inspect it with: xcrun notarytool log ${result.id} --keychain-profile <profile>`);
} finally {
  rmSync(workDir, { recursive: true, force: true });
}

run('xcrun', ['stapler', 'staple', appPath]);
run('xcrun', ['stapler', 'validate', appPath]);
const assessment = runCapture('spctl', ['--assess', '--type', 'execute', '-vv', appPath]);
assert(/source=Notarized Developer ID/.test(assessment), `Gatekeeper did not report a notarized Developer ID: ${assessment.trim()}`);
process.stdout.write('Notarized, stapled, and Gatekeeper-assessed the direct build.\n');

function requiredEnvironment(name) {
  const value = process.env[name]?.trim();
  assert(value, `${name} is required.`);
  return value;
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function run(command, args) {
  return runCapture(command, args).trim();
}

// spctl and codesign write their findings to stderr, so both streams are returned.
function runCapture(command, args) {
  const result = spawnSync(command, args, { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 });
  if (result.error) throw result.error;
  assert(result.status === 0, `${command} ${args[0] ?? ''} failed: ${(result.stderr || result.stdout).trim()}`);
  return `${result.stdout}${result.stderr}`;
}

function cargoTargetDirectory() {
  const metadata = JSON.parse(
    run('cargo', ['metadata', '--format-version', '1', '--no-deps', '--manifest-path', join(tauriRoot, 'Cargo.toml')]),
  );
  return metadata.target_directory;
}

// package.json's build:direct script is just `node scripts/build-direct.mjs`; the --target flag
// itself is baked into that script's own `tauri build` invocation, not into the npm script text.
function configuredBuildTarget() {
  const source = readFileSync(join(root, 'scripts', 'build-direct.mjs'), 'utf8');
  const flag = source.match(/'--target',\s*'([^']+)'/);
  return flag ? flag[1] : null;
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
