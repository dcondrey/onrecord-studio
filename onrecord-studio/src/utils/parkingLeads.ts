import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import type { ServiceLead } from './serviceAccountability';

/** The City's half-year parking citation files, recognized by the publisher's column names. */
const COLUMNS = ['citation_id', 'date_issue', 'vio_code', 'sector1', 'location'];
/** `earlier` is the number the publisher's older files write the same violation under, with
 * the same description. */
const SECTIONS: Record<'oversized' | 'unmoved' | 'habitation', { code: string; earlier?: string; name: string; short: string }> = {
  oversized: { code: '86.0139(A)', name: 'oversized, non-motorized or recreational vehicle', short: 'oversized or recreational vehicle' },
  unmoved: { code: '86.0137(G)', earlier: '86.23(G)', name: 'vehicle not moved 1/10 mile in 72 hours', short: 'vehicle not moved in 72 hours' },
  habitation: { code: '86.0137(F)', earlier: '86.23(F)', name: 'vehicle habitation', short: 'vehicle habitation' },
};
type Section = keyof typeof SECTIONS;
/** The section as cited; the earlier number is named only when citations under it were counted. */
const named = (key: Section, earlierCounted: boolean, short = false) => {
  const { code, earlier, name, short: brief } = SECTIONS[key];
  return `SDMC ${code}${earlier && earlierCounted ? ` or its earlier number ${earlier}` : ''} (${short ? brief : name})`;
};
const label = (key: Section) => named(key, true);
/** A section is described, and a block named, only with this many citations behind it. */
const CITATION_MINIMUM = 30;

const isParkingCitations = (columns: string[]) => COLUMNS.every((name) => columns.includes(name));
const text = (name: string) => `CAST(${quoteIdentifier(name)} AS VARCHAR)`;

/** One row per citation number, from its latest entry: the publisher repeats a number when a
 * citation is corrected. Where the latest entries share an entry time but differ in section,
 * warning, issuing unit or fine as read (not as spelt), the citation is marked conflicted and
 * counted under no unit and in no fine total. The publisher writes a warning as the code then _W. */
function citations(dataset: AnalyticalDatasetSummary, columns: string[]) {
  const optional = (name: string, type: string, expression: string) => columns.includes(name) ? expression : `CAST(NULL AS ${type})`;
  const numbers = Object.entries(SECTIONS).flatMap(([key, section]) => [section.code, ...(section.earlier ? [section.earlier] : [])].map((written) => ({ key, written })));
  const sectionOf = numbers.map(({ key, written }) => `WHEN STARTS_WITH(code, '${written}') THEN '${key}'`).join(' ');
  const matched = numbers.map(({ written }) => `WHEN STARTS_WITH(code, '${written}') THEN '${written}'`).join(' ');
  const earlier = Object.values(SECTIONS).flatMap((section) => section.earlier ? [`'${section.earlier}'`] : []).join(', ');
  return `entries AS (
 SELECT NULLIF(TRIM(${text('citation_id')}), '') AS citation, REPLACE(UPPER(TRIM(${text('vio_code')})), ' ', '') AS written_code,
  NULLIF(TRIM(${text('sector1')}), '') AS sector_text, ${optional('vio_fine', 'VARCHAR', `NULLIF(TRIM(${text('vio_fine')}), '')`)} AS fine_text,
  NULLIF(UPPER(TRIM(${text('location')})), '') AS block, TRY_CAST(${text('date_issue')} AS DATE) AS issued,
  ${optional('date_creation', 'TIMESTAMP', `TRY_CAST(${text('date_creation')} AS TIMESTAMP)`)} AS entered
 FROM ${quoteIdentifier(dataset.tableName)}
), coded AS (
 SELECT *, CASE WHEN STARTS_WITH(written_code, 'SDMC') THEN SUBSTR(written_code, 5) ELSE written_code END AS code FROM entries WHERE citation IS NOT NULL
), parsed AS (
 SELECT *, CASE ${sectionOf} END AS section, CASE ${matched} END AS matched_code,
  CASE WHEN sector_text IS NULL THEN NULL WHEN CONTAINS(UPPER(sector_text), 'POLICE') THEN 'police' WHEN CONTAINS(UPPER(sector_text), 'PARKING ENF') THEN 'parking' ELSE 'other' END AS sector,
  TRY_CAST(fine_text AS DOUBLE) AS fine
 FROM coded
), flagged AS (
 SELECT *, COALESCE(SUBSTR(code, LENGTH(matched_code) + 1, 2) = '_W', FALSE) AS warning, COALESCE(matched_code IN (${earlier}), FALSE) AS earlier_number
 FROM parsed
), classed AS (
 SELECT citation, section, warning, earlier_number, sector, fine, sector_text, block, issued,
  MIN(COALESCE(section, '') || '|' || CAST(warning AS VARCHAR) || '|' || COALESCE(sector, '') || '|' || COALESCE(CAST(fine AS VARCHAR), '')) OVER (PARTITION BY citation, entered)
   <> MAX(COALESCE(section, '') || '|' || CAST(warning AS VARCHAR) || '|' || COALESCE(sector, '') || '|' || COALESCE(CAST(fine AS VARCHAR), '')) OVER (PARTITION BY citation, entered) AS conflicted
 FROM flagged
 QUALIFY ROW_NUMBER() OVER (PARTITION BY citation ORDER BY entered DESC NULLS LAST, written_code, sector_text, fine_text, block) = 1
)`;
}

