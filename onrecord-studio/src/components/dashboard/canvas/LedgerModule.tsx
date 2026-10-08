import { useMemo, type ReactNode } from 'react';
import { useAppStore } from '../../../store/useAppStore';
import type { AgencyPerformanceProfile } from '../../../types';
import { paintFor } from '../../charts/core/claim';
import { countedHeadline, ledgerOrder } from '../../charts/figures/DepartmentLedger';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Skeleton from '../../ui/Skeleton';
import CanvasModule from './CanvasModule';

const ROWS = 4;
const COUNTED_FILL = paintFor('counted', 'fill').fill;

/**
 * The counted deadline ledger as a canvas module: the counted sentence, then the departments that
 * account for the most counted misses on one shared count scale. It is the page's only legal claim,
 * so it alone wears the solid "Counted" badge and the crimson mark. It expands into the full
 * response analytics.
 */
export default function LedgerModule({ expanded, onToggle, detail, onInspect, portalName, span }: {
  expanded: boolean;
  onToggle(): void;
  detail: ReactNode;
  onInspect(profile: AgencyPerformanceProfile): void;
  portalName(portalKey: string): string;
  span?: string;
}) {
  const profiles = useAppStore((state) => state.agencyProfiles);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const analyticsStatus = useAppStore((state) => state.analyticsStatus);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const requestAnalyticsRefresh = useAppStore((state) => state.requestAnalyticsRefresh);
  const caseScoped = useAppStore((state) => state.activeCaseId !== null && state.scopeToCase);
  const scoped = useMemo(
    () => profiles.filter((profile) => (!activePortalKey || profile.portalSubdomain === activePortalKey) && profile.totalRequestsProcessed > 0),
    [activePortalKey, profiles],
  );
  const spansPortals = new Set(scoped.map((profile) => profile.portalSubdomain)).size > 1;
  const missed = ledgerOrder(scoped.filter((profile) => profile.statutoryViolationCount > 0));
  const shown = missed.slice(0, ROWS);
  const max = Math.max(1, ...shown.map((profile) => profile.statutoryViolationCount));
  const common = {
    id: 'canvas-ledger',
    icon: 'clock' as const,
    title: 'Deadline ledger',
    badges: <Badge tone="violation">Counted</Badge>,
    expanded,
    onToggle,
    expandLabel: 'Show response analytics',
    collapseLabel: 'Hide response analytics',
    detail,
    span,
  };

  if (caseScoped) {
    return <CanvasModule {...common} headline="Hidden while a case scope is active: these are all-time totals for every request, not the case’s." />;
  }
  if (profiles.length === 0 && analyticsError) {
    return (
      <CanvasModule {...common} headline="Response analytics couldn’t be read, so no deadline is counted here.">
        <Button size="sm" className="w-fit" onClick={requestAnalyticsRefresh}>Try again</Button>
      </CanvasModule>
    );
  }
  if (profiles.length === 0 && analyticsStatus === 'pending') {
    return (
      <CanvasModule {...common} headline={null}>
        <Skeleton variant="lines" lines={4} label="Counting statutory deadlines" />
      </CanvasModule>
    );
  }
  if (scoped.length === 0) {
    return <CanvasModule {...common} headline="No request has been reviewed yet, so no statutory deadline has been counted." />;
  }

  return (
    <CanvasModule
      {...common}
      headline={countedHeadline(scoped)}
      footer="All-time totals. Each miss is counted by the statute evaluator against the request’s own dates."
    >
      {shown.length > 0 && (
        <ol aria-label="Departments with the most counted misses" className="-mx-2 grid gap-px">
          {shown.map((profile) => {
            const name = spansPortals ? `${profile.agencyName} (${portalName(profile.portalSubdomain)})` : profile.agencyName;
            const value = `${profile.statutoryViolationCount.toLocaleString('en-US')} of ${profile.totalRequestsProcessed.toLocaleString('en-US')}`;
            return (
              <li key={`${profile.portalSubdomain}:${profile.agencyId}`}>
                <button
                  type="button"
                  onClick={() => onInspect(profile)}
                  title={`${name}: ${value} requests past the statutory deadline. Opens its requests in Evidence.`}
                  className="focus-ring group grid w-full gap-1.5 rounded-md px-2 py-2 text-left transition-colors duration-fast hover:bg-raised"
                >
                  <span className="flex min-w-0 items-baseline justify-between gap-3">
                    <span className="min-w-0 truncate text-body text-fg">{name}</span>
                    <span className="shrink-0 text-caption tabular-nums text-fg-secondary">{value}<span className="sr-only"> requests past the statutory deadline</span></span>
                  </span>
                  <span aria-hidden="true" className="block h-1.5 w-full overflow-hidden rounded-full" style={{ background: 'var(--viz-track)' }}>
                    <span className="block h-full rounded-full" style={{ width: `${(profile.statutoryViolationCount / max) * 100}%`, background: COUNTED_FILL }} />
                  </span>
                </button>
              </li>
            );
          })}
        </ol>
      )}
      {missed.length > ROWS && (
        <p className="mt-2 text-caption tabular-nums text-muted">
          {(missed.length - ROWS).toLocaleString('en-US')} more {missed.length - ROWS === 1 ? 'department' : 'departments'} with a counted miss
        </p>
      )}
    </CanvasModule>
  );
}
