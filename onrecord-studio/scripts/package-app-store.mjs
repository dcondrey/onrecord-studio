import { spawnSync } from 'node:child_process';
import {
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
} from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
const tauriRoot = join(root, 'src-tauri');
const identifier = 'com.onrecord.studio';
// Cargo nests a cross-target build under its triple, so the bundle path depends on whether
// `build:app-store` passes `--target`. Reading the triple back out of that script keeps the two in
// step: hardcoding it here means dropping the flag would leave this reading a path that still holds
// the previous build, and the whole chain would then validate and sign that stale bundle.
const bundleTarget = configuredBuildTarget();
const appPath = resolve(
  argumentValue('--app')
    ?? process.env.ONRECORD_APP_BUNDLE_PATH
    ?? join(cargoTargetDirectory(), ...bundleTarget ? [bundleTarget] : [], 'release', 'bundle', 'macos', 'OnRecord Studio.app'),
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
const profileApplicationIdentifier = profileStringValue(profile, 'com.apple.application-identifier');
const profileTeamIdentifier = profileStringValue(profile, 'com.apple.developer.team-identifier');
assert(profileTeamIdentifier === teamId, 'The provisioning profile belongs to a different Apple team.');
assert(
  profileApplicationIdentifier === `${teamId}.${identifier}`,
  `The provisioning profile authorizes ${profileApplicationIdentifier}, not ${teamId}.${identifier}.`,
);
const profileExpiration = new Date(profileDateValue(profile, 'ExpirationDate'));
assert(
  profileExpiration.getTime() > Date.now(),
  `The embedded provisioning profile expired on ${profileExpiration.toISOString()}.`,
);

// otool prints one unindented header line per architecture, so a universal binary emits several
// rather than the single one a `.slice(1)` would drop. Every dependency line is indented; keeping
// only those survives both a single-architecture and a universal build.
const links = run('otool', ['-L', executable])
  .split('\n')
  .filter((line) => /^\s/.test(line))
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

// The App Store listing offers the app only to the architectures it actually carries, so a bundle
// that lost one is a silently narrowed release rather than a build error. Every Mach-O file is
// checked, not just the main executable: the dependency allowlist above admits `@rpath` and
// `@loader_path`, and a universal executable beside a single-architecture bundled library still
// fails at dyld load on the architecture that library is missing.
const requiredArchitectures = { 'aarch64-apple-darwin': ['arm64'] }[bundleTarget];
if (requiredArchitectures) {
  for (const machO of machObjectsIn(join(appPath, 'Contents'))) {
    const architectures = run('lipo', ['-archs', machO]).split(/\s+/).filter(Boolean);
    for (const architecture of requiredArchitectures) {
      assert(
        architectures.includes(architecture),
        `${machO} is missing the ${architecture} architecture; it carries ${architectures.join(', ')}.`,
      );
    }
  }
}

// App Store Connect rejects a package containing anything a non-root user cannot read (90255),
// because the code signature is verified under the launching user. Staged build inputs are written
// with restrictive modes, and Tauri copies some of them into the bundle mode and all, so this is
// checked on the finished bundle rather than trusted from the staging step.
for (const path of bundleEntriesIn(appPath)) {
  const mode = lstatSync(path).mode;
  const required = lstatSync(path).isDirectory() ? 0o005 : 0o004;
  assert(
    (mode & required) === required,
    `${path} is not readable by non-root users (mode ${(mode & 0o777).toString(8)}).`,
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
// A Mac App Store submission is signed by a "3rd Party Mac Developer Installer" certificate, and
// that chain is deliberately not trusted by macOS for direct installation: pkgutil reports it as
// "signed by a developer certificate issued by Apple (Development)". Only App Store Connect accepts
// it. Demanding a macOS-trusted signature here would require a Developer ID Installer certificate,
// which App Store Connect rejects, so this asserts the submission chain instead. An unsigned or
// damaged package exits non-zero from pkgutil, which runCombined already fails on.
const packageSignature = runCombined('pkgutil', ['--check-signature', packagePath]);
assert(
  new RegExp(`3rd Party Mac Developer Installer: .*\\(${escapeForRegExp(teamId)}\\)`).test(packageSignature),
  'The installer was not signed by this team\'s 3rd Party Mac Developer Installer certificate.',
);
for (const authority of [
  'Apple Worldwide Developer Relations Certification Authority',
  'Apple Root CA',
]) {
  assert(packageSignature.includes(authority), `The installer certificate chain is missing ${authority}.`);
}
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

function configuredBuildTarget() {
  const script = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).scripts['build:app-store'];
  assert(typeof script === 'string', 'package.json has no build:app-store script.');
  const flag = script.match(/--target\s+(\S+)/);
  return flag ? flag[1] : null;
}

function bundleEntriesIn(directory) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) continue;
    found.push(path);
    if (entry.isDirectory()) found.push(...bundleEntriesIn(path));
  }
  return found;
}

function machObjectsIn(directory) {
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isSymbolicLink()) continue;
    if (entry.isDirectory()) found.push(...machObjectsIn(path));
    else if (entry.isFile() && run('file', ['-b', path]).includes('Mach-O')) found.push(path);
  }
  return found;
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

function escapeForRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function profileStringValue(plistXml, key) {
  const match = plistXml.match(
    new RegExp(`<key>${escapeForRegExp(key)}</key>\\s*<string>([^<]*)</string>`),
  );
  assert(match, `The provisioning profile is missing its ${key} entitlement.`);
  return match[1];
}

function profileDateValue(plistXml, key) {
  const match = plistXml.match(
    new RegExp(`<key>${escapeForRegExp(key)}</key>\\s*<date>([^<]*)</date>`),
  );
  assert(match, `The provisioning profile is missing its ${key} field.`);
  return match[1];
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
