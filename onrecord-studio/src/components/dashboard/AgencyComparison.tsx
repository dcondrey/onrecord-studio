import { useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { AgencyPerformanceProfile } from '../../types/analysis';
import { cn } from '../../utils/cn';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import PageHeader from '../ui/PageHeader';
import EmptyState from '../ui/EmptyState';
import InlineError from '../ui/InlineError';
import { Select } from '../ui/Field';
import Table, { Td, Th } from '../ui/Table';
import { openSourceConnector } from './sourceEvents';
import ObjectLink from '../objects/ObjectLink';
import DispositionCounts from './DispositionCounts';

/**
 * Mirrors `STATUTORY_DEADLINE_Z_SCORE` in `src-tauri/src/analysis/anomaly.rs`, which is not
 * exposed over IPC. Like the backend, only a positive score qualifies: a z-score this far below
 * the mean is faster than peers, and calling it "slower" would be a false statement.
 */
export const PEER_LATENCY_OUTLIER_Z_SCORE = 2.0;

/** Below this many measured closures a single closure moves the average by a fifth or more. */
const SMALL_SAMPLE_CLOSURES = 5;

export const NO_MEASURABLE_CLOSURE = 'no measurable closure';

type ComparisonColumn =
  | 'averageClosureDays'
  | 'validLatencySampleSize'
  | 'performanceZScore'
  | 'statutoryViolationCount'
  | 'statutoryViolationRate'
  | 'suspiciousFastClosureCount'
  | 'totalRequestsProcessed'
  | 'totalDocumentsReleased';

interface ColumnDefinition {
  key: Exclude<ComparisonColumn, 'performanceZScore'>;
  label: string;
  /** The full wording, shown as the header's tooltip when the label is shortened. */
  full?: string;
  /** `null` is an absent measurement, rendered as words in muted italic. */
  format: (profile: AgencyPerformanceProfile) => string | null;
}

/**
 * The z-score has no column: the "Slower than peers" mark says what it means and carries the value
 * in its tooltip, and a bare score invites reading it as a grade.
 */
const COLUMNS: readonly ColumnDefinition[] = [
  {
    key: 'averageClosureDays',
    label: 'Avg days to close',
    full: 'Average days from filing to closure',
    format: (p) => (p.averageClosureDays === null ? null : p.averageClosureDays.toFixed(1)),
  },
  { key: 'validLatencySampleSize', label: 'Measured closures', full: 'Closures with a measurable latency', format: (p) => p.validLatencySampleSize.toLocaleString() },
  { key: 'statutoryViolationCount', label: 'Violations', full: 'Counted statutory violations', format: (p) => p.statutoryViolationCount.toLocaleString() },
  { key: 'statutoryViolationRate', label: 'Violation rate', full: 'Share of measured requests past the statutory deadline', format: (p) => `${(p.statutoryViolationRate * 100).toFixed(1)}%` },
  {
    key: 'suspiciousFastClosureCount',
    label: 'Fast closures',
    full: 'Unusually fast closures: closed faster than the peer distribution; not a finding of wrongdoing.',
    format: (p) => p.suspiciousFastClosureCount.toLocaleString(),
  },
  { key: 'totalRequestsProcessed', label: 'Requests', format: (p) => p.totalRequestsProcessed.toLocaleString() },
  { key: 'totalDocumentsReleased', label: 'Documents', full: 'Documents released', format: (p) => p.totalDocumentsReleased.toLocaleString() },
];

const DEFAULT_COLUMN: ComparisonColumn = 'averageClosureDays';

function isPeerLatencyOutlier(profile: AgencyPerformanceProfile): boolean {
  return profile.performanceZScore !== null && profile.performanceZScore >= PEER_LATENCY_OUTLIER_Z_SCORE;
}

/**
 * Descending order on the column, with an absent measurement (`null`) always last regardless of
 * direction: absence is not a value and must not sort as the smallest or largest one. Ties fall
 * back to agency name so the order is stable across renders.
 */
export function sortProfiles(
  profiles: readonly AgencyPerformanceProfile[],
  column: ComparisonColumn,
  direction: 'asc' | 'desc',
): AgencyPerformanceProfile[] {
  const sign = direction === 'desc' ? -1 : 1;
  return [...profiles].sort((a, b) => {
    const left = a[column];
    const right = b[column];
    if (left === null && right === null) return a.agencyName.localeCompare(b.agencyName);
    if (left === null) return 1;
    if (right === null) return -1;
    if (left !== right) return sign * (left - right);
    return a.agencyName.localeCompare(b.agencyName);
  });
}

/**
 * Every agency profile the analytics pass produced, across portals, sorted on one measured column.
 * A rank here is a position on a latency or a count; only `statutoryViolationCount` is a counted
 * violation, and the outlier marker is a statistical statement about peers, never a legal one.
 */
export default function AgencyComparison() {
  const agencyProfiles = useAppStore((s) => s.agencyProfiles);
  const analyticsError = useAppStore((s) => s.analyticsError);
  const analyticsStatus = useAppStore((s) => s.analyticsStatus);
  const requestAnalyticsRefresh = useAppStore((s) => s.requestAnalyticsRefresh);
  const setWorkspaceView = useAppStore((s) => s.setWorkspaceView);
  const savedSources = useAppStore((s) => s.portals);
  const [column, setColumn] = useState<ComparisonColumn>(DEFAULT_COLUMN);
  const [direction, setDirection] = useState<'asc' | 'desc'>('desc');
  const [portalFilter, setPortalFilter] = useState<string>('');

  const portalName = useMemo(() => {
    const names = new Map(savedSources.map(({ registration }) => [registration.portalKey, registration.displayName]));
    return (portalKey: string) => names.get(portalKey) ?? portalKey;
  }, [savedSources]);

  const portals = useMemo(
    () => [...new Set(agencyProfiles.map((p) => p.portalSubdomain))].sort((a, b) => portalName(a).localeCompare(portalName(b))),
    [agencyProfiles, portalName],
  );

  const rows = useMemo(() => {
    const scoped = portalFilter ? agencyProfiles.filter((p) => p.portalSubdomain === portalFilter) : agencyProfiles;
    return sortProfiles(scoped, column, direction);
  }, [agencyProfiles, column, direction, portalFilter]);
  const spansPortals = new Set(rows.map((p) => p.portalSubdomain)).size > 1;

  const toggleSort = (next: ComparisonColumn) => {
    if (next === column) {
      setDirection((d) => (d === 'desc' ? 'asc' : 'desc'));
      return;
    }
    setColumn(next);
    setDirection('desc');
  };

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-content space-y-6 px-4 pb-10 pt-6 md:px-6">
        <PageHeader
          title="Compare agencies"
          summary="Every agency with an analytics profile, ranked on one measured column at a time."
          method={<>
            A rank is a position on a latency or a count, not a compliance judgment: only the
            violations column is a counted statutory violation, and the slower-than-peers mark is a
            statistical comparison with every other agency that no statute speaks to. An agency with
            fewer than {SMALL_SAMPLE_CLOSURES} measured closures is marked small sample because one
            closure moves its average; an agency that has closed nothing measurable shows {NO_MEASURABLE_CLOSURE}{' '}
            rather than a number.
          </>}
        />

        {analyticsError && (
          <InlineError
            summary={agencyProfiles.length === 0 ? 'Agency profiles could not be read.' : 'Agency profiles could not be refreshed; the table shows the last profiles read.'}
            detail={analyticsError}
            onRetry={requestAnalyticsRefresh}
          />
        )}

        {agencyProfiles.length === 0 && analyticsStatus === 'pending' ? (
          <ComparisonSkeleton />
        ) : agencyProfiles.length === 0 ? (
          analyticsError ? null : savedSources.length === 0 ? (
            <EmptyState
              icon="building"
              title="No agencies to compare yet"
              body="Profiles are computed from the requests a source has synced. Add a records portal to start."
              actions={<Button variant="primary" onClick={openSourceConnector}>Add source</Button>}
            />
          ) : (
            <EmptyState
              icon="building"
              title="Agency profiles have not been computed"
              body="Profiles are computed after a source finishes syncing. Recompute them from what has already been ingested."
              actions={<>
                <Button variant="primary" onClick={requestAnalyticsRefresh}>Refresh analytics</Button>
                <Button onClick={() => setWorkspaceView('activity')}>See sync activity</Button>
              </>}
            />
          )
        ) : (
          <section className="app-panel overflow-hidden" aria-label="Agency comparison table">
            <div className="flex flex-wrap items-center justify-between gap-3 border-b border-line px-5 py-3">
              <Select size="sm" aria-label="Portal filter" value={portalFilter} onChange={(event) => setPortalFilter(event.target.value)} className="w-auto min-w-[12rem]">
                <option value="">All portals</option>
                {portals.map((portal) => (
                  <option key={portal} value={portal}>{portalName(portal)}</option>
                ))}
              </Select>
              <span className="text-caption tabular-nums text-muted">{rows.length.toLocaleString()} {rows.length === 1 ? 'agency' : 'agencies'}</span>
            </div>
            <Table label="Agency comparison">
              <thead>
                <tr>
                  <Th numeric className="sticky left-0 z-[11] w-12 min-w-12">Rank</Th>
                  <Th className="sticky left-12 z-[11] min-w-[12rem]">Agency</Th>
                  {COLUMNS.map((definition) => {
                    const active = definition.key === column;
                    return (
                      <Th
                        key={definition.key}
                        numeric
                        className="px-2"
                        aria-sort={active ? (direction === 'desc' ? 'descending' : 'ascending') : 'none'}
                      >
                        <button
                          type="button"
                          onClick={() => toggleSort(definition.key)}
                          title={definition.full}
                          className={cn('focus-ring inline-flex items-center gap-1 rounded uppercase hover:text-fg', active && 'text-fg')}
                        >
                          {/* The glyph leads so the label's right edge lines up with the right-aligned figures. */}
                          <span aria-hidden="true" className={cn('w-2 text-center', !active && 'invisible')}>{direction === 'desc' ? '▾' : '▴'}</span>
                          {definition.label}
                        </button>
                      </Th>
                    );
                  })}
                </tr>
              </thead>
              <tbody>
                {rows.map((profile, index) => {
                  const outlier = isPeerLatencyOutlier(profile);
                  const smallSample = profile.validLatencySampleSize < SMALL_SAMPLE_CLOSURES;
                  return (
                    <tr key={`${profile.portalSubdomain}-${profile.agencyId}`} className="group">
                      <Td numeric className="sticky left-0 w-12 bg-panel text-muted group-hover:bg-raised">{index + 1}</Td>
                      <Td className="sticky left-12 min-w-[12rem] bg-panel group-hover:bg-raised">
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                          <ObjectLink
                            target={{ kind: 'agency', portalKey: profile.portalSubdomain, agencyId: profile.agencyId, label: profile.agencyName }}
                            className="line-clamp-2 font-medium text-fg no-underline hover:underline"
                          >
                            {profile.agencyName}
                          </ObjectLink>
                          {outlier && (
                            <Badge
                              tone="danger"
                              title={`Closure latency ${profile.performanceZScore?.toFixed(2)}σ above the peer mean (threshold ${PEER_LATENCY_OUTLIER_Z_SCORE.toFixed(1)}σ). Not a statutory finding.`}
                            >
                              Slower than peers
                            </Badge>
                          )}
                          {smallSample && (
                            <Badge tone="neutral" dashed title={`${profile.validLatencySampleSize} measured closures; the average rests on few observations.`}>
                              Small sample
                            </Badge>
                          )}
                        </div>
                        {spansPortals && <p className="mt-0.5 text-caption text-muted" title={profile.portalSubdomain}>{portalName(profile.portalSubdomain)}</p>}
                      </Td>
                      {COLUMNS.map((definition) => {
                        const value = definition.format(profile);
                        return (
                          <Td key={definition.key} numeric className={cn('px-2', value === null && 'italic text-muted')}>
                            {value ?? NO_MEASURABLE_CLOSURE}
                          </Td>
                        );
                      })}
                    </tr>
                  );
                })}
              </tbody>
            </Table>
          </section>
        )}

        {savedSources.length > 0 && <DispositionCounts portalSubdomain={portalFilter || null} />}
      </div>
    </main>
  );
}

