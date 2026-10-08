#!/usr/bin/env node
// Independently verifies a verifiable OnRecord Studio report (.html) from the command line, with
// node's standard library only. It re-derives everything the in-page check does, plus what a page
// cannot do for itself: it rebuilds the template from the file (payload slot restored to its
// placeholder) and compares its SHA-256 with the digest inside the signed payload, so a change to
// the page's own markup or script fails here even though the payload is untouched.
//
// A pass proves the file is unchanged since the embedded key signed it. It does not identify the
// key's holder, and it does not check key revocation.
//
// Exit codes: 0 verified, 1 not verified, 2 cannot verify (an environment limit, not a finding
// about the file).
//
// Usage: node scripts/verify-report.mjs <report.html>

import { readFile } from 'node:fs/promises';
import { createHash, webcrypto } from 'node:crypto';
import process from 'node:process';
import { pathToFileURL } from 'node:url';

const SCHEMA = 'onrecord-verifiable-report-v1';
const OPEN_TAG = '<script type="application/json" id="onrecord-report-payload">';
const CLOSE_TAG = '</script>';
const PLACEHOLDER = '__ONRECORD_REPORT_PAYLOAD__';

function sha256Hex(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function hexToBytes(hex) {
  if (typeof hex !== 'string' || hex.length === 0 || hex.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(hex)) {
    throw new Error('not a hex string');
  }
  return Uint8Array.from(Buffer.from(hex, 'hex'));
}

function deepEqual(a, b) {
  if (a === b) return true;
  if (typeof a !== typeof b || a === null || b === null || typeof a !== 'object') return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const keys = Object.keys(a);
  if (keys.length !== Object.keys(b).length) return false;
  return keys.every((key) => Object.hasOwn(b, key) && deepEqual(a[key], b[key]));
}

function strictBase64(value) {
  if (typeof value !== 'string' || !/^[A-Za-z0-9+/]*={0,2}$/.test(value) || value.length % 4 !== 0) return null;
  return Buffer.from(value, 'base64');
}

export async function verifyReportHtml(html) {
  const results = [];
  let cannotRun = false;
  const record = (ok, label) => results.push([ok, label]);

  const first = html.indexOf(OPEN_TAG);
  const last = html.lastIndexOf(OPEN_TAG);
  const bodyStart = first + OPEN_TAG.length;
  const bodyEnd = html.indexOf(CLOSE_TAG, bodyStart);
  const oneSlot = first !== -1 && first === last && bodyEnd !== -1;
  record(oneSlot, 'The file holds exactly one report payload');
  if (!oneSlot) return { results, state: 'failed' };

  const payloadText = html.slice(bodyStart, bodyEnd);
  let pkg;
  try {
    pkg = JSON.parse(payloadText);
  } catch {
    record(false, 'The payload is readable JSON');
    return { results, state: 'failed' };
  }
  const structured = ['manifestCanonicalBase64', 'manifestSha256', 'manifest', 'signature'].every((field) => pkg && field in pkg);
  record(structured, 'The payload has the fields of a signed package');
  if (!structured) return { results, state: 'failed' };

  const canonical = strictBase64(pkg.manifestCanonicalBase64);
  record(canonical !== null, 'The signed payload is well-formed base64');
  if (canonical === null) return { results, state: 'failed' };

  record(sha256Hex(canonical) === String(pkg.manifestSha256).toLowerCase(), 'SHA-256 of the signed payload matches the recorded digest');

  const sig = pkg.signature ?? {};
  record(sig.algorithm === 'Ed25519', 'Signature algorithm is Ed25519');
  let publicKey = null;
  let signature = null;
  try {
    publicKey = hexToBytes(sig.publicKeyHex);
    signature = hexToBytes(sig.signatureHex);
  } catch {
    /* recorded below */
  }
  const wellFormed = publicKey !== null && signature !== null && publicKey.length === 32 && signature.length === 64;
  record(wellFormed, 'Public key and signature are well-formed');
  if (wellFormed) {
    record(sha256Hex(publicKey) === String(sig.signerKeyId).toLowerCase(), 'Signer key id is the SHA-256 of the public key');
    try {
      const key = await webcrypto.subtle.importKey('raw', publicKey, { name: 'Ed25519' }, false, ['verify']);
      record(await webcrypto.subtle.verify('Ed25519', key, signature, canonical), 'Ed25519 signature over the signed payload verifies');
    } catch (error) {
      if (error?.name === 'NotSupportedError') {
        cannotRun = true;
        results.push([null, 'Ed25519 signature check could not run: this Node does not support Ed25519']);
      } else {
        record(false, `Ed25519 signature check failed: ${error?.message ?? error}`);
      }
    }
  }

  let manifest;
  try {
    manifest = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(canonical));
  } catch {
    manifest = null;
  }
  record(manifest !== null && deepEqual(manifest, pkg.manifest), 'The "manifest" field matches the signed bytes');
  const schemaOk = manifest !== null && manifest.schemaVersion === SCHEMA && Array.isArray(manifest.claims);
  record(schemaOk, 'The signed payload is a report of the expected schema');

  if (schemaOk) {
    const template = html.slice(0, bodyStart) + PLACEHOLDER + html.slice(bodyEnd);
    record(
      sha256Hex(Buffer.from(template, 'utf8')) === manifest.templateSha256,
      'The page markup and script are those the payload was signed for (template SHA-256)',
    );
    record(
      manifest.claims.every(
        (claim) =>
          claim.kind !== 'REDACTION_CANDIDATE_INFERRED' &&
          (claim.evidenceBasis !== 'COUNTED_VIOLATION' || claim.confidence?.state === 'MEASURED'),
      ),
      'No inferred redaction identity appears as a claim, and every legal claim has a measured confidence',
    );
  }

  const failed = results.some(([ok]) => ok === false);
  return { results, state: failed ? 'failed' : cannotRun ? 'unverifiable' : 'verified', manifest };
}

async function main() {
  const path = process.argv[2];
  if (!path || process.argv.length !== 3) {
    process.stderr.write('Usage: node scripts/verify-report.mjs <report.html>\n');
    process.exit(2);
  }
  let html;
  try {
    html = await readFile(path, 'utf8');
  } catch (error) {
    process.stderr.write(`Cannot verify: ${path} could not be read (${error.message}).\n`);
    process.exit(2);
  }
  const { results, state, manifest } = await verifyReportHtml(html);
  for (const [ok, label] of results) {
    process.stdout.write(`${ok === true ? 'PASS' : ok === false ? 'FAIL' : 'N/A '}  ${label}\n`);
  }
  if (state === 'verified') {
    process.stdout.write(`\nVERIFIED: ${manifest.claims.length} claim(s), ${manifest.withheld.length} withholding group(s). Signer identity and key revocation are not checked.\n`);
    process.exit(0);
  }
  if (state === 'unverifiable') {
    process.stdout.write('\nCANNOT VERIFY: at least one check could not run here.\n');
    process.exit(2);
  }
  process.stdout.write('\nNOT VERIFIED: this file was altered or is not a report this tool produced.\n');
  process.exit(1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
