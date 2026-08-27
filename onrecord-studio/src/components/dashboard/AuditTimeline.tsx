import { useAppStore } from '../../store/useAppStore';

export default function AuditTimeline() {
  const { syncLogs } = useAppStore();

  // BUG(audit): Event matching is currently case-sensitive while ingestion
  // messages are not normalized; some completed events may never enter this log.
  // TODO(audit): Parse typed telemetry instead of inferring timeline semantics
  // from display strings.

  // Extract only the operational change notices or warnings from terminal streams
  const timelineEvents = syncLogs
    .filter(log => log.includes('COMPLETED') || log.includes('Warning'))
    .slice(0, 5);

  return (
    <div className="p-4 bg-slate-950/30 border border-slate-900 rounded-xl space-y-4 font-sans select-none">
      <div className="text-[10px] font-bold text-slate-500 uppercase tracking-wider">
        Case Audit Log Matrix
      </div>
      <div className="relative border-l border-slate-800 ml-2 pl-4 space-y-4">
        {timelineEvents.length === 0 ? (
          <p className="text-xs text-slate-600 italic pl-1">Awaiting data injection to sequence timeline modifications...</p>
        ) : (
          timelineEvents.map((event, idx) => {
            const isWarning = event.includes('Warning');
            return (
              <div key={idx} className="relative group">
                {/* Node connector ring anchor */}
                <div className={`absolute -left-[21px] top-1 h-2.5 w-2.5 rounded-full border bg-slate-950 ${
                  isWarning ? 'border-amber-500 shadow-sm shadow-amber-500/20' : 'border-indigo-500'
                }`} />
                <div className="space-y-0.5">
                  <div className="text-[11px] font-medium text-slate-300 leading-tight">
                    {isWarning ? 'Compliance Boundary Deviation Detected' : 'Operational Synchronization Finalized'}
                  </div>
                  <div className="text-[10px] font-mono text-slate-500 truncate max-w-md">
                    {event.substring(event.indexOf(']') + 2)}
                  </div>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
