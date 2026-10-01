import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, existsSync, lstatSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import process from 'node:process';

const root = resolve(import.meta.dirname, '..');
const tauriRoot = join(root, 'src-tauri');
const checkOnly = process.argv.includes('--check');
const identifier = 'com.onrecord.studio';
const teamId = 'U3PZN7P3E5';

const entitlements = read('Entitlements.direct.plist');
const infoTemplate = read('Info.direct.plist.template');
const directConfig = JSON.parse(read('tauri.direct.conf.json'));
const baseConfig = JSON.parse(read('tauri.conf.json'));

assert(baseConfig.identifier === identifier, 'The base bundle identifier changed.');
assert(baseConfig.app?.macOSPrivateApi === false, 'Direct builds must disable private APIs.');
const macOS = directConfig.bundle?.macOS;
assert(macOS?.minimumSystemVersion === '13.0', 'The direct build requires macOS 13.');
assert(macOS?.hardenedRuntime === true, 'Notarization requires the hardened runtime.');
assert(macOS?.entitlements === './Entitlements.direct.plist', 'Direct entitlements path drifted.');
assert(macOS?.infoPlist === './Info.direct.plist', 'Direct Info.plist path drifted.');
assert(
  macOS?.files?.['embedded.provisionprofile'] === './Direct.provisionprofile',
  'The direct provisioning profile path drifted.',
);
assert(macOS?.signingIdentity === undefined, 'The signing identity comes from APPLE_SIGNING_IDENTITY, not this file.');
for (const required of [
  'com.apple.application-identifier',
  'com.apple.developer.team-identifier',
  'keychain-access-groups',
  'com.apple.security.network.client',
  `${teamId}.${identifier}`,
]) {
  assert(entitlements.includes(required), `Direct entitlements are missing ${required}.`);
}
for (const prohibited of [
  'com.apple.security.app-sandbox',
  'com.apple.security.network.server',
  'com.apple.security.cs.',
  'com.apple.developer.applesignin',
  'com.apple.developer.associated-domains',
  'com.apple.developer.hardened-process',
  'com.apple.security.hardened-process',
]) {
  assert(!entitlements.includes(prohibited), `Direct entitlements grant prohibited scope ${prohibited}.`);
}
assert(infoTemplate.includes('__SUPABASE_ANON_KEY__'), 'Info.plist template is missing the anon key placeholder.');
assert(
  /<key>CFBundleURLSchemes<\/key>\s*<array>\s*<string>com\.onrecord\.studio<\/string>\s*<\/array>/.test(infoTemplate),
  'Info.plist template does not register the com.onrecord.studio URL scheme.',
);