const CODE_METHOD = `A violation code is read with its spaces removed and a leading SDMC dropped, so 86.0139(A), 86.0139 (A), 86.0139(A) SDMC and SDMC 86.0139(A) are one code; a code followed by _W is a warning and is counted apart from citations. 86.23(G) and 86.23(F), the numbers the publisher's older files write the 72-hour and vehicle-habitation sections under with the same descriptions, are counted under 86.0137(G) and 86.0137(F). 86.0139(B), 86.0118 and the other sections and subsections are not counted. Each citation number is counted once, from its latest entry by date_creation; where the latest entries share that time but differ in section, warning, issuing unit or fine as read, not as spelt, the citation is counted as conflicting, under no unit and in no fine total.`;
const UNIT_METHOD = 'The issuing unit is read from sector1: text containing POLICE is police, text containing PARKING ENF is parking enforcement, any other text is another City unit, and a blank is counted as not recorded.';

export function parkingQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  const columns = schema.columns.filter((column) => column.tableName === dataset.tableName).map((column) => column.columnName);
  if (!isParkingCitations(columns)) return [];
  const keys = Object.keys(SECTIONS) as Section[];
  const sectionRows = keys.map((key, index) => `SELECT '${key}' AS section, ${index + 1} AS ord`).join(' UNION ALL ');
  const counted = (condition: string) => `COUNT(c.citation) FILTER (WHERE NOT c.warning${condition})`;
  return [{
    id: 'parking_vehicle_sections', title: 'Oversized-vehicle, 72-hour and vehicle-habitation citations, by issuing unit',
    method: `Citations in ${dataset.displayName} under ${keys.map(label).join(', ')}, counted by the unit that issued them, with the fines written on them. ${CODE_METHOD} ${UNIT_METHOD} A fine that is blank or not a number is counted apart and left out of the total; a section with no readable fine has no total. vio_desc is not read.`,
    sql: `WITH ${citations(dataset, columns)}, sections AS (
 ${sectionRows}
), file AS (
 SELECT COUNT(*) AS all_citations, CAST(MIN(issued) AS VARCHAR) AS first_issue, CAST(MAX(issued) AS VARCHAR) AS last_issue FROM classed
)
SELECT sections.section, ${counted('')} AS citations, ${counted(' AND NOT c.conflicted AND c.sector = \'police\'')} AS by_police,
 ${counted(' AND NOT c.conflicted AND c.sector = \'parking\'')} AS by_parking, ${counted(' AND NOT c.conflicted AND c.sector = \'other\'')} AS other_sector,
 ${counted(' AND NOT c.conflicted AND c.sector IS NULL')} AS no_sector, ${counted(' AND c.conflicted')} AS conflicting,
 SUM(c.fine) FILTER (WHERE NOT c.warning AND NOT c.conflicted) AS fines, ${counted(' AND NOT c.conflicted AND c.fine IS NULL')} AS unread_fines,
 COUNT(c.citation) FILTER (WHERE c.warning) AS warnings, ${counted(' AND c.earlier_number')} AS earlier_number, ARRAY_TO_STRING(LIST_SORT(LIST(DISTINCT c.sector_text) FILTER (WHERE NOT c.warning)), '; ') AS sector_texts,
 file.all_citations, file.first_issue, file.last_issue
FROM sections LEFT JOIN classed AS c ON c.section = sections.section CROSS JOIN file
GROUP BY sections.section, sections.ord, file.all_citations, file.first_issue, file.last_issue ORDER BY sections.ord`,
  }, {
    id: 'parking_vehicle_blocks', title: 'Where oversized-vehicle and 72-hour citations were written',
    method: `Citations in ${dataset.displayName} under ${label('oversized')} or ${label('unmoved')}, counted by location as written, upper-cased and trimmed. ${CODE_METHOD} The ten locations with most citations are read, with the total under both sections and the citations with no location recorded. ${UNIT_METHOD}`,
    sql: `WITH ${citations(dataset, columns)}, counted AS (
 SELECT * FROM classed WHERE section IN ('oversized', 'unmoved') AND NOT warning
), totals AS (
 SELECT COUNT(*) AS section_citations, COUNT(*) FILTER (WHERE block IS NULL) AS no_block, CAST(MIN(issued) AS VARCHAR) AS first_issue, CAST(MAX(issued) AS VARCHAR) AS last_issue FROM counted
), blocks AS (
 SELECT block, COUNT(*) AS citations, COUNT(*) FILTER (WHERE section = 'oversized') AS oversized, COUNT(*) FILTER (WHERE section = 'unmoved') AS unmoved,
  COUNT(*) FILTER (WHERE NOT conflicted AND sector = 'police') AS by_police
 FROM counted WHERE block IS NOT NULL GROUP BY block
)
SELECT blocks.block, blocks.citations, blocks.oversized, blocks.unmoved, blocks.by_police, totals.section_citations, totals.no_block, totals.first_issue, totals.last_issue
FROM blocks CROSS JOIN totals ORDER BY blocks.citations DESC, blocks.block LIMIT 10`,
  }];
}

