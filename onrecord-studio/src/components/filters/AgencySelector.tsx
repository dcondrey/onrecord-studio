import { useMemo } from 'react';
import { useAppStore } from '../../store/useAppStore';

export default function AgencySelector() {
  // FEATURE(query): Support typeahead and multi-agency selection once the
  // canonical QueryAst can represent an agency set without flattening IDs.
  const profiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const searchTerm = useAppStore((state) => state.searchTerm);
  const selectedAgencyId = useAppStore((state) => state.selectedAgencyId);
  const selectedStatus = useAppStore((state) => state.selectedStatus);
  const selectedPolicyDeviation = useAppStore((state) => state.selectedPolicyDeviation);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  // PERF(query): This derives options from all loaded profiles; introduce a
  // normalized agency index if profile volumes make selector updates costly.
  const agencies = useMemo(() => {
    const unique = new Map<number, string>();
    for (const profile of profiles) {
      if (!activePortalKey || profile.portalSubdomain === activePortalKey) {
        unique.set(profile.agencyId, profile.agencyName);
      }
    }
    return [...unique.entries()].sort((left, right) => left[1].localeCompare(right[1]));
  }, [activePortalKey, profiles]);

  return (
    <label className="flex min-w-0 items-center gap-2">
      <span className="sr-only">Department</span>
      <select
        value={selectedAgencyId ?? ''}
        onChange={(event) => {
          const agencyId = event.target.value ? Number(event.target.value) : null;
          setSearchFilters(searchTerm, agencyId, selectedStatus, selectedPolicyDeviation);
        }}
        className="focus-ring h-10 max-w-52 rounded-xl border border-slate-800 bg-slate-950/65 px-3 text-[10px] text-slate-300 transition focus:border-indigo-400/50"
      >
        <option value="">All departments</option>
        {agencies.map(([agencyId, agencyName]) => (
          <option key={agencyId} value={agencyId}>{agencyName}</option>
        ))}
      </select>
    </label>
  );
}
