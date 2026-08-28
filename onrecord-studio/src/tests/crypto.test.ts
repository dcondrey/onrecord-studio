import { describe, expect, it } from 'vitest';
import { isValidContentHash, parseDidWeb } from '../utils/crypto';

// These tests back the SECURITY(security) comment in src/utils/crypto.ts. `parseDidWeb` exists to
// reject hostile identifiers BEFORE a resolver turns one into a network request, so every case
// below is a rejection that must keep working, not a formatting preference.

describe('parseDidWeb rejection cases', () => {
  it('accepts the shapes the DID spec defines', () => {
    expect(parseDidWeb('did:web:example.gov')).toEqual({
      method: 'web',
      domain: 'example.gov',
      fragment: null,
    });
    // Colons become path separators, so a port-looking segment is a PATH, not a port.
    expect(parseDidWeb('did:web:example.gov:agency:records')?.domain).toBe(
      'example.gov/agency/records',
    );
    expect(parseDidWeb('did:web:example.gov#key-1')).toEqual({
      method: 'web',
      domain: 'example.gov',
      fragment: 'key-1',
    });
  });

  it('rejects anything that is not a did:web identifier', () => {
    expect(parseDidWeb('')).toBeNull();
    expect(parseDidWeb('did:key:z6Mk')).toBeNull();
    expect(parseDidWeb('https://example.gov')).toBeNull();
    // Prefix matching is exact: a lookalike method must not slip through.
    expect(parseDidWeb('did:webs:example.gov')?.domain).not.toBe('example.gov');
    expect(parseDidWeb('DID:WEB:example.gov')).toBeNull();
  });

  it('never lets credentials smuggle a different authority past the parser', () => {
    // Percent-encoded ':' and '@' decode into a real userinfo section, which is the case that
    // would hand a resolver `https://user:pass@example.gov`. The username check rejects it.
    expect(parseDidWeb('did:web:user%3Apass%40example.gov')).toBeNull();
    expect(parseDidWeb('did:web:user%40example.gov')).toBeNull();

    // The unencoded forms are NOT rejected, and that is correct: ':' is the DID path separator,
    // so the authority is the literal host `user` and the rest is an escaped path segment. What
    // matters is that neither one resolves to example.gov.
    expect(parseDidWeb('did:web:user:pass@example.gov')?.domain).toBe(
      'user/pass%40example.gov',
    );
    // A bare '@' in a single segment IS userinfo once parsed, so this one is rejected outright.
    expect(parseDidWeb('did:web:user@example.gov')).toBeNull();
  });

  it('rejects path traversal, including percent-encoded traversal', () => {
    expect(parseDidWeb('did:web:example.gov:..:secret')).toBeNull();
    expect(parseDidWeb('did:web:example.gov:.')).toBeNull();
    // %2E%2E decodes to '..', which is why the check runs AFTER decodeURIComponent.
    expect(parseDidWeb('did:web:example.gov:%2E%2E:secret')).toBeNull();
  });

  it('rejects encoded path separators that would forge extra path structure', () => {
    // %2F decodes to '/', which would silently add a path segment the DID never declared.
    expect(parseDidWeb('did:web:example.gov:a%2Fb')).toBeNull();
    expect(parseDidWeb('did:web:example.gov%2Fevil.test')).toBeNull();
  });

  it('rejects empty segments and empty authority', () => {
    expect(parseDidWeb('did:web:')).toBeNull();
    expect(parseDidWeb('did:web:example.gov::records')).toBeNull();
    expect(parseDidWeb('did:web::records')).toBeNull();
  });

  it('rejects control characters that could smuggle headers or split a log line', () => {
    expect(parseDidWeb('did:web:example.gov\r\nHost: evil.test')).toBeNull();
    expect(parseDidWeb('did:web:example.gov\u0000')).toBeNull();
    expect(parseDidWeb('did:web:exam\u007Fple.gov')).toBeNull();
  });

  it('rejects malformed fragments', () => {
    // A trailing '#' with nothing after it is not the same as "no fragment".
    expect(parseDidWeb('did:web:example.gov#')).toBeNull();
    expect(parseDidWeb('did:web:example.gov#a#b')).toBeNull();
  });

  it('rejects percent-encoding that does not decode', () => {
    expect(parseDidWeb('did:web:example.gov:%ZZ')).toBeNull();
    expect(parseDidWeb('did:web:example.gov:%')).toBeNull();
  });
});

describe('isValidContentHash', () => {
  const hex = 'a'.repeat(64);
  const base64 = `${'A'.repeat(43)}=`;

  it('accepts the two digest encodings the app stores', () => {
    expect(isValidContentHash(hex)).toBe(true);
    expect(isValidContentHash(hex.toUpperCase())).toBe(true);
    expect(isValidContentHash(base64)).toBe(true);
  });

  it('rejects wrong-length and wrong-alphabet values', () => {
    expect(isValidContentHash('')).toBe(false);
    expect(isValidContentHash('a'.repeat(63))).toBe(false);
    expect(isValidContentHash('a'.repeat(65))).toBe(false);
    // 'g' is outside hex; the length is right, so only the alphabet check can catch it.
    expect(isValidContentHash(`g${'a'.repeat(63)}`)).toBe(false);
    // Base64 of the right length but missing the required terminal '='.
    expect(isValidContentHash('A'.repeat(44))).toBe(false);
  });

  it('LIMIT(integrity): does not distinguish which algorithm produced the digest', () => {
    // Pinned deliberately. Any 64-char hex string passes, so this function answers "is this
    // shaped like a SHA-256 digest", NOT "is this a SHA-256 digest of the content at hand".
    // The compatibility predicate deliberately accepts both supported SHA-256 encodings.
    expect(isValidContentHash('0'.repeat(64))).toBe(true);
  });
});
