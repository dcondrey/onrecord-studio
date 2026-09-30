import type { FinancialDataset, FinancialMapping, FinancialReconciliation, FinancialReport } from '../types/financial';

export function financialAmount(value: string | null | undefined, currency = ''): string {
  if (value == null) return 'Unknown';
  const match = /^(-?)(\d+)(?:\.(\d+))?$/.exec(value);
  if (!match) return value;
  return `${currency ? `${currency} ` : ''}${match[1]}${BigInt(match[2]).toLocaleString('en-US')}.${(match[3] ?? '').padEnd(2, '0')}`;
}
function scaled(value: string | null): bigint | null {
  if (value == null || !/^-?\d+(?:\.\d{1,4})?$/.test(value)) return null;
  const negative = value.startsWith('-');
  const [whole, fraction = ''] = value.replace(/^-/, '').split('.');
  const n = BigInt(whole) * 10000n + BigInt(fraction.padEnd(4, '0'));
  return negative ? -n : n;
}
export function rankFinancialLines(rows: FinancialReconciliation[]): FinancialReconciliation[] {
  const magnitude = (r: FinancialReconciliation) => { const n = scaled(r.actualMinusAdopted) ?? scaled(r.actual) ?? 0n; return n < 0n ? -n : n; };
  return [...rows].sort((a,b) => magnitude(a) > magnitude(b) ? -1 : magnitude(a) < magnitude(b) ? 1 : a.key.localeCompare(b.key));
}
const aliases: Record<string,string[]> = { account_label:['account_label','account_description','account_name'], fy:['report_fy','fiscal_year','fy'],fund:['fund_number','fund_id','fund'],account:['account_number','account_code','account'],department:['dept_name','department'],center:['funds_center_number','cost_center'],amount:['amount','actual_cost','total_cost'],basis:['budget_cycle','cost_basis','basis'],contract_id:['contract_id','contract_number'],vendor_id:['vendor_id','vendor_number'],transaction_id:['transaction_id','transaction_line_id'],invoice_id:['invoice_id','invoice_number'],amendment_id:['amendment_id','amendment_number'],program:['program','program_name'],quantity:['quantity','outcome_count'],denominator:['denominator','eligible_count'],status:['status','payment_status','execution_status'],transaction_date:['transaction_date','payment_date','posting_date'],approval:['approval','approval_status','approved_by'] };
export function suggestFinancialMapping(role: string, dataset: FinancialDataset, fields: string[]): FinancialMapping {
  const columns: Record<string,string> = {};
  for (const field of fields) {
    const names = aliases[field] ?? [field];
    const exact = names.find(name => dataset.columns.includes(name));
    if (exact) columns[field] = exact;
  }
  return {role,snapshotId:dataset.snapshotId,columns,currency:null,agency:null};
}
export function financialReportMarkdown(report: FinancialReport): string {
  const lines = [`# Financial investigation: FY${report.scope.fiscalYear}, fund ${report.scope.fund}`, '', `Generated ${report.createdAt}. These are investigative leads, not findings of wrongdoing.`, ''];
  for (const finding of report.findings) {
    lines.push(`## ${finding.title}`, '', finding.observation, '', ...finding.calculations.map(c=>`- ${c.label}: ${c.formula} = ${c.result} ${c.unit}`), '', ...finding.alternatives.map(a=>`- Alternative: ${a}`), '', `Next test: ${finding.nextTest}`, '');
    for (const id of finding.citations) {
      const source = report.evidence.find(e=>e.id===id); if (!source) continue;
      lines.push(`Source: ${source.sourceUrl ?? 'Local imported record'}; snapshot ${source.snapshotId}; SHA-256 ${source.sourceSha256}`, '', '```json', JSON.stringify({columns:source.columns,row:source.row}), '```', '', '```sql', report.queries[source.queryId] ?? 'Query unavailable', '```', '');
    }
  }
  lines.push('## Missing records', '', ...report.gaps.map(g=>`- ${g.title}: ${g.recordsNeeded} ${g.why}`), '', '## Limitations', '', ...report.limitations.map(l=>`- ${l}`));
  return lines.join('\n');
}
