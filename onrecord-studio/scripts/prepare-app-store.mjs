import { spawnSync } from 'node:child_process';
import {
  chmodSync,
  copyFileSync,
  lstatSync,
  readFileSync,
  writeFileSync,
} from 'node:fs';
import { join, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
const tauriRoot = join(root, 'src-tauri');
const checkOnly = process.argv.includes('--check');
const identifier = 'com.onrecord.studio';

const entitlementsTemplate = read('Entitlements.appstore.plist.template');
const infoTemplate = read('Info.appstore.plist.template');
const appStoreConfig = JSON.parse(read('tauri.appstore.conf.json'));
const baseConfig = JSON.parse(read('tauri.conf.json'));

assert(baseConfig.identifier === identifier, 'The base bundle identifier changed.');
assert(baseConfig.app?.macOSPrivateApi === false, 'Mac App Store builds must disable private APIs.');
assert(
  baseConfig.app?.windows?.every((window) => window.transparent === false),
  'Mac App Store windows must not request private transparent-window APIs.',
);
assert(appStoreConfig.bundle?.macOS?.minimumSystemVersion === '13.0', 'StoreKit AppTransaction requires macOS 13.');
assert(appStoreConfig.bundle?.macOS?.entitlements === './Entitlements.appstore.plist', 'App Store entitlements path drifted.');
assert(appStoreConfig.bundle?.macOS?.infoPlist === './Info.appstore.plist', 'App Store Info.plist path drifted.');
for (const required of [
  'com.apple.security.app-sandbox',
  'com.apple.security.network.client',
  'com.apple.security.files.user-selected.read-write',
  '__APP_ID_PREFIX__',
  '__TEAM_ID__',
]) {
  assert(entitlementsTemplate.includes(required), `Entitlements template is missing ${required}.`);
}
for (const prohibited of [
  'com.apple.security.network.server',
  'com.apple.security.cs.disable-library-validation',
  'com.apple.security.cs.allow-unsigned-executable-memory',
]) {
  assert(!entitlementsTemplate.includes(prohibited), `Entitlements template grants prohibited scope ${prohibited}.`);
}
assert(infoTemplate.includes('__USES_NON_EXEMPT_ENCRYPTION__'), 'Info.plist template is missing the encryption declaration.');

if (checkOnly) {
  process.stdout.write('Mac App Store configuration and least-privilege entitlement template verified.\n');
  process.exit(0);
}

const teamId = requiredEnvironment('APPLE_TEAM_ID', /^[A-Z0-9]{10}$/);
const appIdPrefix = requiredEnvironment('APPLE_APP_ID_PREFIX', /^[A-Z0-9]{10}$/);
const profilePath = requiredEnvironment('APPLE_PROVISIONING_PROFILE_PATH');
const compliance = requiredEnvironment('ONRECORD_EXPORT_COMPLIANCE');
assert(
  compliance === 'exempt' || compliance === 'non-exempt',
  'ONRECORD_EXPORT_COMPLIANCE must be either exempt or non-exempt after reviewing the App Store Connect questionnaire.',
);

const profileMetadata = lstatSync(profilePath);
assert(profileMetadata.isFile() && !profileMetadata.isSymbolicLink(), 'The provisioning profile must be a regular file.');
assert(profileMetadata.size > 0 && profileMetadata.size <= 10 * 1024 * 1024, 'The provisioning profile size is invalid.');

const profile = decodeProfile(profilePath);
assert(
  profileStringValue(profile, 'com.apple.application-identifier') === `${appIdPrefix}.${identifier}`,
  `The provisioning profile does not authorize ${appIdPrefix}.${identifier}.`,
);
assert(
  profileStringValue(profile, 'com.apple.developer.team-identifier') === teamId,
  'The provisioning profile belongs to a different Apple team.',
);
const profileExpiration = new Date(profileDateValue(profile, 'ExpirationDate'));
assert(
  profileExpiration.getTime() > Date.now(),
  `The provisioning profile expired on ${profileExpiration.toISOString()}.`,
);

const entitlements = entitlementsTemplate
  .replaceAll('__TEAM_ID__', teamId)
  .replaceAll('__APP_ID_PREFIX__', appIdPrefix);
const usesNonExemptEncryption = compliance === 'non-exempt';
const exportCode = process.env.APPLE_EXPORT_COMPLIANCE_CODE?.trim();
const exportCodeXml = exportCode
  ? `<key>ITSEncryptionExportComplianceCode</key>\n  <string>${escapeXml(exportCode)}</string>`
  : '<!-- App Store Connect will request the applicable export declaration for this build. -->';
const infoPlist = infoTemplate
  .replace('__USES_NON_EXEMPT_ENCRYPTION__', usesNonExemptEncryption ? '<true/>' : '<false/>')
  .replace('__EXPORT_COMPLIANCE_CODE__', exportCodeXml);

writeFileSync(join(tauriRoot, 'Entitlements.appstore.plist'), entitlements, { mode: 0o600 });
writeFileSync(join(tauriRoot, 'Info.appstore.plist'), infoPlist, { mode: 0o600 });
copyFileSync(profilePath, join(tauriRoot, 'AppStore.provisionprofile'));
// Tauri copies this file into Contents/embedded.provisionprofile preserving its mode, and App
// Store Connect rejects a package holding any file only root can read (90255): the code signature
// is verified as the launching user. The profile carries certificates and entitlements, no secret,
// and ships inside every distributed copy of the app, so it is world-readable by design.
chmodSync(join(tauriRoot, 'AppStore.provisionprofile'), 0o644);
process.stdout.write('Materialized App Store entitlements, compliance metadata, and provisioning profile without exposing their values.\n');

function read(name) {
  return readFileSync(join(tauriRoot, name), 'utf8');
}

function requiredEnvironment(name, pattern) {
  const value = process.env[name]?.trim();
  assert(value, `${name} is required.`);
  if (pattern) assert(pattern.test(value), `${name} has an invalid format.`);
  return value;
}

function decodeProfile(path) {
  const result = spawnSync('security', ['cms', '-D', '-i', path], {
    encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  assert(result.status === 0, `The provisioning profile could not be decoded: ${result.stderr.trim()}`);
  return result.stdout;
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

function escapeXml(value) {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&apos;');
}
