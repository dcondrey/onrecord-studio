import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { EvidenceGapView, GapStatus } from '../../types';

const STATUS_TEXT: Record<GapStatus, string> = {
  acquiring: 'Collecting public sources',
  exhausted: 'Not in collected sources',
  noAutomaticSource: 'Needs a records request',
};

export default function EvidenceGaps() {
  const [gaps, setGaps] = useState<EvidenceGapView[]>([]);
  const [error, setError] = useState<string | null>(null);
  const gate = useLatestRequestGate();
  const refreshRequests = useAppStore((state) => state.findingsRefreshRequests);

  useEffect(() => {
    const request = gate.begin();
    invokeCommand('list_evidence_gaps', {})
      .then((rows) => {
        if (!gate.isCurrent(request)) return;
        setGaps(rows);
        setError(null);
      })
      .catch((reason) => {
        if (gate.isCurrent(request)) setError(String(reason));
      });
    return gate.invalidate;
  }, [refreshRequests, gate]);

  if (error) return <p className="text-caption text-warning">Evidence gaps unavailable: {error}</p>;
  if (gaps.length === 0) return null;
  return (
    <div aria-label="Evidence gaps" className="flex flex-col gap-2 border-t border-line pt-3">
      <div className="app-eyebrow">Routes that cannot be tested yet</div>
      <ul className="flex flex-col gap-2">
        {gaps.map((gap) => (
          <li key={gap.key} className="rounded border border-line p-2 text-label text-fg-secondary">
            <div className="font-semibold text-fg">{gap.route}</div>
            <div className="mt-1 text-muted">{gap.blocked}</div>
            <div className="mt-1 text-caption text-muted">
              {STATUS_TEXT[gap.status]}
              {gap.status !== 'acquiring' && <> — {gap.request}</>}
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}
