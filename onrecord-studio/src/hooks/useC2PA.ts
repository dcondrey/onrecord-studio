import { useCallback, useState } from 'react';
import type { DocumentProvenanceVerification } from '../types';
import { invokeCommand } from '../ipc';
import { useLatestRequestGate } from './useLatestRequestGate';

export function useC2PA() {
  const [activeManifest, setActiveManifest] = useState<DocumentProvenanceVerification | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestGate = useLatestRequestGate();

  const verifyAssetProvenance = useCallback(async (documentId: number, portalKey: string) => {
    // The document viewer pairs this current check with a first-class append-only history panel
    // containing signer identity, assertion counts, and exact verification timestamps.
    const request = requestGate.begin();
    setIsValidating(true);
    setError(null);
    try {
      const result = await invokeCommand('verify_document_provenance', {
        documentId,
        portalKey,
      });
      // Superseded. `null` here means "this answer was discarded", never "the check failed";
      // no state is touched, so the newer request's result stands.
      if (!requestGate.isCurrent(request)) return null;
      setActiveManifest(result);
      return result;
    } catch (reason) {
      if (!requestGate.isCurrent(request)) return null;
      setActiveManifest(null);
      setError(String(reason));
      return null;
    } finally {
      if (requestGate.isCurrent(request)) setIsValidating(false);
    }
  }, [requestGate]);

  return { verifyAssetProvenance, activeManifest, isValidating, error };
}
