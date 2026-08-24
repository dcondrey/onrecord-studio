import React from 'react';
import { useAppStore } from '../../store/useAppStore';

export default function AgencySelector() {
  const { selectedAgencyId, setSearchFilters, searchTerm, selectedStatus } = useAppStore();

  const agencies = [
    { id: 1, name: 'All Departments' },
    { id: 44, name: 'Environmental Services' },
    { id: 102, name: 'Police Department' },
    { id: 215, name: 'Housing Commission Office' }
  ];

  return (
    <div className="flex flex-col space-y-1.5 select-none font-sans">
      <label className="text-[10px] font-bold text-slate-500 uppercase tracking-wider">Department Matrix</label>
      <select
        value={selectedAgencyId || 1}
        onChange={(e) => {
          const val = parseInt(e.target.value);
          setSearchFilters(searchTerm, val === 1 ? null : val, selectedStatus);
        }}
        className="bg-slate-950 text-slate-300 text-xs border border-slate-800 rounded-lg px-2.5 py-1.5 focus:outline-none focus:border-indigo-500 font-medium shadow-sm transition-all"
      >
        {agencies.map((agency) => (
          <option key={agency.id} value={agency.id} className="bg-slate-950 text-slate-300">
            {agency.name}
          </option>
        ))}
      </select>
    </div>
  );
}
