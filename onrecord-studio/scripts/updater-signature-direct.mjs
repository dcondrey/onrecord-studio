import { createHash, createPublicKey, verify } from 'node:crypto';

const ED25519_SPKI_PREFIX = Buffer.from('302a300506032b6570032100', 'hex');

function decodeText(base64, label) {
  const text = Buffer.from(base64.trim(), 'base64').toString('utf8');
  const lines = text.split('\n');
  if (!lines[0]?.startsWith('untrusted comment:') || !lines[1]) throw new Error(`${label} is not a minisign file.`);
  return lines;
}

/** Parses the base64 minisign public key that tauri.direct.conf.json carries. */
export function parsePublicKey(pubkey) {
  const lines = decodeText(pubkey, 'The updater public key');
  if (!lines[0].startsWith('untrusted comment: minisign public key')) throw new Error('The updater key is not a public key.');
  const raw = Buffer.from(lines[1], 'base64');
  if (raw.length !== 42 || raw.subarray(0, 2).toString() !== 'Ed') throw new Error('The updater public key is malformed.');
  return { keyId: raw.subarray(2, 10), key: raw.subarray(10) };
}

/** Verifies a `.sig` file (base64 of a minisig) over `data`, including the trusted comment. */
export function verifyUpdaterSignature({ pubkey, signature, data }) {
  const { keyId, key } = parsePublicKey(pubkey);
  const lines = decodeText(signature, 'The updater signature');
  const raw = Buffer.from(lines[1], 'base64');
  if (raw.length !== 74) throw new Error('The updater signature is malformed.');
  const algorithm = raw.subarray(0, 2).toString();
  if (!keyId.equals(raw.subarray(2, 10))) throw new Error('The signature was made by a different key than the configured public key.');
  const publicKey = createPublicKey({ key: Buffer.concat([ED25519_SPKI_PREFIX, key]), format: 'der', type: 'spki' });
  const signed = algorithm === 'ED' ? createHash('blake2b512').update(data).digest() : data;
  if (algorithm !== 'ED' && algorithm !== 'Ed') throw new Error('Unknown minisign algorithm.');
  const body = raw.subarray(10);
  if (!verify(null, signed, publicKey, body)) throw new Error('The signature does not verify against the configured public key.');
  const comment = lines[2]?.startsWith('trusted comment: ') ? lines[2].slice('trusted comment: '.length) : null;
  const global = lines[3] ? Buffer.from(lines[3], 'base64') : null;
  if (comment === null || !global || !verify(null, Buffer.concat([body, Buffer.from(comment)]), publicKey, global)) {
    throw new Error('The signature trusted comment does not verify.');
  }
  return true;
}