const cargoVersion = read('Cargo.toml').match(/^\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
const packageVersion = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
assert(
  cargoVersion && cargoVersion === packageVersion && cargoVersion === baseConfig.version,
  `Versions disagree: package.json ${packageVersion}, src-tauri/Cargo.toml ${cargoVersion}, tauri.conf.json ${baseConfig.version}.`,
);

if (checkOnly) {
  process.stdout.write('Direct-distribution configuration and least-privilege entitlements verified.\n');
  process.exit(0);
}

const anonKey = requiredEnvironment('ONRECORD_SUPABASE_ANON_KEY', /^[A-Za-z0-9._-]{16,512}$/);
const profilePath = requiredEnvironment('APPLE_DIRECT_PROVISIONING_PROFILE_PATH');

const profileMetadata = lstatSync(profilePath);
assert(profileMetadata.isFile() && !profileMetadata.isSymbolicLink(), 'The provisioning profile must be a regular file.');
assert(profileMetadata.size > 0 && profileMetadata.size <= 10 * 1024 * 1024, 'The provisioning profile size is invalid.');
const profile = decodeProfile(profilePath);
assert(
  profileStringValue(profile, 'com.apple.application-identifier') === `${teamId}.${identifier}`,
  `The provisioning profile does not authorize ${teamId}.${identifier}.`,
);
assert(
  profileStringValue(profile, 'com.apple.developer.team-identifier') === teamId,
  'The provisioning profile belongs to a different Apple team.',
);
const expiration = new Date(profileDateValue(profile, 'ExpirationDate'));
assert(expiration.getTime() > Date.now(), `The provisioning profile expired on ${expiration.toISOString()}.`);
assertProfileGrantsEntitlements(profile);
assertProfileCoversSigningIdentity(profile, requiredEnvironment('APPLE_SIGNING_IDENTITY'));
writeFileSync(join(tauriRoot, 'Info.direct.plist'), infoTemplate.replace('__SUPABASE_ANON_KEY__', anonKey), {
  mode: 0o600,
});
copyFileSync(profilePath, join(tauriRoot, 'Direct.provisionprofile'));
// Tauri copies the profile into Contents/embedded.provisionprofile preserving its mode; the code
// signature is verified as the launching user, so it must be world-readable. It carries no secret.
chmodSync(join(tauriRoot, 'Direct.provisionprofile'), 0o644);
process.stdout.write('Materialized the direct Info.plist and provisioning profile without exposing their values.\n');

function read(name) {
  return readFileSync(join(tauriRoot, name), 'utf8');
}

// A git-ignored .env.direct.local (KEY=value lines) fills variables the shell has not set, so the
// public anon key does not have to be exported for every build. The shell always wins.
function localEnvironment(name) {
  const file = join(root, '.env.direct.local');
  if (!existsSync(file)) return undefined;
  const line = readFileSync(file, 'utf8')
    .split('\n')
    .find((entry) => entry.startsWith(`${name}=`));
  return line?.slice(name.length + 1).trim().replace(/^(['"])(.*)\1$/, '$2');
}

function requiredEnvironment(name, pattern) {
  const value = (process.env[name] ?? localEnvironment(name))?.trim();
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

// Restricted entitlements are honored only when the embedded profile grants them; AMFI kills the
// launch otherwise. Unrestricted com.apple.security.* keys never appear in a profile.
function assertProfileGrantsEntitlements(profileXml) {
  const granted = profileEntitlements(profileXml);
  assert(granted && typeof granted === 'object', 'The provisioning profile has no Entitlements dictionary.');
  const wanted = plistJson(entitlements, 'Entitlements.direct.plist');
  for (const [key, value] of Object.entries(wanted)) {
    if (key.startsWith('com.apple.security.')) continue;
    assert(key in granted, `The provisioning profile does not grant ${key}.`);
    const allowed = Array.isArray(granted[key]) ? granted[key] : [granted[key]];
    for (const item of Array.isArray(value) ? value : [value]) {
      assert(
        allowed.some((entry) => entry === item || (typeof entry === 'string' && entry.endsWith('*') && String(item).startsWith(entry.slice(0, -1)))),
        `The provisioning profile does not grant ${key}=${item}.`,
      );
    }
  }
}

// A profile vouches for its entitlements only when the app is signed by one of the profile's own
// certificates; signed by any other identity of the same team, the build succeeds and AMFI kills
// every launch ("Unsatisfied entitlements").
function assertProfileCoversSigningIdentity(profileXml, signingIdentity) {
  const identities = spawnSync('security', ['find-identity', '-v', '-p', 'codesigning'], { encoding: 'utf8' });
  assert(identities.status === 0, 'The code-signing identities could not be listed.');
  const matches = [...identities.stdout.matchAll(/^\s*\d+\) ([0-9A-F]{40}) "([^"]+)"/gm)]
    .filter(([, hash, name]) => hash === signingIdentity.toUpperCase() || name === signingIdentity)
    .map(([, hash]) => hash);
  assert(new Set(matches).size === 1, `APPLE_SIGNING_IDENTITY matches ${new Set(matches).size} valid identities; set it to one certificate's SHA-1.`);
  const count = Number(plistRaw(profileXml, 'DeveloperCertificates'));
  assert(Number.isInteger(count) && count > 0, 'The provisioning profile lists no certificates.');
  const profileHashes = Array.from({ length: count }, (_, index) => createHash('sha1')
    .update(Buffer.from(plistRaw(profileXml, `DeveloperCertificates.${index}`), 'base64'))
    .digest('hex')
    .toUpperCase());
  assert(
    profileHashes.includes(matches[0]),
    `The provisioning profile does not include the signing certificate ${matches[0]}; it lists ${profileHashes.join(', ')}. Regenerate the profile with that certificate.`,
  );
}

function plistRaw(xml, keyPath) {
  const result = spawnSync('plutil', ['-extract', keyPath, 'raw', '-o', '-', '-'], { input: xml, encoding: 'utf8' });
  assert(result.status === 0, `The provisioning profile has no readable ${keyPath}.`);
  return result.stdout.trim();
}

function plistJson(xml, label) {
  const result = spawnSync('plutil', ['-convert', 'json', '-o', '-', '-'], { input: xml, encoding: 'utf8' });
  assert(result.status === 0, `${label} could not be parsed as a property list.`);
  return JSON.parse(result.stdout);
}

// A profile also holds certificate data and dates, which JSON cannot represent, so only the
// Entitlements subtree is converted.
function profileEntitlements(xml) {
  const result = spawnSync('plutil', ['-extract', 'Entitlements', 'json', '-o', '-', '-'], { input: xml, encoding: 'utf8' });
  assert(result.status === 0, 'The provisioning profile has no readable Entitlements.');
  return JSON.parse(result.stdout);
}

function escapeForRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function profileStringValue(plistXml, key) {
  const match = plistXml.match(new RegExp(`<key>${escapeForRegExp(key)}</key>\\s*<string>([^<]*)</string>`));
  assert(match, `The provisioning profile is missing its ${key} entitlement.`);
  return match[1];
}

function profileDateValue(plistXml, key) {
  const match = plistXml.match(new RegExp(`<key>${escapeForRegExp(key)}</key>\\s*<date>([^<]*)</date>`));
  assert(match, `The provisioning profile is missing its ${key} field.`);
  return match[1];
}

function assert(condition, message) {
  if (!condition) throw new Error(message);
}
