export function isValidContentHash(hash: string): boolean {
  // LIMIT(integrity): This compatibility predicate accepts SHA-256 in either hex or base64. Callers
  // that persist a digest also retain the field/schema that specifies its representation.
  return isSha256Hex(hash) || isSha256Base64(hash);
}

export interface ParsedDid {
  method: string;
  domain: string;
  fragment: string | null;
}

/**
 * SECURITY(security): A validation boundary, not a convenience parser. DIDs arrive in provenance
 * metadata we do not control, and a resolver turns the result into an HTTPS request. Rejections
 * are pinned in src/tests/crypto.test.ts; add a case there before relaxing one.
 *
 * LIMIT(security): Success proves shape only, never that the DID resolves or is controlled by the
 * named domain.
 */
export function parseDidWeb(did: string): ParsedDid | null {
  if (!did.startsWith('did:web:') || containsControlCharacter(did)) return null;
  const methodSpecificIdentifier = did.slice('did:web:'.length);
  const fragmentSeparator = methodSpecificIdentifier.indexOf('#');
  if (fragmentSeparator !== -1 && methodSpecificIdentifier.indexOf('#', fragmentSeparator + 1) !== -1) return null;
  const encodedLocation = fragmentSeparator === -1
    ? methodSpecificIdentifier
    : methodSpecificIdentifier.slice(0, fragmentSeparator);
  const fragment = fragmentSeparator === -1
    ? null
    : methodSpecificIdentifier.slice(fragmentSeparator + 1);
  if (encodedLocation.length === 0 || fragment === '') return null;
  try {
    // Parse the DID as the HTTPS URL it would resolve to so credentials, traversal, and encoded
    // path separators are rejected before a future resolver performs network I/O.
    const segments = encodedLocation.split(':').map((segment) => decodeURIComponent(segment));
    if (segments.some((segment) => segment.length === 0 || segment === '.' || segment === '..' || segment.includes('/'))) return null;
    const authority = segments[0];
    const path = segments.slice(1).map((segment) => encodeURIComponent(segment)).join('/');
    const url = new URL(`https://${authority}${path.length > 0 ? `/${path}` : ''}`);
    if (url.username !== '' || url.password !== '' || url.hostname === '') return null;
    return {
      method: 'web',
      domain: `${url.host}${url.pathname === '/' ? '' : url.pathname}`,
      fragment,
    };
  } catch {
    return null;
  }
}

export function isSha256Hex(value: string) {
  return value.length === 64 && Array.from(value).every((character) => {
    const code = character.toLocaleLowerCase().charCodeAt(0);
    return (code >= 48 && code <= 57) || (code >= 97 && code <= 102);
  });
}

function isSha256Base64(value: string) {
  return value.length === 44
    && value[43] === '='
    && Array.from(value.slice(0, 43)).every(isBase64Character);
}

function isBase64Character(character: string) {
  const code = character.charCodeAt(0);
  return (code >= 48 && code <= 57)
    || (code >= 65 && code <= 90)
    || (code >= 97 && code <= 122)
    || character === '+'
    || character === '/';
}

function containsControlCharacter(value: string) {
  return Array.from(value).some((character) => {
    const code = character.charCodeAt(0);
    return code <= 31 || code === 127;
  });
}
