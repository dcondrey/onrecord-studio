import React from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';

export default function Sidebar() {
  const { 
    activePortal, 
    setActivePortal, 
    isSyncing, 
    metrics 
  } = useAppStore();

  const portals = [
    { id: 'sandiego', name: 'City of San Diego', shortcut: '⌘1' },
    { id: 'sandiegohousingcommission', name: 'SD Housing Commission', shortcut: '⌘2' }
  ];

  return (
    <aside className="w-64 h-full border-r border-slate-800 bg-slate-900/40 backdrop-blur-xl p-4 flex flex-col justify-between select-none">
      <div className="space-y-6">
        {/* APP TITLE PLATFORM BRANDING */}
        <div className="flex items-center justify-between px-2 py-1">
          <div className="flex items-center space-x-2.5">
            <div className={cn(
              "h-2.5 w-2.5 rounded-full shadow-lg transition-all duration-300",
              isSyncing ? "bg-emerald-500 animate-pulse ring-4 ring-emerald-500/20" : "bg-indigo-500"
            )} />
            <span className="text-sm font-bold tracking-tight bg-gradient-to-r from-slate-100 to-slate-300 bg-clip-text text-transparent">
              onrecord-studio
            </span>
          </div>
          <span className="text-[10px] font-mono bg-slate-800 text-slate-400 px-1.5 py-0.5 rounded-md border border-slate-700/60">
            PRO
          </span>
        </div>

        {/* WORKSPACE PORTALS MONITOR LIST */}
        <div className="space-y-1">
          <label className="text-[10px] font-bold text-slate-500 uppercase tracking-wider block px-2 mb-2">
            Target Municipal Engines
          </label>
          {portals.map((portal) => {
            const isSelected = activePortal === portal.id;
            return (
              <button
                key={portal.id}
                onClick={() => !isSyncing && setActivePortal(portal.id)}
                disabled={isSyncing}
                className={cn(
                  "w-full flex items-center justify-between px-3 py-2 rounded-lg text-xs font-medium transition-all duration-150 relative border",
                  isSelected
                    ? "bg-indigo-600/10 border-indigo-500/30 text-indigo-400 font-semibold shadow-inner"
                    : "border-transparent text-slate-400 hover:bg-slate-800/60 hover:text-slate-200 cursor-pointer",
                  isSyncing && "opacity-60 cursor-not-allowed hover:bg-transparent"
                )}
              >
                <span className="truncate">{portal.name}</span>
                <span className="text-[9px] font-mono text-slate-600 bg-slate-950/40 px-1 py-0.5 rounded border border-slate-800">
                  {portal.shortcut}
                </span>
              </button>
            );
          })}
        </div>

        {/* PIPELINE LIVE STATUS PANEL */}
        <div className="px-2 pt-2 border-t border-slate-800/60 space-y-3">
          <label className="text-[10px] font-bold text-slate-500 uppercase tracking-wider block">
            Pipeline Analytics
          </label>
          <div className="space-y-2 font-mono text-[11px] text-slate-400">
            <div className="flex justify-between items-center">
              <span className="text-slate-500">Sync Worker:</span>
              <span className={cn(
                "font-semibold uppercase tracking-wide text-[10px]",
                isSyncing ? "text-emerald-400" : "text-slate-500"
              )}>
                {isSyncing ? "Active" : "Standby"}
              </span>
            </div>
            <div className="flex justify-between items-center">
              <span className="text-slate-500">Errors Flagged:</span>
              <span className={cn(
                metrics.anomaliesFound > 0 ? "text-amber-500 font-bold" : "text-slate-400"
              )}>
                {metrics.anomaliesFound}
              </span>
            </div>
          </div>
        </div>
      </div>

      {/* FOOTER ENVIRONMENT BUILD METRIC */}
      <div className="border-t border-slate-800/60 pt-3 flex flex-col space-y-1 text-[10px] font-mono text-slate-500 px-2">
        <div className="flex justify-between">
          <span>Target Architecture:</span>
          <span className="text-slate-400">macOS (AArch64)</span>
        </div>
        <div className="flex justify-between">
          <span>Backend Core:</span>
          <span className="text-indigo-400 font-semibold">Rust v1.80+</span>
        </div>
      </div>
    </aside>
  );
}
