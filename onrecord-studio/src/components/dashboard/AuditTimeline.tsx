import { useAppStore } from '../../store/useAppStore';

export default function AuditTimeline() {
  const timelineEvents = useAppStore((state) => state.syncLogs.slice(0, 8));

  return (
    <div className="p-4 bg-slate-950/30 border border-slate-900 rounded-xl space-y-4 font-sans select-none">
      <div className="text-[10px] font-bold text-slate-500 uppercase tracking-wider">
        Recent operational activity
      </div>
      <div className="relative border-l border-slate-800 ml-2 pl-4 space-y-4">
        {timelineEvents.length === 0 ? (
          <p className="pl-1 text-xs italic text-slate-600">No activity has been recorded in this session.</p>
        ) : (
          timelineEvents.map((event, idx) => {
            const isWarning = isErrorLog(event);
            return (
              <div key={idx} className="relative group">
                {/* Node connector ring anchor */}
                <div className={`absolute -left-[21px] top-1 h-2.5 w-2.5 rounded-full border bg-slate-950 ${
                  isWarning ? 'border-amber-500 shadow-sm shadow-amber-500/20' : 'border-indigo-500'
                }`} />
                <div className="space-y-0.5">
                  <div className="max-w-md break-words font-mono text-[10px] text-slate-500">
                    {event}
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

function isErrorLog(value: string) {
  const normalized = value.toLocaleLowerCase();
  return normalized.includes('failed') || normalized.includes('error') || normalized.includes('warning');
}
