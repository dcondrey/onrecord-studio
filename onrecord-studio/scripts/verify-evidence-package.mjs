#!/usr/bin/env node
// Independently verifies one or more signed OnRecord Studio evidence/chain-of-custody packages
// (`export_legal_brief`'s `.evidence.json` or `export_case_evidence_package`'s `.json` output)
// without running the application, trusting its source, or having Rust installed.
//
// What a passing result proves: the package's `manifest` has not changed since this installation's
// local Ed25519 key signed it. What it does NOT prove: that the signing key belongs to a
// particular real-world person or organization (see `identityAssurance` below), or that any claim
// inside the manifest is true. A citation the manifest itself marks unverified stays unverified;
// no signature changes that. A citation the manifest marks *withheld* is not present at all — its
// label and reason are, so a withholding is visible, never silent.
//
// Given more than one chain-of-custody package (`ledgerSequence`/`previousExportSha256` present),
// this also checks chain continuity across them: each package's `previousExportSha256` must equal
// the prior package's `manifestSha256`, in ledger-sequence order. That only proves the *given*
// files were not reordered or had a link removed between them — it says nothing about an export
// sequence number the caller was never given a file for, which is what "reserved, not confirmed
// written" versus "withheld" means inside the application's own ledger.
//
// An optional `--revocations <file>` names a copy of the installation's distributed
// `.onrecord-export-signing-key-revocations-v1.json` (itself a signed package, self-verified the
// same way). Given one, every verified package's signer is cross-checked against it: this is the
// only way this standalone script can ever learn a key was retired, since it has no access to the
// application's local state.
//
// Usage: node scripts/verify-evidence-package.mjs [--revocations <file>] <path-to-package.json> [more-packages.json ...]

import { readFile } from 'node:fs/promises';
import { createHash, webcrypto } from 'node:crypto';
import { dirname, join } from 'node:path';
import process from 'node:process';

function parseArgs() {
  const args = process.argv.slice(2);
  let revocationsPath = null;
  const paths = [];
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === '--revocations') {
      revocationsPath = args[i + 1];
      i += 1;
    } else {
      paths.push(args[i]);
    }
  }
  return { paths, revocationsPath };
}

