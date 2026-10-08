import type { ReactNode } from 'react';
import type { FindingSeverity } from '../../types';
import { formatShortDate, severityLabel } from '../objects/labels';
import SeverityGlyph from '../dashboard/SeverityGlyph';

/**
 * A queued lead, set inside the queue's button: what kind of lead it is, the headline, then how
 * serious, whose and when. A ranked lead adds its rank and its attention score as a bar on the
 * queue's shared scale; the bar is violet because the app surfaced the lead, never because the
 * score claims anything about the record.
 */
export default function LeadCard({ badges, headline, severity, subject, time, rank, score }: {
  badges: ReactNode;
  headline: string;
  severity?: FindingSeverity;
  subject?: ReactNode;
  /** ISO instant; omitted when the item has no date. */
  time?: string | null;
  rank?: number;
  /** Points and the queue's largest score, so every bar shares one scale. */
  score?: { points: number; max: number };
}) {
  const meta = [
    severity && (
      <span key="severity" className="flex shrink-0 items-center gap-1 text-fg-secondary">
        <SeverityGlyph severity={severity} />
        {severityLabel(severity)}
      </span>
    ),
    subject && <span key="subject" className="min-w-0 truncate">{subject}</span>,
    time && <time key="time" dateTime={time} className="shrink-0 tabular-nums">{formatShortDate(time)}</time>,
  ].filter(Boolean);
  return (
    <>
      <span className="flex min-w-0 items-center gap-1.5">{badges}</span>
      <span className="mt-1.5 line-clamp-2 block text-body font-medium text-fg">{headline}</span>
      {meta.length > 0 && (
        <span className="mt-1 flex min-w-0 items-center gap-1.5 text-caption text-muted">
          {meta.flatMap((part, index) => (index === 0 ? [part] : [<span key={`dot-${index}`} aria-hidden="true" className="shrink-0">·</span>, part]))}
        </span>
      )}
      {score && (
        <span className="mt-2 flex items-center gap-2">
          {rank !== undefined && <span className="w-7 shrink-0 text-caption font-semibold tabular-nums text-fg-secondary">#{rank}</span>}
          <span className="sr-only">Attention score {score.points}</span>
          <span aria-hidden="true" className="h-1.5 flex-1 overflow-hidden rounded-full bg-viz-track">
            <span className="block h-full rounded-full bg-viz-lead" style={{ width: `${Math.max(2, (score.points / Math.max(1, score.max)) * 100)}%` }} />
          </span>
          <span aria-hidden="true" className="w-6 text-right text-caption font-medium tabular-nums text-fg-secondary">{score.points}</span>
        </span>
      )}
    </>
  );
}
