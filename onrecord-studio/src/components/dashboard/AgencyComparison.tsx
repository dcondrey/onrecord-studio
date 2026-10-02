import { useMemo, useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { AgencyPerformanceProfile } from '../../types/analysis';
import PageHeader from '../ui/PageHeader';
import EmptyState from '../ui/EmptyState';
import { openSourceConnector } from './sourceEvents';
import ObjectLink from '../objects/ObjectLink';

/**
 * Mirrors `STATUTORY_DEADLINE_Z_SCORE` in `src-tauri/src/analysis/anomaly.rs`, which is not
 * exposed over IPC. Like the backend, only a positive score qualifies: a z-score this far below
 * the mean is faster than peers, and calling it "slower" would be a false statement.
 */
export const PEER_LATENCY_OUTLIER_Z_SCORE = 2.0;

/** Below this many measured closures a single closure moves the average by a fifth or more. */
export const SMALL_SAMPLE_CLOSURES = 5;

export const NO_MEASURABLE_CLOSURE = 'no measurable closure';

export type ComparisonColumn =
  | 'averageClosureDays'
  | 'validLatencySampleSize'
  | 'performanceZScore'
  | 'statutoryViolationCount'
  | 'statutoryViolationRate'
  | 'suspiciousFastClosureCount'
  | 'totalRequestsProcessed'
  | 'totalDocumentsReleased';

interface ColumnDefinition {
  key: ComparisonColumn;
  label: string;
  format: (profile: AgencyPerformanceProfile) => string;
}

const COLUMNS: readonly ColumnDefinition[] = [
  {
    key: 'averageClosureDays',
    label: 'Avg closure days',
    format: (p) => (p.averageClosureDays === null ? NO_MEASURABLE_CLOSURE : p.averageClosureDays.toFixed(1)),
  },
  { key: 'validLatencySampleSize', label: 'Measured closures', format: (p) => p.validLatencySampleSize.toLocaleString() },
  {
    key: 'performanceZScore',
    label: 'Z-score',
    format: (p) => (p.performanceZScore === null ? 'not computed' : p.performanceZScore.toFixed(2)),
  },
  { key: 'statutoryViolationCount', label: 'Statutory violations', format: (p) => p.statutoryViolationCount.toLocaleString() },
  { key: 'statutoryViolationRate', label: 'Violation rate', format: (p) => `${(p.statutoryViolationRate * 100).toFixed(1)}%` },
  { key: 'suspiciousFastClosureCount', label: 'Suspiciously fast closures', format: (p) => p.suspiciousFastClosureCount.toLocaleString() },
  { key: 'totalRequestsProcessed', label: 'Requests processed', format: (p) => p.totalRequestsProcessed.toLocaleString() },
  { key: 'totalDocumentsReleased', label: 'Documents released', format: (p) => p.totalDocumentsReleased.toLocaleString() },
];

export const DEFAULT_COLUMN: ComparisonColumn = 'averageClosureDays';

export function isPeerLatencyOutlier(profile: AgencyPerformanceProfile): boolean {
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
  const requestAnalyticsRefresh = useAppStore((s) => s.requestAnalyticsRefresh);
  const setWorkspaceView = useAppStore((s) => s.setWorkspaceView);
  const savedSourceCount = useAppStore((s) => s.portals.length);
  const [column, setColumn] = useState<ComparisonColumn>(DEFAULT_COLUMN);
  const [direction, setDirection] = useState<'asc' | 'desc'>('desc');
  const [portalFilter, setPortalFilter] = useState<string>('');

  const portals = useMemo(
    () => [...new Set(agencyProfiles.map((p) => p.portalSubdomain))].sort((a, b) => a.localeCompare(b)),
    [agencyProfiles],
  );

  const rows = useMemo(() => {
    const scoped = portalFilter ? agencyProfiles.filter((p) => p.portalSubdomain === portalFilter) : agencyProfiles;
    return sortProfiles(scoped, column, direction);
  }, [agencyProfiles, column, direction, portalFilter]);

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
      <div className="mx-auto w-full max-w-[1400px] space-y-4 p-4 lg:p-6">
        <PageHeader
          icon="bars"
          eyebrow="Agency comparison"
          title="Compare agencies"
          summary="Every agency with an analytics profile, ranked on one measured column at a time."
          method={<>
            A rank is a position on a latency or a count, not a compliance judgment: only the
            statutory violation column is a counted violation, and the slower-than-peers marker is a
            statistical comparison with every other agency that no statute speaks to. An agency with
            fewer than {SMALL_SAMPLE_CLOSURES} measured closures is marked small sample because one
            closure moves its average; an agency that has closed nothing measurable shows {NO_MEASURABLE_CLOSURE}{' '}
            rather than a number.
          </>}
        />

        {agencyProfiles.length === 0 ? (
          savedSourceCount === 0 ? (
            <EmptyState
              icon="building"
              title="No agencies to compare yet"
              body="Profiles are computed from the requests a source has synced. Add a records portal to start."
              actions={<button type="button" onClick={openSourceConnector} className="motion-press focus-ring rounded-lg border primary-action border-transparent px-3 py-1.5 text-label font-semibold text-white">Add a source</button>}
            />
          ) : (
            <EmptyState
              icon="building"
              title="Agency profiles have not been computed"
              body="Profiles are computed after a source finishes syncing. Recompute them from what has already been ingested."
              actions={<>
                <button type="button" onClick={requestAnalyticsRefresh} className="motion-press focus-ring rounded-lg border primary-action border-transparent px-3 py-1.5 text-label font-semibold text-white">Refresh analytics</button>
                <button type="button" onClick={() => setWorkspaceView('activity')} className="motion-press focus-ring rounded-lg border border-control px-3 py-1.5 text-label font-semibold text-fg-secondary">See sync activity</button>
              </>}
            />
          )
        ) : (
          <>
            <div className="flex flex-wrap items-center gap-3 text-label text-muted">
              <label className="flex items-center gap-2">
                <span>Portal</span>
                <select
                  aria-label="Portal filter"
                  value={portalFilter}
                  onChange={(event) => setPortalFilter(event.target.value)}
                  className="focus-ring rounded-lg border border-control bg-canvas px-2 py-1 text-label text-fg"
                >
                  <option value="">All portals</option>
                  {portals.map((portal) => (
                    <option key={portal} value={portal}>
                      {portal}
                    </option>
                  ))}
                </select>
              </label>
              <span className="font-mono">{rows.length.toLocaleString()} agencies</span>
            </div>

            <div className="overflow-x-auto rounded-xl border border-line bg-canvas/40">
              <table className="w-full text-left text-label" aria-label="Agency comparison">
                <thead className="border-b border-line text-caption uppercase tracking-wider text-muted">
                  <tr>
                    <th scope="col" className="px-3 py-2">Rank</th>
                    <th scope="col" className="px-3 py-2">Agency</th>
                    <th scope="col" className="px-3 py-2">Portal</th>
                    {COLUMNS.map((definition) => {
                      const active = definition.key === column;
                      return (
                        <th
                          key={definition.key}
                          scope="col"
                          aria-sort={active ? (direction === 'desc' ? 'descending' : 'ascending') : 'none'}
                          className="px-3 py-2"
                        >
                          <button
                            type="button"
                            onClick={() => toggleSort(definition.key)}
                            className={`focus-ring rounded px-1 uppercase tracking-wider hover:text-accent ${active ? 'text-accent' : ''}`}
                          >
                            {definition.label}
                            {active ? (direction === 'desc' ? ' ▾' : ' ▴') : ''}
                          </button>
                        </th>
                      );
                    })}
                    <th scope="col" className="px-3 py-2">Marks</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-line">
                  {rows.map((profile, index) => {
                    const outlier = isPeerLatencyOutlier(profile);
                    const smallSample = profile.validLatencySampleSize < SMALL_SAMPLE_CLOSURES;
                    return (
                      <tr key={`${profile.portalSubdomain}-${profile.agencyId}`} className="text-fg-secondary">
                        <td className="px-3 py-2 font-mono text-muted">{index + 1}</td>
                        <td className="px-3 py-2 font-semibold">
                          <ObjectLink target={{ kind: 'agency', portalKey: profile.portalSubdomain, agencyId: profile.agencyId, label: profile.agencyName }} className="text-fg decoration-line-strong hover:text-accent">{profile.agencyName}</ObjectLink>
                        </td>
                        <td className="px-3 py-2 text-muted">{profile.portalSubdomain}</td>
                        {COLUMNS.map((definition) => (
                          <td key={definition.key} className="px-3 py-2 font-mono">
                            {definition.format(profile)}
                          </td>
                        ))}
                        <td className="px-3 py-2">
                          <span className="flex flex-wrap gap-1">
                            {outlier && (
                              <span
                                title={`Closure latency z-score at or above ${PEER_LATENCY_OUTLIER_Z_SCORE.toFixed(1)}: slower than peers. Not a statutory finding.`}
                                className="rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 text-caption font-semibold text-warning"
                              >
                                slower than peers
                              </span>
                            )}
                            {smallSample && (
                              <span
                                title={`${profile.validLatencySampleSize} measured closures; the average rests on few observations.`}
                                className="rounded-full border border-line-strong px-2 py-0.5 text-caption text-muted"
                              >
                                small sample
                              </span>
                            )}
                          </span>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </>
        )}
      </div>
    </main>
  );
}
