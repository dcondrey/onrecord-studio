import { spawnSync } from 'node:child_process';
import {
  existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync,
} from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import process from 'node:process';
import { parsePublicKey, verifyUpdaterSignature } from './updater-signature-direct.mjs';

const root = resolve(import.meta.dirname, '..');
const tauriRoot = join(root, 'src-tauri');
const args = process.argv.slice(2);
const mode = args.includes('--check') ? 'check' : args.includes('--dry-run') ? 'dry-run' : 'package';
const identifier = 'com.onrecord.studio';
const teamId = 'U3PZN7P3E5';
const productName = 'OnRecord Studio';
const feedTemplate = 'https://updates.writerslogic.com/onrecord/updates/{{target}}/{{arch}}/{{current_version}}';
const targets = ['darwin-aarch64', 'darwin-x86_64'];

const localEnvironment = join(root, '.env.direct.local');
if (existsSync(localEnvironment)) process.loadEnvFile(localEnvironment);

assert(process.platform === 'darwin', 'Direct packaging runs on macOS.');
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
assert(/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version), `package.json version "${version}" is not semver.`);
const directConfig = JSON.parse(readFileSync(join(tauriRoot, 'tauri.direct.conf.json'), 'utf8'));
const updater = directConfig.plugins?.updater;
const keyPath = resolve(process.env.ONRECORD_UPDATER_KEY_PATH ?? join(homedir(), '.config', 'onrecord-studio', 'signing', 'updater.key'));
const passwordPath = process.env.ONRECORD_UPDATER_KEY_PASSWORD_PATH ?? `${keyPath}.password`;
const notesPath = resolve(argumentValue('--notes') ?? process.env.RELEASE_NOTES_PATH ?? join(root, 'RELEASE_NOTES.md'));
const outputDir = resolve(argumentValue('--out') ?? join(root, 'dist-tauri', 'release', version));
const baseUrl = process.env.RELEASE_BASE_URL?.trim().replace(/\/+$/, '');
const fileStem = `OnRecord-Studio_${version}_universal`;
const files = {
  dmg: join(outputDir, `${fileStem}.dmg`),
  archive: join(outputDir, `${fileStem}.app.tar.gz`),
  signature: join(outputDir, `${fileStem}.app.tar.gz.sig`),
  feed: join(outputDir, 'latest.json'),
};

const pending = [];
validateConfiguration();
if (mode === 'check') check();
else if (mode === 'dry-run') dryRun();
else packageRelease();

function validateConfiguration() {
  assert(updater && typeof updater === 'object', 'tauri.direct.conf.json has no plugins.updater block.');
  assert(updater.endpoints?.length === 1 && updater.endpoints[0] === feedTemplate, `The updater endpoint must be exactly ${feedTemplate}.`);
  assert(!('dialog' in updater), 'plugins.updater.dialog does not exist in Tauri 2; the app shows its own prompt.');
  verifyUpdaterSignatureShape(updater.pubkey);
  assert(directConfig.bundle?.createUpdaterArtifacts === undefined, 'Updater artifacts are built here after stapling, not by tauri build.');
  const base = JSON.parse(readFileSync(join(tauriRoot, 'tauri.conf.json'), 'utf8'));
  assert(!JSON.stringify(base).includes('"updater"'), 'The base config must not configure the updater; the App Store build never self-updates.');
  assert(!base.app?.security?.capabilities?.includes('direct-updater-capability'), 'The base config lists the updater capability.');
  assert(directConfig.app?.security?.capabilities?.includes('direct-updater-capability'), 'The direct config does not list the updater capability.');
  assert(existsSync(join(tauriRoot, 'capabilities', 'direct.json')), 'capabilities/direct.json is missing.');
  if (baseUrl !== undefined) assert(/^https:\/\/[^\s]+$/.test(baseUrl), 'RELEASE_BASE_URL must be an https URL.');
}

function verifyUpdaterSignatureShape(pubkey) {
  assert(typeof pubkey === 'string' && pubkey.length > 0, 'plugins.updater.pubkey is empty.');
  try {
    parsePublicKey(pubkey);
  } catch (error) {
    assert(false, error.message);
  }
}

