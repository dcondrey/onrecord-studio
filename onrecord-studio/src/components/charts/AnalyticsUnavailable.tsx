import Icon from '../ui/icons';

interface AnalyticsUnavailableProps {
  heading: string;
  detail?: string;
  className?: string;
}

/**
 * Shown when the analytics query failed, in place of the empty state a panel would otherwise
 * fall back to.
 *
 * The distinction matters more here than in an ordinary dashboard. "No fast-drop signals
 * detected" and "Relationship map ready" are findings: they tell an investigator that the
 * corpus was examined and nothing was there. When the query never ran, the panel has no
 * evidence for either claim, and presenting one is the same failure as a false negative in the
 * analysis itself.
 */
export default function AnalyticsUnavailable({ heading, detail, className }: AnalyticsUnavailableProps) {
  return (
    <div
      role="status"
      className={`grid h-full min-h-40 place-items-center rounded-xl border border-dashed border-amber-500/30 bg-amber-500/[0.04] p-4 text-center ${className ?? ''}`}
    >
      <div className="max-w-xs">
        <Icon name="shield" className="mx-auto h-5 w-5 text-amber-400/70" />
        <div className="mt-2 text-[10px] font-semibold text-amber-200/90">{heading}</div>
        <p className="mt-1 text-[9px] leading-relaxed text-slate-400">
          {detail ?? 'The analytics query did not complete, so this panel has nothing to report either way. The activity log records the failure.'}
        </p>
      </div>
    </div>
  );
}
