import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import { encampmentReportCoverage } from './capacityLeads';
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
/** Reports around abatements are stated only with this many abatement place-days behind them. */
const PLACE_DAY_MINIMUM = 30;
const WINDOW_DAYS = 14;
const NEAR_METRES = 150;
const RING_METRES = 800;
/** An abatement's ring is compared before and after only with this many reports in it beforehand. */
const RING_BASELINE = 10;
/** The same count is taken this many days earlier at the same places, to show how the counts were already moving. */
const EARLIER_DAYS = 28;
/** A place is called repeatedly abated at this many separate days. */
const REPEAT_DAYS = 20;
const REPORT_COORDINATES = [['lat', 'lng'], ['latitude', 'longitude']] as const;

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

/** The Encampment report tables that hold every report filed since `since`, when each has coordinates. */
export function reportsAroundAbatements(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): { tables: AnalyticalDatasetSummary[]; coordinates: readonly [string, string]; since: string; until: string } | null {
  if (logRoles(schema, dataset, names.posted) || !logRoles(schema, dataset, names.completed)) return null;
  const coverage = encampmentReportCoverage(schema);
  if (!coverage?.closed.length) return null;
  const tables = [coverage.open, ...coverage.closed];
  const has = (table: AnalyticalDatasetSummary, name: string) => schema.columns.some((column) => column.tableName === table.tableName && column.columnName === name);
  const coordinates = REPORT_COORDINATES.find(([latitude, longitude]) => tables.every((table) => has(table, latitude) && has(table, longitude)));
  if (!coordinates) return null;
  // Closed files are keyed by close year: with every year from the earliest held to the open copy,
  // each report filed since that year began is in one of the tables.
  const until = coverage.open.retrievedAt.slice(0, 10);
  return { tables, coordinates, since: `${Number(until.slice(0, 4)) - coverage.closed.length + 1}-01-01`, until };
}

function repeatQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  if (logRoles(schema, dataset, names.posted)) return [];
  const roles = logRoles(schema, dataset, names.completed);
  if (!roles) return [];
  return [{
    id: 'abatement_repeat_places', title: 'Places abated on many separate days',
    method: `Abatements in ${dataset.displayName} are counted once per place and day; a place is the recorded coordinates rounded to four decimal places (about eleven metres). One place can carry several address texts in the log; the one written most often is shown. Rows without coordinates or a readable time are counted apart. The log records that an abatement was completed, not who was there.`,
    sql: `WITH logged AS (
 SELECT ROUND(TRY_CAST(${quoteIdentifier(roles.latitude)} AS DOUBLE), 4) AS latitude, ROUND(TRY_CAST(${quoteIdentifier(roles.longitude)} AS DOUBLE), 4) AS longitude,
  CAST(${instant(roles.time)} AS DATE) AS day, TRIM(CAST(${quoteIdentifier(roles.address)} AS VARCHAR)) AS address
 FROM ${quoteIdentifier(dataset.tableName)}
), place_days AS (
 SELECT latitude, longitude, day, MODE(address) AS address FROM logged WHERE latitude IS NOT NULL AND longitude IS NOT NULL AND day IS NOT NULL GROUP BY latitude, longitude, day
), places AS (
 SELECT latitude, longitude, MODE(address) AS address, COUNT(*) AS days, MIN(day) AS first_day, MAX(day) AS last_day FROM place_days GROUP BY latitude, longitude
)
SELECT address, days, CAST(first_day AS VARCHAR) AS first_day, CAST(last_day AS VARCHAR) AS last_day,
 COUNT(*) OVER () AS places, SUM(days) OVER () AS place_days, COUNT(*) FILTER (WHERE days = 1) OVER () AS places_abated_once,
 COUNT(*) FILTER (WHERE days >= ${REPEAT_DAYS}) OVER () AS repeat_places, COALESCE(SUM(days) FILTER (WHERE days >= ${REPEAT_DAYS}) OVER (), 0) AS repeat_place_days,
 (SELECT COUNT(*) FROM logged WHERE latitude IS NULL OR longitude IS NULL OR day IS NULL) AS rows_not_placed
FROM places ORDER BY days DESC, address LIMIT 5`,
  }];
}

function reportQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const reports = reportsAroundAbatements(schema, dataset);
  if (!reports) return [];
  const roles = logRoles(schema, dataset, names.completed)!;
  const [latitude, longitude] = reports.coordinates;
  // A box a little larger than the ring keeps the join small; the distance is then computed exactly enough for these radii.
  return [{
    id: 'abatement_reports_nearby', title: `Encampment reports filed near an abatement in the ${WINDOW_DAYS} days before and after it`,
    method: `Abatements in ${dataset.displayName} are counted once per place and day (coordinates rounded to four decimal places). Encampment reports come from ${reports.tables.map((table) => table.displayName).join(', ')}, once per report number; those tables hold every report filed from ${reports.since} to ${reports.until}, so only abatements whose ${WINDOW_DAYS} days before and after fall inside that period are counted. A report is near an abatement within ${NEAR_METRES} metres and in the surrounding ring from ${NEAR_METRES} to ${RING_METRES} metres, by straight-line distance between the recorded coordinates. Each count is of report and abatement pairs: a report near two abatements is counted twice. Reports filed on the abatement day are counted apart. An abatement's ring is compared before and after only when it held at least ${RING_BASELINE} reports beforehand; a rise by half and a fall by a third are the same ratio in each direction. The same before-and-after count is also taken ${EARLIER_DAYS} days earlier at the same places, for the abatements whose earlier ${WINDOW_DAYS * 2 + 1} days are inside the period; it shows how the counts were already moving and is not a period without abatements.`,
    sql: `WITH abatements AS (
 SELECT DISTINCT ROUND(TRY_CAST(${quoteIdentifier(roles.latitude)} AS DOUBLE), 4) AS latitude, ROUND(TRY_CAST(${quoteIdentifier(roles.longitude)} AS DOUBLE), 4) AS longitude, CAST(${instant(roles.time)} AS DATE) AS day
 FROM ${quoteIdentifier(dataset.tableName)}
), report_rows AS (
${reports.tables.map((table) => ` SELECT TRIM(CAST(${quoteIdentifier('service_request_id')} AS VARCHAR)) AS report_id, TRY_CAST(${quoteIdentifier(latitude)} AS DOUBLE) AS latitude, TRY_CAST(${quoteIdentifier(longitude)} AS DOUBLE) AS longitude,
  CAST(TRY_CAST(CAST(${quoteIdentifier('date_requested')} AS VARCHAR) AS TIMESTAMP) AS DATE) AS day
 FROM ${quoteIdentifier(table.tableName)} WHERE TRIM(CAST(${quoteIdentifier('service_name')} AS VARCHAR)) = 'Encampment'`).join('\n UNION ALL\n')}
), reports AS (
 SELECT report_id, MIN(latitude) AS latitude, MIN(longitude) AS longitude, MIN(day) AS day FROM report_rows GROUP BY report_id
), counted AS (
 SELECT * FROM abatements WHERE latitude IS NOT NULL AND longitude IS NOT NULL AND day IS NOT NULL
  AND day - ${WINDOW_DAYS} >= DATE '${reports.since}' AND day + ${WINDOW_DAYS} <= DATE '${reports.until}'
), pairs AS (
 SELECT counted.latitude AS place_latitude, counted.longitude AS place_longitude, counted.day AS place_day, reports.day - counted.day AS offset_days,
  counted.day - ${EARLIER_DAYS + WINDOW_DAYS} >= DATE '${reports.since}' AS earlier_counted,
  SQRT(POW((reports.latitude - counted.latitude) * 110540, 2) + POW((reports.longitude - counted.longitude) * 111320 * COS(RADIANS(counted.latitude)), 2)) AS metres
 FROM counted JOIN reports ON reports.day BETWEEN counted.day - ${EARLIER_DAYS + WINDOW_DAYS} AND counted.day + ${WINDOW_DAYS}
  AND reports.latitude BETWEEN counted.latitude - 0.0075 AND counted.latitude + 0.0075 AND reports.longitude BETWEEN counted.longitude - 0.0095 AND counted.longitude + 0.0095
)
SELECT (SELECT COUNT(*) FROM abatements) AS abatement_place_days, (SELECT COUNT(*) FROM counted) AS counted_place_days,
 (SELECT COUNT(*) FROM reports WHERE day BETWEEN DATE '${reports.since}' AND DATE '${reports.until}') AS reports_in_period,
 (SELECT COUNT(*) FROM reports WHERE latitude IS NULL OR longitude IS NULL OR day IS NULL) AS reports_not_placed,
 COALESCE(SUM(near_before), 0) AS near_before, COALESCE(SUM(near_after), 0) AS near_after, COALESCE(SUM(near_same_day), 0) AS near_same_day,
 COALESCE(SUM(ring_before), 0) AS ring_before, COALESCE(SUM(ring_after), 0) AS ring_after,
 COUNT(*) FILTER (WHERE ring_before >= ${RING_BASELINE}) AS ring_compared,
 COUNT(*) FILTER (WHERE ring_before >= ${RING_BASELINE} AND ring_after > ring_before) AS ring_more_after,
 COUNT(*) FILTER (WHERE ring_before >= ${RING_BASELINE} AND ring_after < ring_before) AS ring_fewer_after,
 COUNT(*) FILTER (WHERE ring_before >= ${RING_BASELINE} AND ring_after * 2 >= ring_before * 3) AS ring_rose_by_half,
 COUNT(*) FILTER (WHERE ring_before >= ${RING_BASELINE} AND ring_after * 3 <= ring_before * 2) AS ring_fell_by_third,
 (SELECT COUNT(*) FROM counted WHERE day - ${EARLIER_DAYS + WINDOW_DAYS} >= DATE '${reports.since}') AS earlier_place_days,
 COALESCE(SUM(earlier_near_before), 0) AS earlier_near_before, COALESCE(SUM(earlier_near_after), 0) AS earlier_near_after,
 COALESCE(SUM(earlier_ring_before), 0) AS earlier_ring_before, COALESCE(SUM(earlier_ring_after), 0) AS earlier_ring_after,
 '${reports.since}' AS reports_since, '${reports.until}' AS reports_until
FROM (
 SELECT place_latitude, place_longitude, place_day,
  COUNT(*) FILTER (WHERE metres <= ${NEAR_METRES} AND offset_days BETWEEN -${WINDOW_DAYS} AND -1) AS near_before, COUNT(*) FILTER (WHERE metres <= ${NEAR_METRES} AND offset_days > 0) AS near_after,
  COUNT(*) FILTER (WHERE metres <= ${NEAR_METRES} AND offset_days = 0) AS near_same_day,
  COUNT(*) FILTER (WHERE metres > ${NEAR_METRES} AND metres <= ${RING_METRES} AND offset_days BETWEEN -${WINDOW_DAYS} AND -1) AS ring_before,
  COUNT(*) FILTER (WHERE metres > ${NEAR_METRES} AND metres <= ${RING_METRES} AND offset_days > 0) AS ring_after,
  COUNT(*) FILTER (WHERE earlier_counted AND metres <= ${NEAR_METRES} AND offset_days BETWEEN -${EARLIER_DAYS + WINDOW_DAYS} AND -${EARLIER_DAYS + 1}) AS earlier_near_before,
  COUNT(*) FILTER (WHERE earlier_counted AND metres <= ${NEAR_METRES} AND offset_days BETWEEN -${EARLIER_DAYS - 1} AND -${EARLIER_DAYS - WINDOW_DAYS}) AS earlier_near_after,
  COUNT(*) FILTER (WHERE earlier_counted AND metres > ${NEAR_METRES} AND metres <= ${RING_METRES} AND offset_days BETWEEN -${EARLIER_DAYS + WINDOW_DAYS} AND -${EARLIER_DAYS + 1}) AS earlier_ring_before,
  COUNT(*) FILTER (WHERE earlier_counted AND metres > ${NEAR_METRES} AND metres <= ${RING_METRES} AND offset_days BETWEEN -${EARLIER_DAYS - 1} AND -${EARLIER_DAYS - WINDOW_DAYS}) AS earlier_ring_after
 FROM pairs GROUP BY place_latitude, place_longitude, place_day
) AS per_abatement`,
  }];
}

