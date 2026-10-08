import { useMemo } from 'react';
import { Select } from '../ui/Field';
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
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const agencyIndex = useMemo(() => {
    const all = new Map<string, { value: string; portalKey: string; agencyId: number; name: string }>();
    for (const profile of profiles) {
      const value = JSON.stringify([profile.portalSubdomain, profile.agencyId]);
      all.set(value, { value, portalKey: profile.portalSubdomain, agencyId: profile.agencyId, name: profile.agencyName });
    }
    return [...all.values()].sort((left, right) => left.name.localeCompare(right.name) || left.portalKey.localeCompare(right.portalKey));
  }, [profiles]);
  const agencies = activePortalKey ? agencyIndex.filter((agency) => agency.portalKey === activePortalKey) : agencyIndex;

  return (
    <Select
        aria-label="Department"
        value={selectedAgencyId !== null && activePortalKey ? JSON.stringify([activePortalKey, selectedAgencyId]) : ''}
        onChange={(event) => {
          const agency = agencyIndex.find((candidate) => candidate.value === event.target.value);
          if (agency && activePortalKey !== agency.portalKey) setActivePortal(agency.portalKey);
          setSearchFilters(searchTerm, agency?.agencyId ?? null, selectedStatus, selectedPolicyDeviation);
        }}
        className="focus-ring w-48 truncate"
      >
        <option value="">All departments</option>
        {agencies.map((agency) => (
          <option key={agency.value} value={agency.value}>{agency.name}{activePortalKey ? '' : ` · ${agency.portalKey}`}</option>
        ))}
      </Select>
  );
}