/** The table at its loaded shape: the same header row and rows at body height, so nothing shifts on arrival. */
function ComparisonSkeleton() {
  return (
    <section className="app-panel overflow-hidden" aria-label="Agency comparison table" aria-busy="true">
      <div className="flex items-center justify-between gap-3 border-b border-line px-5 py-3">
        <div className="skeleton h-7 w-48 rounded-md" />
        <div className="skeleton h-4 w-20 rounded" />
      </div>
      <div role="status">
        <span className="sr-only">Loading agency profiles</span>
        <Table label="Agency comparison, loading">
          <thead>
            <tr>
              <Th numeric className="w-12 min-w-12">Rank</Th>
              <Th className="min-w-[12rem]">Agency</Th>
              {COLUMNS.map((definition) => <Th key={definition.key} numeric className="px-2">{definition.label}</Th>)}
            </tr>
          </thead>
          <tbody aria-hidden="true">
            {Array.from({ length: 6 }, (_, row) => (
              <tr key={row}>
                <Td numeric><span className="skeleton ml-auto block h-3 w-4 rounded" /></Td>
                <Td><span className={cn('skeleton block h-3 rounded', row % 2 === 0 ? 'w-48' : 'w-36')} /></Td>
                {COLUMNS.map((definition) => (
                  <Td key={definition.key} numeric className="px-2"><span className="skeleton ml-auto block h-3 w-10 rounded" /></Td>
                ))}
              </tr>
            ))}
          </tbody>
        </Table>
      </div>
    </section>
  );
}
