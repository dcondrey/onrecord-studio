import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../ipc';
import { useAppStore } from '../store/useAppStore';
import { useLatestRequestGate } from './useLatestRequestGate';
import type { Finding, FindingsFilter } from '../types';

export type FindingsLoad =
  | { status: 'loading'; findings: Finding[] }
  | { status: 'ready'; findings: Finding[] }
  | { status: 'failed'; findings: Finding[]; message: string };

/**
 * Loads `list_findings` once per findings refresh and once per change of `filter` or `limit`.
 * A result that resolves after a newer load began, or after unmount, is discarded: the rows
 * would describe the corpus as it was, not as it is. The previous rows stay in place while a
 * reload runs so a strip does not flash empty; `status` says whether they are current.
 *
 * `filter` is compared by value through its JSON form, so callers may pass a fresh literal.
 */
export function useFindings(filter: FindingsFilter | null, limit: number): FindingsLoad & { reload: () => void } {
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const gate = useLatestRequestGate();
  const [state, setState] = useState<FindingsLoad>({ status: 'loading', findings: [] });
  const filterKey = JSON.stringify(filter);

  const reload = useCallback(() => {
    const request = gate.begin();
    setState((current) => ({ status: 'loading', findings: current.findings }));
    invokeCommand('list_findings', { filter: JSON.parse(filterKey) as FindingsFilter | null, limit })
      .then((rows) => {
        if (!gate.isCurrent(request)) return;
        setState({ status: 'ready', findings: rows });
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        setState((current) => ({
          status: 'failed',
          findings: current.findings,
          message: formatNativeError(reason),
        }));
      });
  }, [filterKey, gate, limit]);

  useEffect(() => {
    reload();
    return gate.invalidate;
  }, [findingsRefreshRequests, gate, reload]);

  return { ...state, reload };
}
