/**
 * Cryptographic utility layer for verification and decentralized identifier validation.
 */

/**
 * Verifies that a base64 or hex hash string matches standard SHA-256 formatting parameters.
 */
export function isValidContentHash(hash: string): boolean {
  if (!hash) return false;
  // Support both raw hex (64 chars) and standard Base64 string formats
  const hexRegex = /^[a-fA-F0-9]{64}$/;
  const base64Regex = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
  return hexRegex.test(hash) || (hash.length >= 44 && base64Regex.test(hash));
}

/**
 * Parses and extracts key context from a did:web string layout.
 * Example target format: "did:web:writerslogic.com#key-1"
 */
export interface ParsedDid {
  method: string;
  domain: string;
  fragment: string | null;
}

export function parseDidWeb(did: string): ParsedDid | null {
  if (!did || !did.startsWith("did:web:")) return null;

  try {
    const withoutPrefix = did.substring(8);
    const [domainAndPath, fragment] = withoutPrefix.split("#");
    
    // Convert did:web colon pathing back to standard web domain structures
    const domain = domainAndPath.replace(/:/g, "/");

    return {
      method: "web",
      domain,
      fragment: fragment || null
    };
  } catch {
    return null;
  }
}