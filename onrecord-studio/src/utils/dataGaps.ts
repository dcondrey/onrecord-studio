import type { AnalyticalDatasetSummary, AnalyticalSchema, CityDataset } from '../types';
import type { AutomaticProfile } from './automaticDiscovery';
import { budgetTable, serviceCompanions, staffGroupDepartments } from './serviceAccountability';

/** A question the imported tables cannot answer, and the published table that would answer it. */
export interface DataGap {
  key: string;
  snapshotId: string;
  title: string;
  detail: string;
  fetch: { dataset: CityDataset; year: number | null };
}

const GET_IT_DONE = 'https://seshat.datasd.org/get_it_done_reports/';
const OPEN_REPORTS = `${GET_IT_DONE}get_it_done_requests_open_datasd.csv`;
const closedReports = (year: number) => `${GET_IT_DONE}get_it_done_requests_closed_${year}_datasd.csv`;
/** The publisher replaces the open file daily; a copy this old is worth comparing with a new one. */
const RECOPY_DAYS = 7;

const day = (dataset: AnalyticalDatasetSummary) => dataset.retrievedAt.slice(0, 10);
/** A closed-reports import filtered to one service cannot say what became of other services' reports. */
function holdsEveryService(dataset: AnalyticalDatasetSummary): boolean {
  try {
    const filter: unknown = (JSON.parse(dataset.validationSummary) as { derivedSubset?: { filter?: unknown } | null }).derivedSubset?.filter;
    return typeof filter !== 'string' || filter.startsWith('service_request_id');
  } catch { return true; }
}

export function dataGaps(schema: AnalyticalSchema, profiles: Record<string, AutomaticProfile>, today: string): DataGap[] {
  const latest = schema.datasets.filter((dataset) => dataset.sourceUri === OPEN_REPORTS).sort((a, b) => a.retrievedAt.localeCompare(b.retrievedAt)).at(-1);
  if (!latest) return [];
  const result = (id: string) => profiles[latest.snapshotId]?.results.find((entry) => entry.query.id === id)?.result;
  const gaps: DataGap[] = [];
  const age = (Date.parse(today.slice(0, 10)) - Date.parse(day(latest))) / 86_400_000;
  if (result('service_backlog')?.rows.length && age >= RECOPY_DAYS) gaps.push({
    key: `copy:${latest.snapshotId}`, snapshotId: latest.snapshotId, fetch: { dataset: 'GET_IT_DONE_OPEN', year: null },
    title: 'A newer copy of the open reports would show what changed',
    detail: `The open-reports file was last retrieved ${day(latest)}, and the publisher replaces it daily. A second copy shows which reports left and which arrived since then.`,
  });
  const backlog = result('service_backlog');
  const group = backlog ? backlog.columns.indexOf('staff_group') : -1;
  const owners = [...new Set((backlog?.rows ?? []).flatMap((row) => staffGroupDepartments(row[group]) ?? []))];
  if (owners.length && !budgetTable(schema)) gaps.push({
    key: `budget:${latest.snapshotId}`, snapshotId: latest.snapshotId, fetch: { dataset: 'OPERATING_BUDGET', year: null },
    title: 'The operating budget would show what the responsible departments were given',
    detail: `The open reports are handled by staff groups read as ${owners.join(', ')}. The City’s operating budget file lists each department’s adopted amounts by fiscal year.`,
  });
  const movement = result('service_movement');
  const earlier = serviceCompanions(schema, latest)?.earlier;
  const unrecorded = movement?.rows.length ? Number(movement.rows[0][movement.columns.indexOf('all_no_outcome_record')]) : 0;
  if (earlier && unrecorded > 0) for (let year = Number(day(earlier).slice(0, 4)); year <= Number(day(latest).slice(0, 4)); year += 1) {
    if (schema.datasets.some((dataset) => dataset.sourceUri === closedReports(year) && day(dataset) >= day(latest) && holdsEveryService(dataset))) continue;
    gaps.push({
      key: `outcome:${latest.snapshotId}:${year}`, snapshotId: latest.snapshotId, fetch: { dataset: 'GET_IT_DONE_OUTCOMES', year },
      title: `${unrecorded.toLocaleString('en-US')} reports left the open file with no close record imported`,
      detail: `The publisher’s closed-reports file for ${year} records close dates and referrals. Only the rows for reports seen in an imported open copy are kept.`,
    });
  }
  return gaps;
}
