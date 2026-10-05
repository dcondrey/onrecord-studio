import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import { serviceRequestRoles, type ServiceLead } from './serviceAccountability';

/** Encampment abatement logs as agencies release them: one table of notices posted and one of
 * abatements completed, each with a place and a time. Found by the publisher's column names. */
const names = {
  posted: ['date_posted', 'date_time_posted', 'posted_date', 'notice_posted', 'posting_date'],
  completed: ['date_time_closed', 'date_time_completed', 'date_completed', 'completed_date', 'abatement_date'],
  address: ['customer_provided_address', 'address', 'location', 'street_address'],
  latitude: ['geolocation__latitude', 'latitude', 'lat'],
  longitude: ['geolocation__longitude', 'longitude', 'lng', 'lon'],
};
interface LogRoles { time: string; address: string; latitude: string; longitude: string }
/** A pairing is stated only with this many places behind it. */
const PAIRED_MINIMUM = 30;

function logRoles(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary, time: string[]): LogRoles | null {
  const columns = schema.columns.filter((column) => column.tableName === dataset.tableName);
  const first = (candidates: string[]) => candidates.find((name) => columns.some((column) => column.columnName === name)) ?? null;
  const roles = { time: first(time), address: first(names.address), latitude: first(names.latitude), longitude: first(names.longitude) };
  // A service-report table also has a close date and an address; it is not an abatement log.
  return roles.time && roles.address && roles.latitude && roles.longitude && !serviceRequestRoles(columns) ? roles as LogRoles : null;
}

/** Whether any imported table is a log of posted notices or of completed abatements. */
export function holdsAbatementLog(schema: AnalyticalSchema): boolean {
  return schema.datasets.some((dataset) => logRoles(schema, dataset, names.posted) || logRoles(schema, dataset, names.completed));
}

/** The latest imported log of completed abatements, for a table of posted notices. */
export function abatementCompanion(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): AnalyticalDatasetSummary | null {
  if (!logRoles(schema, dataset, names.posted)) return null;
  return schema.datasets.filter((other) => other.snapshotId !== dataset.snapshotId && !logRoles(schema, other, names.posted) && logRoles(schema, other, names.completed))
    .sort((a, b) => b.retrievedAt.localeCompare(a.retrievedAt) || a.snapshotId.localeCompare(b.snapshotId))[0] ?? null;
}

// Released logs write times as "7/31/2023, 10:42 AM"; other exports use ISO text.
const instant = (name: string) => `COALESCE(TRY_CAST(CAST(${quoteIdentifier(name)} AS VARCHAR) AS TIMESTAMP), TRY_STRPTIME(CAST(${quoteIdentifier(name)} AS VARCHAR), ['%m/%d/%Y, %I:%M %p', '%-m/%-d/%Y, %-I:%M %p', '%m/%d/%Y %H:%M']))`;
const rows = (table: AnalyticalDatasetSummary, roles: LogRoles) => `SELECT LOWER(TRIM(CAST(${quoteIdentifier(roles.address)} AS VARCHAR))) AS address,
  ROUND(TRY_CAST(${quoteIdentifier(roles.latitude)} AS DOUBLE), 5) AS latitude, ROUND(TRY_CAST(${quoteIdentifier(roles.longitude)} AS DOUBLE), 5) AS longitude, ${instant(roles.time)} AS logged_at
 FROM ${quoteIdentifier(table.tableName)}`;
const places = (source: string) => `SELECT address, latitude, longitude, COUNT(*) AS entries, MIN(logged_at) AS logged_at FROM ${source}
 WHERE address IS NOT NULL AND latitude IS NOT NULL AND longitude IS NOT NULL AND logged_at IS NOT NULL GROUP BY address, latitude, longitude`;

