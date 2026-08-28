import { useMemo } from 'react';
import { useAppStore } from '../../store/useAppStore';

export default function AgencySelector() {
  // One agency is the supported native filter. The active portal scopes otherwise-duplicate
  // agency identifiers before they are offered to the user.
  const profiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const searchTerm = useAppStore((state) => state.searchTerm);
  const selectedAgencyId = useAppStore((state) => state.selectedAgencyId);
  const selectedStatus = useAppStore((state) => state.selectedStatus);
  const selectedPolicyDeviation = useAppStore((state) => state.selectedPolicyDeviation);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const agencyIndex = useMemo(() => {
    const all = new Map<number, string>();
    const byPortal = new Map<string, Map<number, string>>();
    for (const profile of profiles) {
      all.set(profile.agencyId, profile.agencyName);
      const scoped = byPortal.get(profile.portalSubdomain) ?? new Map<number, string>();
      scoped.set(profile.agencyId, profile.agencyName);
      byPortal.set(profile.portalSubdomain, scoped);
    }
    const sorted = (values: Map<number, string>) => [...values.entries()].sort((left, right) => left[1].localeCompare(right[1]));
    return {
      all: sorted(all),
      byPortal: new Map([...byPortal].map(([portalKey, values]) => [portalKey, sorted(values)])),
    };
  }, [profiles]);
  const agencies = activePortalKey ? agencyIndex.byPortal.get(activePortalKey) ?? [] : agencyIndex.all;

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
