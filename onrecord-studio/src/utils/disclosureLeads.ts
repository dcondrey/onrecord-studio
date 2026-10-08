import type { AnalyticalDatasetSummary, AnalyticalSchema, SqlQueryResult } from '../types';
import { quoteIdentifier, type DiscoveryQuery } from './dataDiscovery';
import type { ServiceLead } from './serviceAccountability';
import { numberOrNull, wholeNumber as shown, records } from './leadRows';

/** A city's agency-report export from its disclosure filing portal: one row per report on FPPC
 * Form 801, 802, 803 or 806. Found by the export's column names. */
const REQUIRED = ['filing_date', 'filing_type', 'description', 'agency', 'official'];
/** Behested payments are described only with this many reports behind them. */
const REPORT_MINIMUM = 10;
/** An official is named only with this many reports. */
const OFFICIAL_MINIMUM = 3;
const FORMS: Record<string, string> = {
  'FPPC 801': 'payments to the agency (Form 801)',
  'FPPC 802': 'ticket distributions (Form 802)',
  'FPPC 803': 'behested payments (Form 803)',
  'FPPC 806': 'paid appointments (Form 806)',
};

export const isAgencyReportExport = (schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary) =>
  REQUIRED.every((name) => schema.columns.some((column) => column.tableName === dataset.tableName && column.columnName === name));

export function disclosureQueries(schema: AnalyticalSchema, dataset: AnalyticalDatasetSummary): DiscoveryQuery[] {
  if (!isAgencyReportExport(schema, dataset)) return [];
  // The type cell can carry a note ("FPPC 802; Not electronically filed."); the form is its first eight characters.
  const reports = `reports AS (
 SELECT SUBSTR(UPPER(TRIM(CAST(${quoteIdentifier('filing_type')} AS VARCHAR))), 1, 8) AS form,
  TRY_CAST(SUBSTR(CAST(${quoteIdentifier('filing_date')} AS VARCHAR), 1, 10) AS DATE) AS filed_on,
  NULLIF(TRIM(CAST(${quoteIdentifier('official')} AS VARCHAR)), '') AS official
 FROM ${quoteIdentifier(dataset.tableName)}
)`;
  const method = `Each row of ${dataset.displayName} is one report an agency or official filed. The form is read from the first eight characters of the filing type, and the date from the first ten characters of the filing date. A behested payment is a payment made to a third party at an official's request; the report is the official's own disclosure.`;
  return [{
    id: 'disclosure_reports_by_form', title: 'Agency reports filed, by form', method,
    sql: `WITH ${reports}
SELECT form, COUNT(*) AS reports, COUNT(*) FILTER (WHERE filed_on IS NULL) AS undated,
 CAST(MIN(filed_on) AS VARCHAR) AS first_filed, CAST(MAX(filed_on) AS VARCHAR) AS last_filed
FROM reports GROUP BY form ORDER BY reports DESC, form LIMIT 12`,
  }, {
    id: 'disclosure_behested_by_official', title: 'Behested payment reports, by the official named', method: `${method} A report that names no official is counted apart, never under a name.`,
    sql: `WITH ${reports}
SELECT official, COUNT(*) AS reports, CAST(MIN(filed_on) AS VARCHAR) AS first_filed, CAST(MAX(filed_on) AS VARCHAR) AS last_filed,
 SUM(COUNT(*)) OVER () AS all_reports, COUNT(*) FILTER (WHERE official IS NOT NULL) OVER () AS officials_named,
 SUM(COUNT(*)) FILTER (WHERE official IS NULL) OVER () AS unnamed_reports
FROM reports WHERE form = 'FPPC 803' GROUP BY official ORDER BY reports DESC, official LIMIT 13`,
  }];
}

const count = (value: unknown): number => numberOrNull(value) ?? 0;

export function disclosureLeads(results: { query: DiscoveryQuery; result?: SqlQueryResult }[]): ServiceLead[] {
  const byOfficial = results.find((item) => item.query.id === 'disclosure_behested_by_official');
  if (!byOfficial?.result?.rows.length) return [];
  const officials = records(byOfficial.result);
  const all = count(officials[0].all_reports);
  if (all < REPORT_MINIMUM) return [];
  const named = officials.filter((row) => row.official != null);
  const unnamed = count(officials[0].unnamed_reports);
  const dates = officials.flatMap((row) => [row.first_filed, row.last_filed]).filter((value): value is string => typeof value === 'string').sort();
  const period = dates.length ? ` filed ${dates[0]} to ${dates[dates.length - 1]}` : '';
  const listed = named.filter((row) => count(row.reports) >= OFFICIAL_MINIMUM).slice(0, 5).map((row) => `${String(row.official)} ${shown(count(row.reports))}`);
  const forms = results.find((item) => item.query.id === 'disclosure_reports_by_form')?.result;
  const other = (forms ? records(forms) : []).filter((row) => row.form !== 'FPPC 803' && FORMS[String(row.form)])
    .map((row) => `${shown(count(row.reports))} ${FORMS[String(row.form)]}`);
  const officialCount = count(officials[0].officials_named);
  return [{ key: 'disclosure_behested_by_official', priority: 88, query: byOfficial.query,
    title: `${shown(all)} behested payment reports${period}${officialCount ? ` name ${shown(officialCount)} ${officialCount === 1 ? 'official' : 'officials'}` : ''}`,
    detail: `${listed.length ? `Reports by the official named: ${listed.join('; ')}.` : 'No official is named on three or more reports.'}${unnamed ? ` ${shown(unnamed)} name no official.` : ''}${other.length ? ` The same export also holds ${other.join(', ')}.` : ''} A behested payment is a payment made to a third party, often a charity or a public event, at an official's request, and the official files the report. The export gives a short description of each report and no amount column; the payor, the payee and the amount are on the filed form. A count of reports says nothing about any decision the official made, and a larger count can reflect more complete reporting.`,
    question: 'Did any payor named on these reports have a contract, permit or other matter before the official in the year around the payment? The filed Form 803 names the payor and the amount; council dockets and contract awards show the matters.',
  }];
}
