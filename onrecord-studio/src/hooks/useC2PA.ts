import { useCallback, useRef, useState } from 'react';
import type { DocumentProvenanceVerification } from '../types';
import { invokeCommand } from '../ipc';

export function useC2PA() {
  const [activeManifest, setActiveManifest] = useState<DocumentProvenanceVerification | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Identity of the most recent verification asked for. A check started for document A and still
  // in flight when B is selected resolves under B, and publishing its result would attribute A's
  // provenance finding to B: a false statement about evidence, not a stale render.
  const latestRequestRef = useRef('');

  const verifyAssetProvenance = useCallback(async (documentId: number, portalKey: string) => {
    // FEATURE(provenance): Expose signer identity, assertion history, and
    // verification timestamps as a first-class evidence panel.
    const requestedFor = `${documentId}:${portalKey}`;
    latestRequestRef.current = requestedFor;
    setIsValidating(true);
    setError(null);
    try {
      const result = await invokeCommand('verify_document_provenance', {
        documentId,
        portalKey,
      });
      // Superseded. `null` here means "this answer was discarded", never "the check failed";
      // no state is touched, so the newer request's result stands.
      if (latestRequestRef.current !== requestedFor) return null;
      setActiveManifest(result);
      return result;
    } catch (reason) {
      if (latestRequestRef.current !== requestedFor) return null;
      setActiveManifest(null);
      setError(String(reason));
      return null;
    } finally {
      if (latestRequestRef.current === requestedFor) setIsValidating(false);
    }
  }, []);

  return { verifyAssetProvenance, activeManifest, isValidating, error };
}
