import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../ipc';
import type { ResolutionClusterView } from '../types';
import { useLatestRequestGate } from './useLatestRequestGate';

export type ResolutionClustersState =
  | { status: 'idle' }
  | { status: 'loading'; entityId: string }
  /** The id names no entity in the index (a rebuild discards ids). Not an empty cluster. */
  | { status: 'gone'; entityId: string }
  | { status: 'ready'; entityId: string; cluster: ResolutionClusterView }
  | { status: 'failed'; entityId: string; error: string };

/**
 * The entities a reviewer judged to be the same as `entityId`, with the reasons and the
 * `resolutionId` that cites this reading of the ledger. Applying it is the caller's choice; nothing
 * here merges anything. A result for an entity the caller has since moved off is discarded.
 */
export function useResolutionClusters(entityId: string | null): ResolutionClustersState {
  const gate = useLatestRequestGate();
  const [state, setState] = useState<ResolutionClustersState>({ status: 'idle' });

  useEffect(() => {
    if (!entityId) {
      gate.invalidate();
      setState({ status: 'idle' });
      return;
    }
    const request = gate.begin();
    setState({ status: 'loading', entityId });
    invokeCommand('get_resolution_clusters', { entityId })
      .then((cluster) => {
        if (!gate.isCurrent(request)) return;
        setState(cluster ? { status: 'ready', entityId, cluster } : { status: 'gone', entityId });
      })
      .catch((reason) => {
        if (gate.isCurrent(request)) setState({ status: 'failed', entityId, error: formatNativeError(reason) });
      });
  }, [entityId, gate]);

  return state;
}