function check() {
  for (const tool of ['hdiutil', 'codesign', 'spctl', 'tar', 'ditto', 'plutil']) {
    assert(spawnSync('which', [tool]).status === 0, `${tool} is not available.`);
  }
  for (const tool of ['notarytool', 'stapler']) {
    assert(spawnSync('xcrun', ['--find', tool]).status === 0, `${tool} is not available.`);
  }
  const versions = spawnSync(process.execPath, [join(root, 'scripts', 'prepare-direct.mjs'), '--check'], { encoding: 'utf8' });
  assert(versions.status === 0, `prepare-direct --check failed: ${(versions.stderr || versions.stdout).trim()}`);

  if (existsSync(keyPath)) {
    assertPrivate(keyPath);
    const password = readPassword();
    if (password) proveKeyMatchesConfig(password);
    else pending.push(`updater key password (${passwordPath} or TAURI_SIGNING_PRIVATE_KEY_PASSWORD)`);
  } else pending.push(`updater private key at ${keyPath}`);
  if (!existsSync(notesPath)) pending.push(`release notes at ${notesPath}`);
  else assert(readFileSync(notesPath, 'utf8').trim().length > 0, `${notesPath} is empty.`);
  if (baseUrl === undefined) pending.push('RELEASE_BASE_URL');
  for (const name of ['APPLE_SIGNING_IDENTITY', 'APPLE_NOTARY_KEYCHAIN_PROFILE']) {
    if (!process.env[name]?.trim()) pending.push(name);
  }
  process.stdout.write('Direct packaging configuration, updater key pairing, feed format, and tooling verified.\n');
  if (pending.length) process.stdout.write(`Not checkable until provided: ${pending.join('; ')}.\n`);
}

function dryRun() {
  check();
  const plan = [
    `1. Locate ${productName}.app (--app, ONRECORD_APP_BUNDLE_PATH, or the Cargo universal bundle); require Developer ID signature, team ${teamId}, stapled ticket, Gatekeeper "Notarized Developer ID", CFBundleShortVersionString ${version}.`,
    `2. Build ${files.dmg} with hdiutil; sign with APPLE_SIGNING_IDENTITY; notarize with APPLE_NOTARY_KEYCHAIN_PROFILE; staple; spctl --type open.`,
    `3. Archive the stapled app to ${files.archive}; sign it with ${keyPath}; verify the signature against the configured public key; extract and re-verify codesign and stapler.`,
    `4. Write ${files.feed} for ${targets.join(', ')} with notes from ${notesPath}, URLs under ${baseUrl ?? '$RELEASE_BASE_URL'}/${version}/.`,
    '5. Upload nothing. The owner hosts the files.',
  ];
  process.stdout.write(`${plan.join('\n')}\n`);
}

