import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import StatusDot from '../ui/statusDot';

export default function ExtractionTerminal() {
  const syncLogs = useAppStore((state) => state.syncLogs);
  const isSyncing = useAppStore((state) => state.isSyncing);
  const isOpen = useAppStore((state) => state.isExtractionTerminalOpen);
  const setOpen = useAppStore((state) => state.setTerminalVisibility);
  const clear = useAppStore((state) => state.resetTelemetryMetrics);

  return (
    <section className="flex h-full flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-950 font-mono text-label leading-relaxed shadow-lg" aria-label="Extraction event stream">
      <div className="h-9 border-b border-slate-800 px-4 bg-slate-900/30 flex items-center justify-between select-none">
        <button type="button" className="focus-ring flex items-center gap-2 rounded text-caption font-bold uppercase tracking-wider text-slate-400" aria-expanded={isOpen} onClick={() => setOpen(!isOpen)}>
          Extraction event stream
          <span aria-hidden="true">{isOpen ? '−' : '+'}</span>
        </button>
        <div className="flex items-center gap-3">
          {syncLogs.length > 0 && <button type="button" onClick={clear} className="focus-ring rounded px-1.5 py-1 text-caption text-slate-400 hover:text-slate-300">Clear session telemetry</button>}
          <StatusDot state={isSyncing ? 'active' : 'idle'} label={isSyncing ? 'Synchronization active' : 'Synchronization idle'} className={isSyncing ? 'status-pulse rounded-full' : undefined} />
        </div>
      </div>
      {isOpen && (
        <div className="min-h-0 flex-1 overflow-y-auto p-3 text-slate-400" aria-live="polite">
          {syncLogs.length === 0
            ? <div className="p-2 text-slate-400">No ingestion activity has been recorded in this session.</div>
            : syncLogs.map((log, index) => (
              <div key={`${index}-${log}`} className={cn('flex items-start gap-1.5 border-l py-1 pl-2.5', isErrorLog(log) ? 'border-rose-500/50 text-rose-300/80' : 'border-slate-800 text-slate-400')}>
                {isErrorLog(log) && <StatusDot state="error" label="Error" className="mt-1" />}
                <span className="min-w-0 flex-1">{log}</span>
              </div>
            ))}
        </div>
      )}
    </section>
  );
}

function isErrorLog(value: string) {
  const normalized = value.toLocaleLowerCase();
  return normalized.includes('failed') || normalized.includes('error') || normalized.includes('warning');
}
