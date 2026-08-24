import React from 'react';
import { useAppStore } from '../../store/useAppStore';

export default function ExtractionTerminal() {
  const { metrics, isSyncing } = useAppStore();

  return (
    <div className="border border-slate-800 rounded-xl bg-slate-950 font-mono text-[10px] leading-relaxed shadow-lg overflow-hidden flex flex-col h-full">
      <div className="h-9 border-b border-slate-800 px-4 bg-slate-900/30 flex items-center justify-between select-none">
        <span className="font-bold text-slate-400 uppercase tracking-wider text-[9px]">Text Core Processor</span>
        {isSyncing && <div className="h-1.5 w-1.5 rounded-full bg-cyan-400 animate-pulse" />}
      </div>
      <div className="p-4 flex-1 overflow-y-auto space-y-1 text-slate-400 select-none">
        <div>[SYSTEM] Initializing Rust core extraction channels...</div>
        <div>[SYSTEM] Local SQLite connection mapping pools online. WAL enabled.</div>
        {metrics.pagesProcessed > 0 && <div className="text-indigo-400">[PARSER] Sync pipeline linked to DuckDB database memory boundaries.</div>}
        {metrics.documentsDownloaded > 0 && <div className="text-cyan-400">[DECOMPRESSOR] Extracted {metrics.documentsDownloaded} file blobs via native PDF text layers.</div>}
        {isSyncing ? (
          <div className="text-emerald-400 animate-pulse">[PARSER] Streaming background thread processes...</div>
        ) : (
          <div className="text-slate-600">[SYSTEM] Extraction core idle. Standby status.</div>
        )}
      </div>
    </div>
  );
}
