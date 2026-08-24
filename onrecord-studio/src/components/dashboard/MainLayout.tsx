import React from 'react';
import { useAppStore } from '../../store/useAppStore';
import MetricsGrid from './MetricsGrid';
import SearchFilter from '../filters/SearchFilter';
import { cn } from '../../utils/cn';

export default function MainLayout() {
  const { 
    syncLogs, 
    isSyncing, 
    filteredResults, 
    searchTerm 
  } = useAppStore();

  return (
    <div className="flex-1 flex flex-col min-w-0 overflow-hidden bg-slate-950">
      {/* 1. ADVANCED FULL-TEXT FILTER ENGINE LAYER */}
      <SearchFilter />

      {/* 2. LIVE METRICS COUNTERS OVERVIEW BAR */}
      <MetricsGrid />

      {/* 3. MULTI-PANEL ANALYTICAL HUB GRID */}
      <div className="flex-1 grid grid-cols-1 lg:grid-cols-3 gap-6 p-6 min-h-0 overflow-hidden">
        
        {/* LEFT/CENTER DOUBLE-PANEL: DISCOVERY DISPATCH FEED */}
        <div className="lg:col-span-2 flex flex-col border border-slate-800/80 rounded-xl bg-slate-900/10 backdrop-blur-md overflow-hidden">
          <div className="h-10 border-b border-slate-800/80 px-4 flex items-center justify-between bg-slate-900/30 select-none">
            <span className="text-[11px] font-bold text-slate-400 uppercase tracking-wider">
              Discovery Workbench
            </span>
            {searchTerm && (
              <span className="text-[10px] font-mono text-indigo-400 bg-indigo-500/10 border border-indigo-500/20 px-1.5 py-0.5 rounded">
                {filteredResults.length} Documents Matched
              </span>
            )}
          </div>
          
          <div className="flex-1 p-6 overflow-y-auto flex flex-col justify-center items-center text-center">
            {searchTerm ? (
              <div className="space-y-2 max-w-sm">
                <p className="text-xs font-semibold text-slate-300">
                  FTS5 Search Filter Active
                </p>
                <p className="text-[11px] text-slate-500 leading-relaxed">
                  Your search for <span className="text-indigo-400 font-mono">"{searchTerm}"</span> returned matching records from the SQLite layer. Select a document or audit record row below to map deep compliance trends.
                </p>
              </div>
            ) : (
              <div className="space-y-3 max-w-sm">
                <div className="h-8 w-8 mx-auto rounded-full bg-slate-900 flex items-center justify-center border border-slate-800 text-slate-600 font-mono text-xs">
                  i
                </div>
                <p className="text-xs font-semibold text-slate-400">
                  No Active Case Selected
                </p>
                <p className="text-[11px] text-slate-600 leading-relaxed">
                  Initialize a crawl pipeline on a target municipal engine or execute boolean search filters to load case profiles.
                </p>
              </div>
            )}
          </div>
        </div>

        {/* RIGHT PANEL: LIVE THREAD PROCESSING STREAM TERMINAL */}
        <div className="flex flex-col border border-slate-800/80 rounded-xl bg-slate-950/80 overflow-hidden shadow-inner">
          <div className="h-10 border-b border-slate-800/80 px-4 flex items-center justify-between bg-slate-900/20 select-none">
            <span className="text-[11px] font-bold text-slate-400 uppercase tracking-wider">
              Telemetry Terminal
            </span>
            {isSyncing && (
              <div className="flex items-center space-x-1.5">
                <div className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-ping" />
                <span className="text-[10px] font-mono text-emerald-400 uppercase tracking-wide font-bold">
                  Streaming
                </span>
              </div>
            )}
          </div>
          
          <div className="flex-1 p-4 font-mono text-[10px] leading-relaxed text-slate-300 overflow-y-auto space-y-2 selection:bg-indigo-500/30 selection:text-indigo-200">
            {syncLogs.length === 0 ? (
              <div className="text-slate-600 italic px-2 pt-1 select-none">
                Pipeline execution logs quiet. Ready to sync background workers...
              </div>
            ) : (
              syncLogs.map((log, idx) => {
                const isError = log.includes('FAILED') || log.includes('ERROR') || log.includes('❌');
                const isSuccess = log.includes('COMPLETED') || log.includes('SUCCESS') || log.includes('🎉');
                
                return (
                  <div 
                    key={idx} 
                    className={cn(
                      "border-l-2 pl-3 py-0.5 tracking-tight transition-all",
                      isError ? "border-amber-500/70 bg-amber-500/[0.02] text-amber-300" :
                      isSuccess ? "border-emerald-500/70 bg-emerald-500/[0.02] text-emerald-400" : 
                      "border-indigo-500/20 text-slate-400"
                    )}
                  >
                    {log}
                  </div>
                );
              })
            )}
          </div>
        </div>

      </div>
    </div>
  );
}
