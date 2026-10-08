import type { ReactNode } from 'react';
import type { FindingSeverity } from '../../types';
import type { InspectorTarget } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import ObjectLink from '../objects/ObjectLink';
import { formatShortDate, severityLabel } from '../objects/labels';
import SeverityGlyph from './SeverityGlyph';

/** What the card opens: an object in the inspector, or an action such as switching view. */
export type InsightTarget = { target: InspectorTarget } | { onOpen: () => void };

// The headline control's ::after covers the whole card, so the card is one hit target while the
// footer actions (raised above it) stay separately clickable.
const STRETCHED =
  'text-left text-heading text-fg no-underline decoration-fg/40 underline-offset-2 hover:underline focus-visible:underline after:absolute after:inset-0 after:rounded-lg after:content-[""]';

/**
 * One lead or item that needs a reader's attention: badges, a one-line headline, who and when.
 * The badge row says what kind of item it is; only a counted statutory violation is solid.
 */
export default function InsightCard({ badges, severity, headline, open, body, subject, time, timeLabel, meta, chips, actions, nested = false, className }: {
  badges: ReactNode;
  severity?: FindingSeverity;
  headline: string;
  open?: InsightTarget;
  /** Supporting sentences under the headline. */
  body?: ReactNode;
  /** A display name, never a raw portal key. */
  subject?: ReactNode;
  /** ISO instant; rendered as a short date in a <time>. Omitted when there is no date. */
  time?: string | null;
  /** The date as worded for this item ("Tue, Oct 13 · 6:00 PM"); defaults to the short date. */
  timeLabel?: string;
  /** A relative phrase after the date ("in 3 days"). */
  meta?: string;
  chips?: ReactNode;
  actions?: ReactNode;
  /** Inside a panel the card takes the nested radius. */
  nested?: boolean;
  className?: string;
}) {
  return (
    <article
      className={cn(
        'relative flex h-full flex-col gap-2 border border-line bg-panel p-4 transition-colors',
        nested ? 'rounded-lg' : 'rounded-xl shadow-panel',
        open && 'hover:border-line-strong hover:bg-raised/40',
        className,
      )}
    >
      <div className="flex flex-wrap items-center gap-2">{badges}</div>
      <h4 className="min-w-0 text-heading text-fg">
        {open && 'target' in open ? (
          <ObjectLink target={open.target} className={STRETCHED}>{headline}</ObjectLink>
        ) : open ? (
          <button type="button" onClick={open.onOpen} className={cn('focus-ring rounded-sm', STRETCHED)}>{headline}</button>
        ) : (
          headline
        )}
      </h4>
      {body && <div className="space-y-1 text-body text-fg-secondary">{body}</div>}
      {(severity || subject || time || meta) && (
        <p className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-caption text-muted">
          {[
            severity && (
              <span key="severity" className="flex items-center gap-1 text-fg-secondary">
                <SeverityGlyph severity={severity} />
                {severityLabel(severity)}
              </span>
            ),
            subject && <span key="subject" className="min-w-0 truncate">{subject}</span>,
            time && <time key="time" dateTime={time} className="tabular-nums">{timeLabel ?? formatShortDate(time)}</time>,
            meta && <span key="meta" className="tabular-nums">{meta}</span>,
          ].filter(Boolean).flatMap((part, index) => (index === 0 ? [part] : [<span key={`dot-${index}`} aria-hidden="true">·</span>, part]))}
        </p>
      )}
      {chips && <div className="flex flex-wrap gap-1.5">{chips}</div>}
      {actions && <div className="relative z-10 mt-auto flex flex-wrap items-center gap-2 pt-2">{actions}</div>}
    </article>
  );
}
