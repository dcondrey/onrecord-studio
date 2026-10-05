import type { OrganisationFilings, OrganisationReturn } from '../types';

/** One organisation's row in the portfolio table: what its latest read return, its audits and the
 * library's tables state, with absence kept apart from No and zero. */
export interface PortfolioRow {
  ein: string;
  name: string;
  checkedAt: string;
  /** Tax period (YYYY-MM) of the latest return with a read statement, else null. */
  latestPeriod: string | null;
  totalRevenue: number | null;
  governmentGrants: number | null;
  significantDiversion: 'Yes' | 'No' | 'no answer' | 'not read';
  formerListed: 'Yes' | 'No' | 'no answer' | 'not read';
  severance: 'Yes' | 'No' | 'no answer' | 'no Schedule J' | 'not read';
  scheduleLRows: number | null;
  /** Returns listed by the IRS indexes that could not be read, by return type. */
  unread: string[];
  audits: number | null;
  latestAuditYear: string | null;
  auditFindingRows: number | null;
  /** Sum of the matched payment groups when every group has a readable sum; null when none matched or any could not be summed. */
  paymentsTotal: number | null;
  paymentRows: number;
}

const answer = (value: boolean | null): 'Yes' | 'No' | 'no answer' => value === null ? 'no answer' : value ? 'Yes' : 'No';
const period = (value: string) => value.length === 6 ? `${value.slice(0, 4)}-${value.slice(4)}` : value;

const csvCell = (value: string | number | null) => {
  if (value === null) return '';
  const text = String(value);
  return /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
};

/** The portfolio as CSV: one row per organisation, absent values left empty, never 0 or No. */
export function portfolioCsv(rows: PortfolioRow[]): string {
  const head = ['ein', 'organisation', 'checked_at', 'latest_return_period', 'total_revenue', 'government_grants', 'diversion_of_assets_part_vi_5', 'former_listed_part_vii_3', 'severance_schedule_j_4a', 'schedule_l_rows', 'returns_not_read', 'single_audits', 'latest_audit_year', 'audit_finding_rows', 'payments_total', 'payment_rows'];
  const lines = rows.map((row) => [row.ein, row.name, row.checkedAt, row.latestPeriod, row.totalRevenue, row.governmentGrants, row.significantDiversion, row.formerListed, row.severance, row.scheduleLRows, row.unread.join('; '), row.audits, row.latestAuditYear, row.auditFindingRows, row.paymentsTotal, row.paymentRows].map(csvCell).join(','));
  return [head.join(','), ...lines].join('\r\n') + '\r\n';
}

export function latestReadReturn(returns: OrganisationReturn[]): OrganisationReturn | undefined {
  return [...returns].filter((r) => r.statement).sort((a, b) => b.taxPeriod.localeCompare(a.taxPeriod))[0];
}

export function portfolioRow(filings: OrganisationFilings): PortfolioRow {
  const latest = latestReadReturn(filings.returns);
  const statement = latest?.statement ?? null;
  const audits = filings.singleAudits?.audits ?? null;
  const sums = filings.publicMoney.map((line) => line.amount === null ? null : Number(line.amount));
  const summable = sums.length > 0 && sums.every((value) => value !== null && Number.isFinite(value));
  return {
    ein: filings.ein,
    name: filings.name ?? filings.ein,
    checkedAt: filings.checkedAt,
    latestPeriod: latest ? period(latest.taxPeriod) : null,
    totalRevenue: statement?.totalRevenue ?? null,
    governmentGrants: statement?.governmentGrants ?? null,
    significantDiversion: statement ? answer(statement.significantDiversionReported) : 'not read',
    formerListed: statement ? answer(statement.formerListed) : 'not read',
    severance: statement ? (statement.scheduleJAttached ? answer(statement.severancePaymentReported) : 'no Schedule J') : 'not read',
    scheduleLRows: statement ? statement.scheduleLTransactions.length : null,
    unread: filings.returns.filter((r) => !r.statement).map((r) => `${r.returnType} ${period(r.taxPeriod)}`),
    audits: audits ? audits.length : null,
    latestAuditYear: audits?.map((audit) => audit.auditYear).filter((year): year is string => year !== null).sort().at(-1) ?? null,
    auditFindingRows: audits ? audits.reduce((count, audit) => count + audit.findings.length, 0) : null,
    paymentsTotal: summable ? sums.reduce<number>((total, value) => total + (value as number), 0) : null,
    paymentRows: filings.publicMoney.reduce((count, line) => count + line.rows, 0),
  };
}
