import type { AnalyticalDatasetSummary, AnalyticalSchema } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import type { ServiceLead } from './serviceAccountability';
import { records, wholeNumber, type QueryEntry } from './leadRows';

/** A stored-property log: one row per entry taken into storage, with what became of it. Found by
 * column names; the disposition wording is counted as the agency wrote it, never interpreted. */
const names = {
  intake: ['date_stored', 'date_received', 'date_impounded', 'date_collected', 'intake_date', 'storage_date', 'date_of_intake'],
  disposition: ['disposition', 'property_disposition', 'final_disposition', 'item_disposition'],
  released: ['date_released', 'date_retrieved', 'date_claimed', 'date_returned', 'release_date', 'retrieved_date', 'pickup_date'],
};
const LISTED = 8;

export function belongingsQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const columns = schema.columns.filter((column) => column.tableName === dataset.tableName);
  const first = (candidates: string[]) => candidates.find((name) => columns.some((column) => column.columnName === name)) ?? null;
  const [intake, disposition, released] = [first(names.intake), first(names.disposition), first(names.released)];
  if (!intake || !disposition) return [];
  const filled = (name: string) => `NULLIF(TRIM(CAST(${quoteIdentifier(name)} AS VARCHAR)), '')`;
  // Released logs write dates as "7/31/2023" or "7/31/2023, 10:42 AM"; other exports use ISO text.
  const day = (name: string) => `CAST(COALESCE(TRY_CAST(CAST(${quoteIdentifier(name)} AS VARCHAR) AS TIMESTAMP), TRY_STRPTIME(CAST(${quoteIdentifier(name)} AS VARCHAR), ['%m/%d/%Y, %I:%M %p', '%m/%d/%Y %H:%M', '%m/%d/%Y', '%-m/%-d/%Y'])) AS DATE)`;
  return [{
    id: 'belongings_disposition', title: 'Stored-property entries by recorded disposition',
    method: `Rows of ${dataset.displayName} are counted by the text in ${disposition} exactly as written; rows where it is blank are counted apart. ${released ? `An entry is counted as released when ${released} is not blank.` : 'The table has no release or retrieval date column, so releases are not counted.'} A row is an entry in the log as released, not necessarily one item or one person.`,
    // A missing release column yields NULL, not zero: nothing was counted.
    sql: `WITH entries AS (
 SELECT ${filled(disposition)} AS disposition, ${released ? `${filled(released)} IS NOT NULL` : 'CAST(NULL AS BOOLEAN)'} AS released, ${day(intake)} AS taken
 FROM ${quoteIdentifier(dataset.tableName)}
)
SELECT disposition, COUNT(*) AS entries, SUM(COUNT(*)) OVER () AS all_entries, COUNT(*) OVER () AS dispositions,
 ${released ? 'SUM(COUNT(*) FILTER (WHERE released)) OVER ()' : 'CAST(NULL AS BIGINT)'} AS released_entries,
 CAST(MIN(MIN(taken)) OVER () AS VARCHAR) AS first_taken, CAST(MAX(MAX(taken)) OVER () AS VARCHAR) AS last_taken
FROM entries GROUP BY disposition ORDER BY entries DESC, disposition LIMIT ${LISTED}`,
  }];
}

const shown = (value: unknown) => wholeNumber(Number(value));

export function belongingsLeads(results: QueryEntry[]): ServiceLead[] {
  const entry = results.find((item) => item.query.id === 'belongings_disposition');
  if (!entry?.result?.rows.length) return [];
  const rows = records(entry.result);
  const [top] = rows;
  const named = (row: Record<string, unknown>) => row.disposition == null ? 'no disposition recorded' : `recorded as "${String(row.disposition)}"`;
  const unlisted = Number(top.dispositions) - rows.length;
  return [{ key: 'belongings_disposition', priority: 96, query: entry.query,
    title: `${shown(top.entries)} of ${shown(top.all_entries)} stored-property entries have ${named(top)}`,
    detail: `The log holds ${shown(top.all_entries)} entries. By disposition as written: ${rows.map((row) => `${shown(row.entries)} ${named(row)}`).join('; ')}${unlisted > 0 ? `; ${shown(unlisted)} other wordings are not listed` : ''}. ${top.released_entries == null ? 'The log has no release or retrieval date column, so it cannot show how many entries were returned.' : `${shown(top.released_entries)} entries have a release or retrieval date.`} An entry is a row of the log as released, not necessarily one item or one person, and the disposition wording is the agency's own.`,
    question: 'How do these entries compare with the abatements logged for the same period, and what storage and return does the written procedure require?',
    scope: top.first_taken != null && top.last_taken != null ? { period: `${String(top.first_taken)} to ${String(top.last_taken)}` } : undefined,
  }];
}