export function abatementQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const completed = abatementCompanion(schema, dataset);
  if (!completed) return [...repeatQueries(schema, dataset), ...reportQueries(schema, dataset)];
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

function repeatLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'abatement_repeat_places');
  if (!entry?.result?.rows.length) return [];
  const rows = entry.result.rows.map((values) => Object.fromEntries(entry.result!.columns.map((name, index) => [name, values[index]])));
  const repeatPlaces = count(rows[0].repeat_places);
  if (!repeatPlaces) return [];
  const placeDays = count(rows[0].place_days); const notPlaced = count(rows[0].rows_not_placed);
  const repeated = rows.filter((row) => count(row.days) >= REPEAT_DAYS);
  const listed = repeated.map((row) => `${String(row.address ?? 'a place with no address text')}, ${shown(count(row.days))} days from ${String(row.first_day)} to ${String(row.last_day)}`);
  return [{ key: 'abatement_repeat_places', priority: 98, query: entry.query,
    title: `${shown(count(rows[0].repeat_place_days))} of ${shown(placeDays)} abatements were at ${shown(repeatPlaces)} ${repeatPlaces === 1 ? 'place' : 'places'} abated on ${REPEAT_DAYS} or more separate days`,
    detail: `The log records abatements at ${shown(count(rows[0].places))} places; ${shown(count(rows[0].places_abated_once))} were abated on one day only. The most abated: ${listed.join('; ')}.${notPlaced ? ` ${shown(notPlaced)} rows have no coordinates or readable time and are in none of these counts.` : ''} A place is the recorded coordinates rounded to about eleven metres, and an abatement is counted once per place and day. The log records that an abatement was completed. It does not record who was there, whether the same people returned, what was offered, or what became of belongings.`,
    question: 'At the places abated most often, what did each abatement cost, and what shelter or storage was offered there? The contractor\'s invoices by date and the outreach and stored-property records for those addresses would show it.',
    scope: { period: `${repeated.map((row) => String(row.first_day)).sort()[0]} to ${repeated.map((row) => String(row.last_day)).sort().at(-1)!}`, identifiers: repeated.flatMap((row) => row.address == null ? [] : [String(row.address)]) },
  }];
}

function reportLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'abatement_reports_nearby');
  if (!entry?.result?.rows.length) return [];
  const row = Object.fromEntries(entry.result.columns.map((name, index) => [name, entry.result!.rows[0][index]]));
  const placeDays = count(row.counted_place_days);
  if (placeDays < PLACE_DAY_MINIMUM) return [];
  const [nearBefore, nearAfter, ringBefore, ringAfter] = [count(row.near_before), count(row.near_after), count(row.ring_before), count(row.ring_after)];
  const notPlaced = count(row.reports_not_placed);
  const compared = count(row.ring_compared);
  const earlier = count(row.earlier_place_days);
  return [{ key: 'abatement_reports_nearby', priority: 94, query: entry.query,
    title: `Around ${shown(placeDays)} abatements, ${shown(nearAfter)} Encampment reports were filed within ${NEAR_METRES} metres in the ${WINDOW_DAYS} days after and ${shown(nearBefore)} in the ${WINDOW_DAYS} days before`,
    detail: `${shown(placeDays)} of ${shown(count(row.abatement_place_days))} abatement place-days fall where the imported reports are complete (${String(row.reports_since)} to ${String(row.reports_until)}, ${shown(count(row.reports_in_period))} Encampment reports). Within ${NEAR_METRES} metres: ${shown(nearBefore)} reports in the ${WINDOW_DAYS} days before, ${shown(count(row.near_same_day))} on the day, ${shown(nearAfter)} in the ${WINDOW_DAYS} days after. From ${NEAR_METRES} to ${RING_METRES} metres: ${shown(ringBefore)} before and ${shown(ringAfter)} after.${compared ? ` Of ${shown(compared)} abatements with at least ${RING_BASELINE} reports in that ring beforehand, ${shown(count(row.ring_more_after))} had more afterwards and ${shown(count(row.ring_fewer_after))} fewer; in ${shown(count(row.ring_rose_by_half))} the count rose by half or more and in ${shown(count(row.ring_fell_by_third))} it fell by a third or more.` : ''}${earlier >= PLACE_DAY_MINIMUM ? ` The same count taken ${EARLIER_DAYS} days earlier at ${shown(earlier)} of these abatements' places: within ${NEAR_METRES} metres ${shown(count(row.earlier_near_before))} then ${shown(count(row.earlier_near_after))}; in the ring ${shown(count(row.earlier_ring_before))} then ${shown(count(row.earlier_ring_after))}. That is how the counts were moving before the abatement.` : ''}${notPlaced ? ` ${shown(notPlaced)} reports have no coordinates or date and are in none of these counts.` : ''} These are counts of report and abatement pairs, so a report near several abatements is counted once for each. A report is a resident's complaint, not a person or a camp, and an abatement is often scheduled because reports were filed, so more reports before than after is expected whatever the abatement did. The counts do not show where anyone went.`,
    question: 'Where the surrounding ring has more reports after an abatement than before, what did the abatement record say became of the people and belongings there? The work orders and the stored-property log for those places would show it.',
    scope: { period: `${String(row.reports_since)} to ${String(row.reports_until)}` },
  }];
}

export function abatementLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'abatement_notice_hours');
  if (!entry?.result?.rows.length) return [...repeatLeads(results), ...reportLeads(results)];
  const row = Object.fromEntries(entry.result.columns.map((name, index) => [name, entry.result!.rows[0][index]]));
  const paired = count(row.paired_places);
  if (paired < PAIRED_MINIMUM) return [];
  const period = row.first_posted && row.last_posted ? ` (notices posted ${String(row.first_posted)} to ${String(row.last_posted)})` : '';
  const early = count(row.completed_before_posted); const unnoticed = count(row.abatements_without_notice_in_log);
  return [{ key: 'abatement_notice_hours', priority: 100, query: entry.query,
    title: `${shown(count(row.under_24_hours))} of ${shown(paired)} abatements matched to a single posted notice were completed less than 24 hours after posting`,
    detail: `${shown(count(row.posted_notices))} notices and ${shown(count(row.completed_abatements))} abatements are in the two logs${period}. ${shown(paired)} places have exactly one of each, matched on address and coordinates. Completed within 3 hours of posting: ${shown(count(row.under_3_hours))}. Within 24 hours: ${shown(count(row.under_24_hours))}. From 24 to 72 hours: ${shown(count(row.from_24_to_72_hours))}. After 72 hours or more: ${shown(count(row.over_72_hours))}.${row.median_hours == null ? '' : ` Median ${String(row.median_hours)} hours.`}${early ? ` ${shown(early)} are recorded completed before the notice was posted.` : ''}${unnoticed ? ` ${shown(unnoticed)} of ${shown(count(row.abatements_in_notice_period))} abatements dated within the notice log's period are at a place with no notice in that log.` : ''} The logs record when a notice was posted and when an abatement was closed. They do not record the notice period that applied, whether anyone was present, or what was done with belongings; 24 and 72 hours are yardsticks, not the agency's stated requirement.`,
    question: 'What notice period does the agency\'s abatement procedure require, and what happened to belongings at the places cleared soonest after posting? A records request for the storage log and the notices as posted would show it.',
    scope: row.first_posted && row.last_posted ? { period: `${String(row.first_posted)} to ${String(row.last_posted)}` } : undefined,
  }];
}
