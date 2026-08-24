import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';

export default function SearchFilter() {
  const { 
    searchTerm, 
    selectedAgencyId, 
    selectedStatus, 
    setSearchFilters, 
    setFilteredResults,
    isSyncing 
  } = useAppStore();

  const [inputVal, setInputVal] = useState(searchTerm);
  const [isSearching, setIsSearching] = useState(false);

  // Synchronize internal layout input if global store state resets or shifts
  useEffect(() => {
    setInputVal(searchTerm);
  }, [searchTerm]);

  /**
   * Debounces the search request and executes the Tauri IPC full-text search command.
   */
  useEffect(() => {
    if (inputVal.trim() === '') {
      setFilteredResults([]);
      return;
    }

    const delayDebounceFn = setTimeout(async () => {
      try {
        setIsSearching(true);
        
        // Invoke native Rust FTS5 scanning engine across indexed text layers
        const matchingIds: number[] = await invoke('query_audit_records', {
          searchTerm: inputVal
        });
        
        setFilteredResults(matchingIds);
      } catch (err) {
        console.error("FTS5 Query execution breakdown:", err);
      } finally {
        setIsSearching(false);
      }
    }, 350); // 350ms debounce window to limit excessive disk-read stress

    return () => clearTimeout(delayDebounceFn);
  }, [inputVal, setFilteredResults]);

  const handleClearFilters = () => {
    setInputVal('');
    setSearchFilters('', null, null);
    setFilteredResults([]);
  };

  return (
    <div className="px-6 py-4 border-b border-slate-800/80 bg-slate-900/10 backdrop-blur-md space-y-3 select-none">
      <div className="flex items-center space-x-3">
        {/* KEYWORD INPUT WRAPPER */}
        <div className="relative flex-1">
          <input
            type="text"
            value={inputVal}
            onChange={(e) => setInputVal(e.target.value)}
            disabled={isSyncing}
            placeholder={isSyncing ? "Search deactivated during active crawl..." : 'Search document layers (e.g., "sweep" AND "police")...'}
            className={cn(
              "w-full bg-slate-950/60 text-slate-100 text-xs pl-9 pr-4 py-2.5 rounded-lg border border-slate-800 transition-all focus:outline-none focus:border-indigo-500/80 focus:ring-1 focus:ring-indigo-500/30 font-medium placeholder-slate-500 shadow-inner",
              isSyncing && "opacity-50 cursor-not-allowed placeholder-slate-700"
            )}
          />
          {/* LOUPE ICON & LIVE THREAD STATUS INDICATOR */}
          <div className="absolute left-3 top-1/2 -translate-y-1/2 flex items-center justify-center">
            {isSearching ? (
              <div className="h-3.5 w-3.5 rounded-full border-2 border-indigo-500/20 border-t-indigo-400 animate-spin" />
            ) : (
              <svg className="h-3.5 w-3.5 text-slate-500" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2.5}>
                <path strokeLinecap="round" strokeLinejoin="round" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
              </svg>
            )}
          </div>
        </div>

        {/* RESET CONTROL TRIGGER */}
        {(inputVal || selectedAgencyId || selectedStatus) && (
          <button
            onClick={handleClearFilters}
            className="px-3 py-2 text-[11px] font-semibold text-slate-400 hover:text-slate-200 border border-slate-800 hover:bg-slate-800/40 rounded-lg transition-all"
          >
            Clear Filters
          </button>
        )}
      </div>

      {/* METRIC BADGES MATCH COUNTER */}
      {inputVal && !isSearching && (
        <div className="flex items-center space-x-2 font-mono text-[10px] text-slate-400">
          <span>Active Query Status:</span>
          <span className="text-indigo-400 bg-indigo-500/10 border border-indigo-500/20 px-1.5 py-0.5 rounded">
            FTS5 Match Success
          </span>
        </div>
      )}
    </div>
  );
}
