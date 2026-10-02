import { useMemo } from 'react';
import { invokeCommand } from '../ipc';
import { useAppStore } from '../store/useAppStore';
import { useGatedLoad, type LoadState } from './useGatedLoad';
import type { CaseScope } from '../types';

export interface ActiveCaseScope {
  caseId: string;
  findingIds: Set<string>;
  requestKeys: Set<string>;
  scope: CaseScope;
}

export const requestKey = (portalKey: string, requestId: number) => `${portalKey}\u0000${requestId}`;

/**
 * The active case's coverage when views are scoped to it; `null` when no case is active or
 * scoping is off. A scope still loading or failed is reported as such, so a view never filters to
 * an empty set it has not measured.
 */
export function useCaseScope(): { status: 'off' } | { status: 'pending' } | { status: 'failed'; message: string } | { status: 'ready'; value: ActiveCaseScope } {
  const activeCaseId = useAppStore((state) => state.activeCaseId);
  const scopeToCase = useAppStore((state) => state.scopeToCase);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const enabled = activeCaseId !== null && scopeToCase;
  const load: LoadState<CaseScope | null> = useGatedLoad(
    `${enabled ? activeCaseId : ''}:${findingsRefreshRequests}`,
    () => enabled ? invokeCommand('get_case_scope', { caseId: activeCaseId! }) : Promise.resolve(null),
  );
  return useMemo(() => {
    if (!enabled) return { status: 'off' as const };
    if (load.status === 'pending' || (load.status === 'ready' && load.data === null)) return { status: 'pending' as const };
    if (load.status === 'failed') return { status: 'failed' as const, message: load.message };
    const scope = load.data!;
    return {
      status: 'ready' as const,
      value: {
        caseId: scope.caseId,
        findingIds: new Set(scope.findingIds),
        requestKeys: new Set(scope.requests.map((request) => requestKey(request.portalKey, request.requestId))),
        scope,
      },
    };
  }, [enabled, load]);
}
