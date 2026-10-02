import { useAppStore } from '../../store/useAppStore';

export default function AuditTimeline() {
  const timelineEvents = useAppStore((state) => state.syncLogs.slice(0, 8));

  return (
    <div className="p-4 bg-canvas/40 border border-line rounded-xl space-y-4 font-sans select-none">
      <div className="text-label font-bold text-muted uppercase tracking-wider">
        Recent operational activity
      </div>
      <div className="relative border-l border-line ml-2 pl-4 space-y-4">
        {timelineEvents.length === 0 ? (
          <p className="pl-1 text-label leading-4 italic text-muted">No activity has been recorded in this session.</p>
        ) : (
          timelineEvents.map((event, idx) => {
            const isWarning = isErrorLog(event);
            return (
              <div key={idx} className="relative group">
                {/* Node connector ring anchor */}
                {/* Shape, not only color, separates a warning node from a routine one. */}
                <div
                  role="img"
                  aria-label={isWarning ? 'Warning event' : 'Routine event'}
                  className={`absolute -left-[21px] top-1 h-2.5 w-2.5 border bg-canvas ${
                    isWarning ? 'rotate-45 rounded-[1px] border-warning shadow-sm shadow-warning/30' : 'rounded-full border-highlight'
                  }`}
                />
                <div className="space-y-0.5">
                  <div className="max-w-md break-words font-mono text-label text-muted">
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
