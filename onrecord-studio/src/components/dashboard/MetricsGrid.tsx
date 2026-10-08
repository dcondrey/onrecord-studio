import { useMemo } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import { VIZ } from '../charts/core/claim';
import Badge from '../ui/Badge';
import InlineError from '../ui/InlineError';

interface MetricCard {
  label: string;
  value: string;
  detail: string;
  /** Null when there is nothing to divide by, or the ratio says nothing a reader can use. */
  ratio: number | null;
  /**
   * The claim the figure makes, worn as the same tag the charts below use: counted is the only
   * crimson, statistical a dashed outline. Values themselves are always ink.
   */
  claim: 'counted' | 'statistical' | null;
  status: string | null;
  policy: string | null;
}

export default function MetricsGrid() {
  const profiles = useAppStore((state) => state.agencyProfiles);
  const correlations = useAppStore((state) => state.crossAgencyCorrelations);
  const analyticsError = useAppStore((state) => state.analyticsError);
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const searchTerm = useAppStore((state) => state.searchTerm);
  const selectedAgencyId = useAppStore((state) => state.selectedAgencyId);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const requestAnalyticsRefresh = useAppStore((state) => state.requestAnalyticsRefresh);
  const values = useMemo(() => {
    const scoped = profiles.filter((profile) => !activePortalKey || profile.portalSubdomain === activePortalKey);
    const requests = scoped.reduce((total, profile) => total + profile.totalRequestsProcessed, 0);
    const closed = scoped.reduce((total, profile) => total + profile.totalClosed, 0);
    const statutory = scoped.reduce((total, profile) => total + profile.statutoryViolationCount, 0);
    const fastClosures = scoped.reduce((total, profile) => total + profile.suspiciousFastClosureCount, 0);
    const correlated = correlations.filter((value) => !activePortalKey || value.portalKey === activePortalKey).length;
    return { requests, closed, statutory, fastClosures, correlations: correlated };
  }, [activePortalKey, correlations, profiles]);

  // Zeros here are indistinguishable from real zeros, and three of these four cards state a
  // conclusion about the corpus. With no analytics result there is no conclusion to state.
  const unavailable = analyticsError !== null && profiles.length === 0 && correlations.length === 0;
  if (unavailable) {
    return (
      <InlineError
        summary="Response analytics couldn’t be read."
        detail={analyticsError ?? undefined}
        onRetry={requestAnalyticsRefresh}
      />
    );
  }

  const cards: MetricCard[] = [
    {
      label: 'Requests reviewed',
      value: values.requests.toLocaleString(),
      detail: `${values.closed.toLocaleString()} closed`,
      ratio: values.requests === 0 ? null : values.closed / values.requests,
      claim: null,
      status: null,
      policy: null,
    },
    {
      label: 'Missed statutory deadlines',
      value: values.statutory.toLocaleString(),
      detail: values.requests > 0 ? `${formatPercent(values.statutory / values.requests)} of reviewed requests` : 'No request has been reviewed yet',
      ratio: values.requests === 0 ? null : values.statutory / values.requests,
      claim: 'counted',
      status: null,
      policy: 'STATUTORY_LATENCY_VIOLATION',
    },
    {
      label: 'Rapid closures',
      value: values.fastClosures.toLocaleString(),
      detail: values.fastClosures > 0 ? 'Closed within 5 minutes of filing; a pattern, not a violation' : 'None closed within 5 minutes',
      ratio: values.closed === 0 ? null : values.fastClosures / values.closed,
      claim: 'statistical',
      status: 'closed',
      policy: 'SUSPICIOUSLY_FAST_CLOSURE',
    },
    {
      label: 'Cross-agency signals',
      value: values.correlations.toLocaleString(),
      detail: 'Patterns shared across agencies',
      ratio: null,
      claim: null,
      status: null,
      policy: 'CROSS_AGENCY_311_CORRELATION',
    },
  ];

  return (
    <section aria-label="Audit metrics" className="grid grid-cols-1 divide-y divide-line overflow-hidden rounded-lg border border-line sm:grid-cols-2 sm:divide-y-0 xl:grid-cols-4 xl:divide-x">
      {cards.map((card) => (
        <button
          key={card.label}
          type="button"
          onClick={() => {
            setSearchFilters(searchTerm, selectedAgencyId, card.status, card.policy);
            setWorkspaceView('evidence');
          }}
          className="focus-ring group flex min-w-0 flex-col justify-start p-4 text-left transition-colors duration-fast hover:bg-raised/60 focus-visible:ring-inset"
        >
          <span className="flex min-w-0 items-center gap-2">
            <span className="truncate text-caption text-muted" title={card.label}>{card.label}</span>
            {card.claim === 'counted' && <Badge tone="violation" className="shrink-0">Counted</Badge>}
            {card.claim === 'statistical' && <Badge dashed className="shrink-0">Statistical</Badge>}
          </span>
          <span className="mt-1 block text-metric tabular-nums text-fg">{card.value}</span>
          <span className="mt-1 block min-h-8 text-caption text-muted">{card.detail}</span>
          <span aria-hidden="true" className={cn('mt-3 flex items-center gap-2', card.ratio === null && 'invisible')}>
            <span className="block h-1 min-w-0 flex-1 overflow-hidden rounded-full bg-raised">
              <span className="block h-full origin-left rounded-full" style={{ background: card.claim === 'counted' ? VIZ.counted : VIZ.mark, transform: `scaleX(${Math.min(1, Math.max(0, card.ratio ?? 0))})` }} />
            </span>
            <span className="w-12 text-right text-caption tabular-nums text-muted">{card.ratio === null ? '' : formatPercent(card.ratio)}</span>
          </span>
          <span className="sr-only">Show matching requests</span>
        </button>
      ))}
    </section>
  );
}

function formatPercent(value: number) {
  return new Intl.NumberFormat(undefined, { style: 'percent', maximumFractionDigits: 1 }).format(value);
}
