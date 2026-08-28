import { spawnSync } from 'node:child_process';
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
const tauriRoot = join(root, 'src-tauri');
const identifier = 'com.onrecord.studio';
const appPath = resolve(
  argumentValue('--app')
    ?? process.env.ONRECORD_APP_BUNDLE_PATH
    ?? join(cargoTargetDirectory(), 'release', 'bundle', 'macos', 'OnRecord Studio.app'),
);

assert(process.platform === 'darwin', 'Mac App Store packages must be produced on macOS.');
assert(existsSync(appPath) && lstatSync(appPath).isDirectory(), `App bundle not found: ${appPath}`);

const infoPlist = join(appPath, 'Contents', 'Info.plist');
const embeddedProfile = join(appPath, 'Contents', 'embedded.provisionprofile');
const bundleIdentifier = run('plutil', ['-extract', 'CFBundleIdentifier', 'raw', '-o', '-', infoPlist]);
const executableName = run('plutil', ['-extract', 'CFBundleExecutable', 'raw', '-o', '-', infoPlist]);
const executable = join(appPath, 'Contents', 'MacOS', executableName);
assert(bundleIdentifier === identifier, `Expected ${identifier}, found ${bundleIdentifier}.`);
assert(existsSync(executable) && lstatSync(executable).isFile(), 'The app executable is missing.');
assert(existsSync(embeddedProfile) && lstatSync(embeddedProfile).isFile(), 'The embedded provisioning profile is missing.');

run('codesign', ['--verify', '--deep', '--strict', '--verbose=4', appPath]);
const signing = runCombined('codesign', ['-dvvv', appPath]);
assert(!signing.includes('Signature=adhoc'), 'The app has only an ad-hoc signature.');
assert(!signing.includes('TeamIdentifier=not set'), 'The app signature has no Apple team identifier.');
const teamId = requiredEnvironment('APPLE_TEAM_ID', /^[A-Z0-9]{10}$/);
assert(signing.includes(`TeamIdentifier=${teamId}`), 'The app was signed by a different Apple team.');

const entitlements = runCombined('codesign', ['-d', '--entitlements', ':-', appPath]);
for (const required of [
  'com.apple.security.app-sandbox',
  'com.apple.application-identifier',
  'com.apple.developer.team-identifier',
]) {
  assert(entitlements.includes(required), `The signed app is missing ${required}.`);
}
for (const prohibited of [
  'com.apple.security.network.server',
  'com.apple.security.cs.disable-library-validation',
  'com.apple.security.cs.allow-unsigned-executable-memory',
]) {
  assert(!entitlements.includes(prohibited), `The signed app grants prohibited scope ${prohibited}.`);
}

const profile = run('security', ['cms', '-D', '-i', embeddedProfile]);
assert(profile.includes(teamId), 'The provisioning profile belongs to a different Apple team.');
assert(profile.includes(identifier), 'The provisioning profile does not authorize this bundle identifier.');

const links = run('otool', ['-L', executable])
  .split('\n')
  .slice(1)
  .map((line) => line.trim().split(/\s+/, 1)[0])
  .filter(Boolean);
for (const link of links) {
  assert(
    link.startsWith('/System/Library/')
      || link.startsWith('/usr/lib/')
      || link.startsWith('@rpath/')
      || link.startsWith('@loader_path/')
      || link.startsWith('@executable_path/'),
    `The app depends on a non-system library outside its bundle: ${link}`,
  );
}

const executableBytes = readFileSync(executable);
assert(
  !executableBytes.includes(Buffer.from('sourceMappingURL=')),
  'The release executable embeds a frontend source-map reference.',
);

if (process.argv.includes('--validate-only')) {
  process.stdout.write(`Validated signed Mac App Store bundle: ${appPath}\n`);
  process.exit(0);
}

const installerIdentity = requiredEnvironment('APPLE_INSTALLER_SIGNING_IDENTITY');
const packagePath = resolve(
  argumentValue('--output')
    ?? process.env.ONRECORD_APP_STORE_PKG_PATH
    ?? join(root, 'release', 'OnRecord Studio.pkg'),
);
mkdirSync(dirname(packagePath), { recursive: true });
run('xcrun', [
  'productbuild',
  '--sign',
  installerIdentity,
  '--component',
  appPath,
  '/Applications',
  packagePath,
]);
const packageSignature = runCombined('pkgutil', ['--check-signature', packagePath]);
assert(packageSignature.includes('signed by a certificate trusted by macOS'), 'The installer signature is not trusted by macOS.');
process.stdout.write(`Created and verified signed Mac App Store package: ${packagePath}\n`);

function cargoTargetDirectory() {
  const metadata = JSON.parse(run('cargo', [
    'metadata',
    '--format-version',
    '1',
    '--no-deps',
    '--manifest-path',
    join(tauriRoot, 'Cargo.toml'),
  ]));
  return metadata.target_directory;
}

function argumentValue(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) return null;
  assert(index + 1 < process.argv.length, `${name} requires a value.`);
  return process.argv[index + 1];
}

function requiredEnvironment(name, pattern) {
  const value = process.env[name]?.trim();
  assert(value, `${name} is required.`);
  if (pattern) assert(pattern.test(value), `${name} has an invalid format.`);
  return value;
}

function run(command, arguments_) {
  const result = spawnSync(command, arguments_, {
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  assert(
    result.status === 0,
    `${command} failed (${result.status ?? 'no exit status'}): ${(result.stderr || result.stdout).trim()}`,
  );
  return result.stdout.trim();
}

function runCombined(command, arguments_) {
  const result = spawnSync(command, arguments_, {
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  assert(
    result.status === 0,
    `${command} failed (${result.status ?? 'no exit status'}): ${(result.stderr || result.stdout).trim()}`,
  );
  return `${result.stdout}\n${result.stderr}`;
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