function packageRelease() {
  check();
  assert(pending.length === 0, `Cannot package: missing ${pending.join('; ')}.`);
  const password = readPassword();
  const identity = requiredEnvironment('APPLE_SIGNING_IDENTITY');
  const profile = requiredEnvironment('APPLE_NOTARY_KEYCHAIN_PROFILE');
  const appPath = resolveApp();
  verifyApp(appPath);
  assert(run('plutil', ['-extract', 'CFBundleShortVersionString', 'raw', '-o', '-', join(appPath, 'Contents', 'Info.plist')]) === version, `The bundle version does not match package.json ${version}.`);

  rmSync(outputDir, { recursive: true, force: true });
  mkdirSync(outputDir, { recursive: true });
  const work = mkdtempSync(join(tmpdir(), 'onrecord-package-'));
  try {
    buildDmg(appPath, work, identity, profile);
    buildUpdaterArchive(appPath, work, password);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
  writeFeed();
  process.stdout.write(`Packaged ${version}: ${files.dmg}\n${files.archive}\n${files.signature}\n${files.feed}\n`);
}

function buildDmg(appPath, work, identity, profile) {
  const staging = join(work, 'dmg');
  mkdirSync(staging);
  run('ditto', [appPath, join(staging, `${productName}.app`)]);
  symlinkSync('/Applications', join(staging, 'Applications'));
  run('hdiutil', ['create', '-volname', productName, '-srcfolder', staging, '-fs', 'HFS+', '-format', 'UDZO', '-ov', files.dmg]);
  run('codesign', ['--force', '--timestamp', '--sign', identity, files.dmg]);
  run('codesign', ['--verify', '--strict', files.dmg]);
  const result = JSON.parse(run('xcrun', ['notarytool', 'submit', files.dmg, '--keychain-profile', profile, '--wait', '--output-format', 'json']));
  assert(result.status === 'Accepted', `DMG notarization ended as ${result.status}; inspect it with: xcrun notarytool log ${result.id} --keychain-profile <profile>`);
  run('xcrun', ['stapler', 'staple', files.dmg]);
  run('xcrun', ['stapler', 'validate', files.dmg]);
  const assessment = run('spctl', ['--assess', '--type', 'open', '--context', 'context:primary-signature', '-vv', files.dmg]);
  assert(/source=Notarized Developer ID/.test(assessment), `Gatekeeper did not accept the DMG as notarized: ${assessment}`);
}

function buildUpdaterArchive(appPath, work, password) {
  const parent = join(appPath, '..');
  // COPYFILE_DISABLE keeps AppleDouble entries out of the archive; they would break the signature seal.
  run('tar', ['-czf', files.archive, '-C', parent, `${productName}.app`], { COPYFILE_DISABLE: '1' });
  run('npx', ['tauri', 'signer', 'sign', '--private-key-path', keyPath, files.archive], { TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password });
  assert(existsSync(files.signature), 'tauri signer did not write the .sig file.');
  verifyUpdaterSignature({
    pubkey: updater.pubkey,
    signature: readFileSync(files.signature, 'utf8'),
    data: readFileSync(files.archive),
  });
  // The archive must carry a bundle Gatekeeper still accepts: what the updater installs is what is verified here.
  const extracted = join(work, 'extracted');
  mkdirSync(extracted);
  run('tar', ['-xzf', files.archive, '-C', extracted]);
  verifyApp(join(extracted, `${productName}.app`));
}

function writeFeed() {
  const notes = readFileSync(notesPath, 'utf8').trim();
  const signature = readFileSync(files.signature, 'utf8').trim();
  const url = `${baseUrl}/${version}/${encodeURIComponent(`${fileStem}.app.tar.gz`)}`;
  const platforms = Object.fromEntries(targets.map((target) => [target, { signature, url }]));
  const feed = { version, notes, pub_date: new Date().toISOString(), platforms };
  writeFileSync(files.feed, `${JSON.stringify(feed, null, 2)}\n`);
  assertFeed(JSON.parse(readFileSync(files.feed, 'utf8')));
}

function assertFeed(feed) {
  assert(feed.version === version && feed.notes.length > 0 && !Number.isNaN(Date.parse(feed.pub_date)), 'latest.json header is invalid.');
  for (const target of targets) {
    const entry = feed.platforms?.[target];
    assert(entry && /^https:\/\//.test(entry.url) && entry.signature.length > 0, `latest.json has no valid ${target} entry.`);
  }
}

function verifyApp(appPath) {
  run('codesign', ['--verify', '--deep', '--strict', appPath]);
  run('xcrun', ['stapler', 'validate', appPath]);
  const assessment = run('spctl', ['--assess', '--type', 'execute', '-vv', appPath]);
  assert(/source=Notarized Developer ID/.test(assessment), `Gatekeeper did not report a notarized Developer ID: ${assessment}`);
}

function resolveApp() {
  const explicit = argumentValue('--app') ?? process.env.ONRECORD_APP_BUNDLE_PATH;
  const target = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).scripts['build:direct'] ? 'universal-apple-darwin' : null;
  const metadata = JSON.parse(run('cargo', ['metadata', '--format-version', '1', '--no-deps', '--manifest-path', join(tauriRoot, 'Cargo.toml')]));
  const path = resolve(explicit ?? join(metadata.target_directory, ...(target ? [target] : []), 'release', 'bundle', 'macos', `${productName}.app`));
  assert(existsSync(path) && lstatSync(path).isDirectory(), `App bundle not found: ${path}`);
  return path;
}

function proveKeyMatchesConfig(password) {
  const scratch = mkdtempSync(join(tmpdir(), 'onrecord-keycheck-'));
  try {
    const sample = join(scratch, 'sample');
    writeFileSync(sample, 'onrecord updater key pairing check');
    run('npx', ['tauri', 'signer', 'sign', '--private-key-path', keyPath, sample], { TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password });
    verifyUpdaterSignature({ pubkey: updater.pubkey, signature: readFileSync(`${sample}.sig`, 'utf8'), data: readFileSync(sample) });
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

function readPassword() {
  const fromEnvironment = process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  if (fromEnvironment) return fromEnvironment;
  if (!existsSync(passwordPath)) return null;
  assertPrivate(passwordPath);
  return readFileSync(passwordPath, 'utf8').trim();
}

function assertPrivate(path) {
  const metadata = statSync(path);
  assert(metadata.isFile(), `${path} is not a regular file.`);
  assert((metadata.mode & 0o077) === 0, `${path} must be mode 600.`);
}

function requiredEnvironment(name) {
  const value = process.env[name]?.trim();
  assert(value, `${name} is required.`);
  return value;
}

function argumentValue(name) {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
}

// Output is returned to the caller only; commands here never receive a secret on argv.
function run(command, commandArgs, extraEnv = {}) {
  const result = spawnSync(command, commandArgs, {
    cwd: root, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, env: { ...process.env, ...extraEnv },
  });
  if (result.error) throw result.error;
  assert(result.status === 0, `${command} ${commandArgs[0] ?? ''} failed: ${(result.stderr || result.stdout).trim()}`);
  return `${result.stdout}${result.stderr}`.trim();
}

function assert(condition, message) {
  if (!condition) {
    process.stderr.write(`package-direct: ${message}\n`);
    process.exit(1);
  }
}

