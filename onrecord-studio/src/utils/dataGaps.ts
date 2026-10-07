import type { AnalyticalDatasetSummary, AnalyticalSchema, CityDataset } from '../types';
import type { AutomaticProfile } from './automaticDiscovery';
import { budgetTable, serviceCompanions, staffGroupDepartments } from './serviceAccountability';
import { missingStopCompanions } from './stopLeads';
import { holdsAbatementLog } from './abatementLeads';
import { closedReports, encampmentReportCoverage, OPEN_REPORTS, shelterInventoryTable } from './capacityLeads';
import { actualsTable } from './spendingLeads';
import { missingCallCompanions } from './callLeads';

/** A question the imported tables cannot answer, and what would answer it: one published table to
 * fetch, or a records portal to collect from for a subject. */
export interface DataGap {
  key: string;
  snapshotId: string;
  title: string;
  detail: string;
  fetch?: { dataset: CityDataset; year: number | null };
  collect?: { url: string; subject: string };
}

/** The publisher replaces the open file daily; a copy this old is worth comparing with a new one. */
const RECOPY_DAYS = 7;
const CITY_RECORDS = 'https://sandiego.nextrequest.com';
/** One portal search per comma-separated entry. */
const ENCAMPMENT_RECORDS = 'encampment, abatement, homeless shelter, public restroom, stored property';

const day = (dataset: AnalyticalDatasetSummary) => dataset.retrievedAt.slice(0, 10);
/** A closed-reports import filtered to one service cannot say what became of other services' reports. */
function holdsEveryService(dataset: AnalyticalDatasetSummary): boolean {
  try {
    const filter: unknown = (JSON.parse(dataset.validationSummary) as { derivedSubset?: { filter?: unknown } | null }).derivedSubset?.filter;
    return typeof filter !== 'string' || filter.startsWith('service_request_id');
  } catch { return true; }
}

export function dataGaps(schema: AnalyticalSchema, profiles: Record<string, AutomaticProfile>, today: string): DataGap[] {
  const gaps: DataGap[] = [];
  for (const dataset of schema.datasets) for (const missing of missingStopCompanions(schema, dataset)) {
    gaps.push({
      key: `stops:${dataset.snapshotId}:${missing.dataset}`, snapshotId: dataset.snapshotId, fetch: { dataset: missing.dataset, year: null },
      title: 'The stop records need their companion table',
      detail: `${dataset.displayName} lists the persons stopped. The publisher keeps ${missing.holds} in a separate table, matched on stop and person number.`,
    });
  }
  for (const dataset of schema.datasets) for (const missing of missingCallCompanions(schema, dataset)) {
    gaps.push({
      key: `calls:${dataset.snapshotId}:${missing.dataset}`, snapshotId: dataset.snapshotId, fetch: { dataset: missing.dataset, year: null },
      title: 'The calls for service need their code list',
      detail: `${dataset.displayName} records each call by code. The publisher keeps ${missing.holds} in a separate table.`,
    });
  }
  const budget = budgetTable(schema);
  if (budget && !actualsTable(schema)) gaps.push({
    key: `actuals:${budget.snapshotId}`, snapshotId: budget.snapshotId, fetch: { dataset: 'OPERATING_ACTUALS', year: null },
    title: 'The operating actuals would show what each department spent against what was adopted',
    detail: `${budget.displayName} lists adopted amounts and no table of actual spending is imported. The City publishes actual revenue and expense by department, fund and account for each closed fiscal year.`,
  });
  const latest = schema.datasets.filter((dataset) => dataset.sourceUri === OPEN_REPORTS).sort((a, b) => a.retrievedAt.localeCompare(b.retrievedAt)).at(-1);
  if (!latest) return gaps;
  const result = (id: string) => profiles[latest.snapshotId]?.results.find((entry) => entry.query.id === id)?.result;
  const services = result('service_backlog');
  const encampmentReports = Boolean(services?.rows.some((row) => /encampment/i.test(String(row[services.columns.indexOf('category')]))));
  // The City publishes no abatement log; the ones that exist were released to records requests.
  if (encampmentReports && !holdsAbatementLog(schema)) gaps.push({
    key: `records:${latest.snapshotId}`, snapshotId: latest.snapshotId, collect: { url: CITY_RECORDS, subject: ENCAMPMENT_RECORDS },
    title: 'Released City records would show when encampment notices were posted and abatements carried out',
    detail: `Encampment reports are open in the imported file, and no log of posted notices or completed abatements is imported. The City has released such logs, arrest and citation exports and contractor invoices to public records requests. Collecting searches ${new URL(CITY_RECORDS).host} for ${ENCAMPMENT_RECORDS} and downloads what was released; a released spreadsheet becomes a table when Data upkeep imports it.`,
  });
  // Encampment reports say where residents complained; the stop records say what enforcement followed.
  if (!schema.columns.some((column) => column.columnName === 'perceivedunhoused') && encampmentReports) gaps.push({
    key: `stops:${latest.snapshotId}`, snapshotId: latest.snapshotId, fetch: { dataset: 'RIPA_STOPS', year: null },
    title: 'The police stop records would show enforcement against people perceived as unhoused',
    detail: 'Encampment reports are open in the imported file. SDPD publishes each person stopped with the officer’s perceived-unhoused entry, the reason, the result and property seized.',
  });
  const inventory = shelterInventoryTable(schema);
  if (encampmentReports && !inventory) gaps.push({
    key: `shelter:${latest.snapshotId}`, snapshotId: latest.snapshotId, fetch: { dataset: 'SHELTER_INVENTORY', year: null },
    title: 'The emergency shelter inventory would show how many beds each year\'s count found',
    detail: 'Encampment reports are open in the imported file. UCSD\'s Homelessness Hub publishes each emergency-shelter project and its beds per year, compiled from the Regional Task Force on Homelessness count.',
  });
  // One year at a time, latest first: each closed-year file makes one more fiscal year countable.
  const coverage = inventory && budgetTable(schema) ? encampmentReportCoverage(schema) : null;
  if (encampmentReports && coverage?.nextClosedYear != null) gaps.push({
    key: `capacity:${latest.snapshotId}:${coverage.nextClosedYear}`, snapshotId: latest.snapshotId, fetch: { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: coverage.nextClosedYear },
    title: `The ${coverage.nextClosedYear} closed Encampment reports are needed to count a full year of reports filed`,
    detail: `The publisher files a closed report under the year it was closed, so the reports filed in a fiscal year are spread over the open file and every later closed-year file. ${coverage.fiscalYears.length ? `Fiscal ${coverage.fiscalYears[0]} to ${coverage.fiscalYears.at(-1)!} can be counted from the imported files` : 'No fiscal year can be counted from the imported files yet'}; a copy of the ${coverage.nextClosedYear} file${coverage.nextClosedYear === Number(day(coverage.open).slice(0, 4)) ? ' no older than the open copy' : ''} adds another to set beside the budget and the shelter inventory.`,
  });
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