export function abatementQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const completed = abatementCompanion(schema, dataset);
  if (!completed) return [];
  const posted = logRoles(schema, dataset, names.posted)!;
  return [{
    id: 'abatement_notice_hours', title: 'Hours between a posted notice and the abatement',
    method: `Notices in ${dataset.displayName} and abatements in ${completed.displayName} are matched on the same recorded address and the same coordinates to five decimal places. Hours are counted only where a place has exactly one notice and exactly one abatement, so a notice is never paired with another notice's abatement. Also counted: abatements dated within the period the notice log covers at a place with no notice in that log. Rows without coordinates or a readable time are counted apart. The logs record a posting time and a closing time, not the notice period that applied.`,
    sql: `WITH posted_rows AS (
 ${rows(dataset, posted)}
), completed_rows AS (
 ${rows(completed, logRoles(schema, completed, names.completed)!)}
), posted_places AS (
 ${places('posted_rows')}
), completed_places AS (
 ${places('completed_rows')}
), paired AS (
 SELECT DATE_DIFF('minute', posted_places.logged_at, completed_places.logged_at) / 60.0 AS hours FROM posted_places JOIN completed_places
  ON completed_places.address = posted_places.address AND completed_places.latitude = posted_places.latitude AND completed_places.longitude = posted_places.longitude
 WHERE posted_places.entries = 1 AND completed_places.entries = 1
), notice_period AS (SELECT MIN(logged_at) AS first_posted, MAX(logged_at) AS last_posted FROM posted_rows
), in_period AS (
 SELECT completed_rows.* FROM completed_rows, notice_period WHERE logged_at >= first_posted AND logged_at <= last_posted AND address IS NOT NULL AND latitude IS NOT NULL AND longitude IS NOT NULL
)
SELECT (SELECT COUNT(*) FROM posted_rows) AS posted_notices, (SELECT COUNT(*) FROM completed_rows) AS completed_abatements,
 (SELECT COUNT(*) FROM posted_rows WHERE logged_at IS NULL OR latitude IS NULL OR longitude IS NULL OR address IS NULL) AS notices_not_matchable,
 COUNT(*) AS paired_places, COUNT(*) FILTER (WHERE hours < 0) AS completed_before_posted, COUNT(*) FILTER (WHERE hours >= 0 AND hours < 3) AS under_3_hours,
 COUNT(*) FILTER (WHERE hours >= 0 AND hours < 24) AS under_24_hours, COUNT(*) FILTER (WHERE hours >= 24 AND hours < 72) AS from_24_to_72_hours,
 COUNT(*) FILTER (WHERE hours >= 72) AS over_72_hours, ROUND(MEDIAN(hours) FILTER (WHERE hours >= 0), 1) AS median_hours,
 (SELECT COUNT(*) FROM in_period) AS abatements_in_notice_period,
 (SELECT COUNT(*) FROM in_period WHERE NOT EXISTS (SELECT 1 FROM posted_places WHERE posted_places.address = in_period.address AND posted_places.latitude = in_period.latitude AND posted_places.longitude = in_period.longitude)) AS abatements_without_notice_in_log,
 (SELECT CAST(CAST(first_posted AS DATE) AS VARCHAR) FROM notice_period) AS first_posted, (SELECT CAST(CAST(last_posted AS DATE) AS VARCHAR) FROM notice_period) AS last_posted
FROM paired`,
  }];
}

const count = (value: unknown): number => { const parsed = Number(value); return value == null || value === '' || !Number.isFinite(parsed) ? 0 : parsed; };
const shown = (value: number) => Math.round(value).toLocaleString('en-US');

export function abatementLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'abatement_notice_hours');
  if (!entry?.result?.rows.length) return [];
  const row = Object.fromEntries(entry.result.columns.map((name, index) => [name, entry.result!.rows[0][index]]));
  const paired = count(row.paired_places);
  if (paired < PAIRED_MINIMUM) return [];
  const period = row.first_posted && row.last_posted ? ` (notices posted ${String(row.first_posted)} to ${String(row.last_posted)})` : '';
  const early = count(row.completed_before_posted); const unnoticed = count(row.abatements_without_notice_in_log);
  return [{ key: 'abatement_notice_hours', priority: 100, query: entry.query,
    title: `${shown(count(row.under_24_hours))} of ${shown(paired)} abatements matched to a single posted notice were completed less than 24 hours after posting`,
    detail: `${shown(count(row.posted_notices))} notices and ${shown(count(row.completed_abatements))} abatements are in the two logs${period}. ${shown(paired)} places have exactly one of each, matched on address and coordinates. Completed within 3 hours of posting: ${shown(count(row.under_3_hours))}. Within 24 hours: ${shown(count(row.under_24_hours))}. From 24 to 72 hours: ${shown(count(row.from_24_to_72_hours))}. After 72 hours or more: ${shown(count(row.over_72_hours))}.${row.median_hours == null ? '' : ` Median ${String(row.median_hours)} hours.`}${early ? ` ${shown(early)} are recorded completed before the notice was posted.` : ''}${unnoticed ? ` ${shown(unnoticed)} of ${shown(count(row.abatements_in_notice_period))} abatements dated within the notice log's period are at a place with no notice in that log.` : ''} The logs record when a notice was posted and when an abatement was closed. They do not record the notice period that applied, whether anyone was present, or what was done with belongings; 24 and 72 hours are yardsticks, not the agency's stated requirement.`,
    question: 'What notice period does the agency\'s abatement procedure require, and what happened to belongings at the places cleared soonest after posting? A records request for the storage log and the notices as posted would show it.',
  }];
}
