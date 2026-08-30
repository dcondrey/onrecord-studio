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
 * detected" and "No departments profiled yet" are findings: they tell an investigator that the
 * corpus was examined and nothing was there. When the query never ran, the panel has no
 * evidence for either claim, and presenting one is the same failure as a false negative in the
 * analysis itself.
 *
 * An empty state must also not describe a capability that does not exist. This panel's empty
 * state read "Relationship map ready" until 2026-08-30, promising an entity relationship map
 * that is unbuilt (docs/investigative-workbench-architecture.md section 7, step 4) from a panel
 * that has never drawn one -- it renders agency performance profiles. A caption is the only
 * thing an operator has to go on when the picture is blank, and one that names a feature the
 * app does not have is a false statement about the evidence, not decoration.
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
