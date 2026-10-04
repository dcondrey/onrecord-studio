import { useAppStore } from '../../store/useAppStore';
import type { DetectorNotRunView } from '../../types';

export function checksNotRunHeading(notRun: readonly DetectorNotRunView[]): string {
  return `${notRun.length} check${notRun.length === 1 ? '' : 's'} did not run`;
}

/**
 * Names each check the latest sweep could not perform and why. Without it a skipped check is
 * indistinguishable from one that ran and found nothing.
 */
export default function ChecksNotRunNotice() {
  const outcome = useAppStore((state) => state.findingsRefreshOutcome);
  if (outcome?.status !== 'completed' || outcome.notRun.length === 0) return null;
  const { notRun } = outcome;
  return (
    <div role="status" className="rounded-lg border border-warning/40 bg-warning/5 px-3 py-2 text-caption text-fg-secondary">
      {notRun.length === 1 ? (
        <p>
          <span className="font-semibold text-fg">{checksNotRunHeading(notRun)}:</span>{' '}
          {notRun[0].detectorName}: {notRun[0].reason}. Its earlier findings are kept.
        </p>
      ) : (
        <>
          <p className="font-semibold text-fg">
            {checksNotRunHeading(notRun)}; their earlier findings are kept:
          </p>
          <ul className="mt-1 list-disc pl-4">
            {notRun.map((check) => (
              <li key={check.detector}>
                {check.detectorName}: {check.reason}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