const count = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};
const shown = (value: number | null) => Math.round(value ?? 0).toLocaleString('en-US');
const dollars = (value: number) => `$${value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
const records = (result: SqlQueryResult) => result.rows.map((row) => Object.fromEntries(result.columns.map((name, i) => [name, row[i]])));
type Entry = { query: DiscoveryQuery; result?: SqlQueryResult };
/** A result cut short by the row limit would give short counts, so it gives none. */
const rowsOf = (results: Entry[], id: string) => {
  const entry = results.find((item) => item.query.id === id);
  return entry?.result?.rows.length && !entry.result.truncated ? { query: entry.query, rows: records(entry.result) } : null;
};
const span = (row: Record<string, unknown>) => row.first_issue != null && row.last_issue != null ? { text: ` from ${String(row.first_issue)} to ${String(row.last_issue)}`, period: `${String(row.first_issue)} to ${String(row.last_issue)}` } : null;
const CAVEAT = 'A parking citation records the vehicle and the place, not whether anyone lived in it. The fine is the amount written on the citation, not what was paid.';
const QUESTION = 'Were any of these vehicles, or others on the same blocks and dates, cited for vehicle habitation under SDMC 86.0137(F), and how many safe-parking spaces were open then? The police department\'s habitation citations and the safe parking program\'s nightly capacity for the period would show it.';

const earlierOf = (key: Section, row: Record<string, unknown> | undefined) => {
  const n = count(row?.earlier_number) ?? 0;
  return { n, note: n ? `, ${shown(n)} of them under the earlier number ${SECTIONS[key].earlier}` : '' };
};

function fineText(fines: number | null, unread: number, conflicting: number): string {
  if (fines != null) return `The fines written on them total ${dollars(fines)}${unread ? `; ${shown(unread)} have no readable fine and are not in that total` : ''}${conflicting ? '; the conflicting citations are not in it' : ''}.`;
  if (!unread) return 'No fine total is given: each of these citations is conflicting, and conflicting citations are in no total.';
  return `Their fines are not recorded in a readable form${conflicting ? ', and the conflicting citations are in no total' : ''}.`;
}

function described(key: Section, row: Record<string, unknown> | undefined): string {
  const n = count(row?.citations) ?? 0;
  const warnings = count(row?.warnings) ?? 0;
  const warned = warnings ? ` ${shown(warnings)} warnings under the same code are not counted as citations.` : '';
  const earlier = earlierOf(key, row);
  if (!row || !n) return `No citation under ${named(key, true)} is in the file.${warned}`;
  const conflicting = count(row.conflicting) ?? 0;
  return `${named(key, earlier.n > 0)}: ${shown(n)} citations${earlier.note}; ${shown(count(row.by_police))} written by police, ${shown(count(row.by_parking))} by parking enforcement, ${shown(count(row.other_sector))} by another City unit and ${shown(count(row.no_sector))} with no issuing unit recorded${conflicting ? `; ${shown(conflicting)} were entered at the same time with different sections, units or fines and are counted under no unit` : ''}.${typeof row.sector_texts === 'string' && row.sector_texts ? ` Issuing units as written: ${row.sector_texts}.` : ''} ${fineText(count(row.fines), count(row.unread_fines) ?? 0, conflicting)}${warned}`;
}

export function parkingLeads(results: Entry[]): ServiceLead[] {
  const leads: ServiceLead[] = [];
  const sections = rowsOf(results, 'parking_vehicle_sections');
  if (sections) {
    const row = (key: Section) => sections.rows.find((item) => item.section === key);
    const headline = (['oversized', 'unmoved'] as const).find((key) => (count(row(key)?.citations) ?? 0) >= CITATION_MINIMUM);
    if (headline) {
      const top = row(headline)!;
      const file = span(top);
      const habitation = count(row('habitation')?.citations) ?? 0;
      const habitationEarlier = earlierOf('habitation', row('habitation'));
      leads.push({ key: 'parking_vehicle_sections', priority: 96, query: sections.query,
        title: `${shown(count(top.citations))} citations under ${named(headline, earlierOf(headline, top).n > 0, true)}${file?.text ?? ''}, ${shown(count(top.by_police))} written by police`,
        detail: `The file holds ${shown(count(top.all_citations))} citation numbers${file?.text ?? ''}, each counted once. ${described('oversized', row('oversized'))} ${described('unmoved', row('unmoved'))} ${habitation
          ? `${shown(habitation)} citations under ${named('habitation', habitationEarlier.n > 0)} are in this file${habitationEarlier.note}, ${shown(count(row('habitation')?.by_police))} written by police.`
          : `No citation under ${named('habitation', true)} is in this file.`} ${CAVEAT}`,
        question: QUESTION,
        scope: file ? { period: file.period } : undefined,
      });
    }
  }
  const blocks = rowsOf(results, 'parking_vehicle_blocks');
  const [top] = blocks?.rows ?? [];
  if (blocks && (count(top.citations) ?? 0) >= CITATION_MINIMUM) {
    const dates = span(top);
    const noBlock = count(top.no_block) ?? 0;
    leads.push({ key: 'parking_vehicle_blocks', priority: 93, query: blocks.query,
      title: `${shown(count(top.citations))} of ${shown(count(top.section_citations))} oversized-vehicle and 72-hour citations${dates?.text ?? ''} were written at ${String(top.block)}, ${shown(count(top.by_police))} by police`,
      detail: `Citations under ${label('oversized')} or ${label('unmoved')}, by location as written: ${blocks.rows.map((row) => `${String(row.block)}: ${shown(count(row.citations))} (${shown(count(row.oversized))} oversized, ${shown(count(row.unmoved))} 72-hour; ${shown(count(row.by_police))} by police)`).join('; ')}.${noBlock ? ` ${shown(noBlock)} have no location recorded.` : ''} A location is the place as the officer wrote it, so one block can appear under more than one spelling. ${CAVEAT}`,
      question: QUESTION,
      scope: dates ? { period: dates.period } : undefined,
    });
  }
  return leads;
}