function hexToBytes(hex) {
  if (hex.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(hex)) {
    throw new Error(`Not a hex string: ${hex.slice(0, 16)}...`);
  }
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i += 1) {
    bytes[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

// The signature block is not inside the signed bytes, so the key id that revocation lookups and
// "signed by" use must be bound to the key that actually verified, or any key could claim any id.
function signatureBlockBound(signature, publicKeyBytes) {
  return (
    createHash('sha256').update(publicKeyBytes).digest('hex') === String(signature.signerKeyId).toLowerCase() &&
    signature.algorithm === 'Ed25519' &&
    signature.identityAssurance === 'SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED'
  );
}

function deepEqual(a, b) {
  if (a === b) return true;
  if (typeof a !== typeof b || a === null || b === null) return false;
  if (typeof a !== 'object') return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  return aKeys.every((key) => deepEqual(a[key], b[key]));
}

// Fails CLOSED, not open: if `--revocations` was given, this either returns a verified entry list
// or the whole process exits non-zero. It must never silently fall back to "no list given" —
// that would let a tampered or unsigned revocations file be indistinguishable from none at all,
// defeating the one thing this file exists to guarantee.
async function loadRevocations(path) {
  if (!path) return null;
  const raw = await readFile(path, 'utf8');
  const package_ = JSON.parse(raw);
  const canonicalBytes = Buffer.from(package_.manifestCanonicalBase64, 'base64');
  const publicKeyBytes = hexToBytes(package_.signature.publicKeyHex);
  const signatureBytes = hexToBytes(package_.signature.signatureHex);
  const key = await webcrypto.subtle.importKey('raw', publicKeyBytes, { name: 'Ed25519' }, false, ['verify']);
  const valid =
    signatureBlockBound(package_.signature, publicKeyBytes) &&
    (await webcrypto.subtle.verify('Ed25519', key, signatureBytes, canonicalBytes));
  const hashMatches = createHash('sha256').update(canonicalBytes).digest('hex') === package_.manifestSha256;
  // The entry list is read from the verified canonical bytes, never from `package_.manifest`
  // directly: that field is unsigned JSON sitting next to the signature, so an attacker could
  // delete a COMPROMISED entry from it, leave manifestCanonicalBase64 untouched, and this would
  // still say "verified" if the entries came from the wrong place.
  let canonicalManifest;
  try {
    canonicalManifest = JSON.parse(canonicalBytes.toString('utf8'));
  } catch {
    canonicalManifest = null;
  }
  const structurallyConsistent =
    canonicalManifest !== null && deepEqual(canonicalManifest, package_.manifest);
  // A correctly signed *anything* is not a revocation list: without this, a case evidence package
  // or legal brief passed to --revocations would verify and yield an empty (harmless-looking)
  // entry array, silently defeating the flag rather than rejecting the wrong file.
  const isRevocationList =
    structurallyConsistent &&
    canonicalManifest.schemaVersion === 'onrecord-signing-key-revocations-v1' &&
    Array.isArray(canonicalManifest.entries);
  if (!valid || !hashMatches || !isRevocationList) {
    console.error(`FAIL: the revocation list at ${path} does not verify, or is not a signing-key revocation list. Refusing to proceed with it rather than silently treating it as absent.`);
    process.exit(1);
  }
  console.log(`\nLoaded ${canonicalManifest.entries.length} revoked signing key(s) from ${path}, signed by key ${package_.signature.signerKeyId} (list itself verified).`);
  return canonicalManifest.entries;
}

async function verifyOne(path, revocations) {
  const raw = await readFile(path, 'utf8');
  const package_ = JSON.parse(raw);
  if (!('manifestCanonicalBase64' in package_) && 'manifestSha256' in package_ && 'manifest' in package_) {
    console.error('FAIL: this looks like a pre-v3 OnRecord Studio package (no embedded canonical bytes). This verifier requires the v3+ format; re-export it from a current build to verify it this way.');
    return { path, package_: null, allPassed: false };
  }
  const required = ['manifestCanonicalBase64', 'manifestSha256', 'manifest', 'signature'];
  for (const field of required) {
    if (!(field in package_)) {
      console.error(`FAIL: package is missing required field "${field}" — this is not a package this tool produced.`);
      return { path, package_: null, allPassed: false };
    }
  }

  const canonicalBytes = Buffer.from(package_.manifestCanonicalBase64, 'base64');
  const results = [];

  const computedHash = createHash('sha256').update(canonicalBytes).digest('hex');
  const hashMatches = computedHash === package_.manifestSha256;
  results.push([hashMatches, 'Canonical bytes hash to the recorded manifestSha256']);

  let signatureValid = false;
  try {
    const publicKeyBytes = hexToBytes(package_.signature.publicKeyHex);
    const signatureBytes = hexToBytes(package_.signature.signatureHex);
    const key = await webcrypto.subtle.importKey('raw', publicKeyBytes, { name: 'Ed25519' }, false, ['verify']);
    signatureValid =
      signatureBlockBound(package_.signature, publicKeyBytes) &&
      (await webcrypto.subtle.verify('Ed25519', key, signatureBytes, canonicalBytes));
  } catch (error) {
    console.error(`Signature check could not run: ${error.message}`);
  }
  results.push([signatureValid, `Ed25519 signature over the canonical bytes verifies against the embedded public key, whose SHA-256 is the signer key id (${package_.signature.algorithm ?? 'unknown algorithm'})`]);

  let manifestConsistent;
  try {
    manifestConsistent = deepEqual(JSON.parse(canonicalBytes.toString('utf8')), package_.manifest);
  } catch {
    manifestConsistent = false;
  }
  results.push([manifestConsistent, 'The human-readable "manifest" field matches the signed canonical bytes structurally']);

  // A research-snapshot manifest describes a sibling `.duckdb` file by name and SHA-256. When that
  // file is present beside the manifest, its digest is checked too, so one command verifies both
  // the signature and that the database being queried is the one signed for.
  const { fileName, fileSha256 } = package_.manifest ?? {};
  if (typeof fileName === 'string' && typeof fileSha256 === 'string') {
    const siblingPath = join(dirname(path), fileName);
    const fileDigest = await readFile(siblingPath).then(
      (bytes) => createHash('sha256').update(bytes).digest('hex'),
      () => null,
    );
    if (fileDigest === null) {
      console.log(`\nNote: ${fileName} is not beside the manifest, so its digest was not checked (expected ${fileSha256}).`);
    } else {
      results.push([fileDigest === fileSha256, `Sibling file ${fileName} hashes to the manifest's fileSha256`]);
    }
  }

  const allPassed = results.every(([passed]) => passed);
  console.log(`\nVerifying: ${path}`);
  console.log(`Schema: ${package_.manifest?.schemaVersion ?? 'unknown'}\n`);
  for (const [passed, label] of results) {
    console.log(`${passed ? 'PASS' : 'FAIL'}  ${label}`);
  }
  console.log(`\nIdentity assurance: ${package_.signature.identityAssurance ?? 'unknown'}`);
  console.log('This label is the actual claim: a self-managed local key proves only that whoever');
  console.log('held that key file signed these bytes, not a notarized or third-party-attested identity.');

  let signerCompromised = false;
  if (Array.isArray(revocations)) {
    // The worst entry wins: a key can carry both a RETIRED and a later COMPROMISED entry, and
    // taking the first match found would let that pair read as merely retired.
    const matching = revocations.filter((entry) => entry.keyId === package_.signature?.signerKeyId);
    const revoked = matching.find((entry) => entry.status !== 'RETIRED') ?? matching.find((entry) => entry.status === 'RETIRED') ?? null;
    if (revoked) {
      // Fail closed: only an exact 'RETIRED' status is informational; anything else is treated
      // as compromised, never silently trusted.
      signerCompromised = revoked.status !== 'RETIRED';
      if (signerCompromised) {
        console.log(`\nCOMPROMISED: signer key ${package_.signature.signerKeyId} was revoked as compromised on ${revoked.revokedAt} (${revoked.reason}).`);
        console.log('The signature over these bytes is still mathematically valid; treat this package as untrusted anyway.');
      } else {
        console.log(`\nRetired: signer key ${package_.signature.signerKeyId} was retired (routine rotation, not compromise) on ${revoked.revokedAt} (${revoked.reason}).`);
        console.log('This is informational — packages this key already signed remain trusted.');
      }
    }
  }

  if (Array.isArray(package_.manifest?.unverifiedCitationLabels) && package_.manifest.unverifiedCitationLabels.length > 0) {
    console.log(`\n${package_.manifest.unverifiedCitationLabels.length} citation(s) inside this signed manifest are themselves marked unverified:`);
    for (const label of package_.manifest.unverifiedCitationLabels) {
      console.log(`  - ${label}`);
    }
    console.log('A valid signature does not make these true; it only proves they were exported unchanged.');
  }

  if (Array.isArray(package_.manifest?.withheldCitations) && package_.manifest.withheldCitations.length > 0) {
    console.log(`\n${package_.manifest.withheldCitations.length} citation(s) were withheld from this package's evidence, by label and reason:`);
    for (const entry of package_.manifest.withheldCitations) {
      console.log(`  - ${entry.label} (${entry.reason})`);
    }
  }

  const overallPassed = allPassed && !signerCompromised;
  console.log(`\nOverall: ${overallPassed ? 'PASS — the package is internally consistent and unaltered since signing.' : 'FAIL — see above. Treat this package as unverified.'}`);
  return { path, package_, allPassed: overallPassed };
}

// Checks that the given packages' chain-of-custody manifests link into one unbroken sequence,
// scoped per case (the ledger is per-case, so two different cases both legitimately have a
// sequence 1 — that is not a fork). Within one case's packages, sorted by `ledgerSequence`:
// - only a CONSECUTIVE pair (sequence N, N+1) is link-checked: current.previousExportSha256 must
//   equal prior.manifestSha256. A non-consecutive pair is reported as a GAP, not checked and not
//   a failure — the application's own ledger, not this script, is what distinguishes "never
//   exported", "reserved but not confirmed written", and "withheld".
// - two packages at the SAME sequence with different hashes is a FORK: always a failure.
function verifyChain(verified) {
  const withSequence = verified.filter(
    (entry) => entry.package_ && typeof entry.package_.manifest?.ledgerSequence === 'number',
  );
  if (withSequence.length < 2) return true;

  const byCase = new Map();
  for (const entry of withSequence) {
    const caseId = entry.package_.manifest?.case?.id ?? '(unknown case)';
    if (!byCase.has(caseId)) byCase.set(caseId, []);
    byCase.get(caseId).push(entry);
  }

  let chainPassed = true;
  for (const [caseId, entries] of byCase) {
    entries.sort((a, b) => a.package_.manifest.ledgerSequence - b.package_.manifest.ledgerSequence);
    if (entries.length < 2) continue;
    console.log(`\nChecking chain continuity for case "${caseId}" across ${entries.length} export(s):`);
    for (let i = 1; i < entries.length; i += 1) {
      const prior = entries[i - 1];
      const current = entries[i];
      const priorSequence = prior.package_.manifest.ledgerSequence;
      const currentSequence = current.package_.manifest.ledgerSequence;
      if (currentSequence === priorSequence) {
        const forked = current.package_.manifestSha256 !== prior.package_.manifestSha256;
        console.log(`  ${forked ? 'FAIL' : 'PASS'}  sequence ${currentSequence} appears twice (${prior.path}, ${current.path})${forked ? ' with DIFFERENT hashes — this is a forked chain' : ' with the same hash'}`);
        chainPassed = chainPassed && !forked;
        continue;
      }
      if (currentSequence !== priorSequence + 1) {
        console.log(`  GAP   sequence ${priorSequence} to ${currentSequence}: not consecutive (files for the sequence(s) between them were not given to this verifier, or were never written). Not checked, not a failure.`);
        continue;
      }
      const linked = current.package_.manifest.previousExportSha256 === prior.package_.manifestSha256;
      console.log(`  ${linked ? 'PASS' : 'FAIL'}  sequence ${currentSequence} (${current.path}) links to sequence ${priorSequence} (${prior.path}) via previousExportSha256`);
      chainPassed = chainPassed && linked;
    }
  }
  console.log(`\nChain: ${chainPassed ? 'PASS — every consecutive pair given links correctly, and no case forked.' : 'FAIL — a link between consecutive exports does not match, or a sequence forked. Treat the sequence as tampered or reordered.'}`);
  return chainPassed;
}

async function main() {
  const { paths, revocationsPath } = parseArgs();
  if (paths.length === 0) {
    console.error('Usage: node scripts/verify-evidence-package.mjs [--revocations <file>] <path-to-package.json> [more-packages.json ...]');
    process.exit(2);
  }
  const revocations = await loadRevocations(revocationsPath);
  const verified = [];
  for (const path of paths) {
    verified.push(await verifyOne(path, revocations));
  }
  const everyPackagePassed = verified.every((entry) => entry.allPassed);
  const chainPassed = verifyChain(verified);
  process.exit(everyPackagePassed && chainPassed ? 0 : 1);
}

main().catch((error) => {
  console.error(`Could not verify package: ${error.message}`);
  process.exit(1);
});
