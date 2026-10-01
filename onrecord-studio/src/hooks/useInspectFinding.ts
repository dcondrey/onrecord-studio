import { useCallback } from 'react';
import { useAppStore } from '../store/useAppStore';
import type { Finding } from '../types';

/** Opens the evidence view scoped to a finding's subject. Shared so every surface that lists findings lands in the same place. */
export function useInspectFinding() {
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  return useCallback(
    (finding: Finding) => {
      const { portalKey, agencyId } = finding.subject;
      if (portalKey) setActivePortal(portalKey);
      setSearchFilters('', agencyId ?? null, null, null);
      setWorkspaceView('evidence');
    },
    [setActivePortal, setSearchFilters, setWorkspaceView],
  );
}
