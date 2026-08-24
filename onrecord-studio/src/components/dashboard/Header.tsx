import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';

export default function Header() {
  const { 
    activePortal, 
    isSyncing, 
    setIsSyncing, 
    appendSyncLog,
    resetTelemetryMetrics
  } = useAppStore();

  /**
   * Dispatches the asynchronous IPC command to activate background scraping threads.
   */
  const handleStartExtraction = async () => {
    if (isSyncing) return;

    try {
      setIsSyncing(true);
      resetTelemetryMetrics();
      appendSyncLog(`INITIALIZING DISCOVERY TIMELINE: Targeting https://${activePortal}.nextrequest.com`);

      // Invoke the native Rust command handler to begin the ingestion queue
      const statusMessage: string = await invoke('start_subdomain_sync', {
        subdomain: activePortal,
        startPage: 1,
        perPage: 50
      });

      appendSyncLog(`SUCCESS: ${statusMessage}`);
    } catch (err) {
      appendSyncLog(`CRITICAL ERROR: Failed to hand off queue payload: ${err}`);
      setIsSyncing(false);
    }
  };

  /**
   * Helper to format active subdomain context into human-readable municipal titles.
   */
  const getFriendlyPortalName = () => {
    switch (activePortal) {
      case 'sandiego':
        return 'City of San Diego';
      case 'sandiegohousingcommission':
        return 'San Diego Housing Commission';
      default:
        return activePortal;
    }
  };

  return (
    <header className="h-14 border-b border-slate-800/80 flex items-center justify-between px-6 bg-slate-900/10 backdrop-blur-md select-none">
      {/* LEFT AREA: WORKSPACE IDENTITY METADATA */}
      <div className="flex flex-col">
        <div className="text-xs font-semibold text-slate-200">
          {getFriendlyPortalName()}
        </div>
        <div className="text-[10px] font-mono text-slate-500 mt-0.5 tracking-tight">
          Target Network Layer: <span className="text-indigo-400/80">{activePortal}://</span>
        </div>
      </div>

      {/* RIGHT AREA: ACTIONABLE INTERLOCK PIPELINE TRIGGERS */}
      <div className="flex items-center space-x-3">
        <button
          onClick={handleStartExtraction}
          disabled={isSyncing}
          className={cn(
            "text-xs font-semibold px-4 py-2 rounded-lg transition-all duration-200 shadow-sm border focus:outline-none",
            isSyncing 
              ? "bg-slate-900 border-slate-800 text-slate-500 cursor-not-allowed" 
              : "bg-indigo-600 border-indigo-500 text-white hover:bg-indigo-500 hover:scale-[1.02] active:scale-[0.98] cursor-pointer shadow-indigo-600/10"
          )}
        >
          {isSyncing ? (
            <div className="flex items-center space-x-2">
              <div className="h-3 w-3 rounded-full border-2 border-slate-600 border-t-slate-400 animate-spin" />
              <span>Ingestion Active</span>
            </div>
          ) : (
            "Sync Public Ledger"
          )}
        </button>
      </div>
    </header>
  );
}
