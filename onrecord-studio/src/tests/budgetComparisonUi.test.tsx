// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import BudgetComparison from '../components/dashboard/BudgetComparison';
import type { AnalyticalSchema, AnalyticalDatasetSummary } from '../types';

afterEach(cleanup);
const dataset = (id: string): AnalyticalDatasetSummary => ({ snapshotId: id, tableName: `analysis_${id}`, datasetKind: 'REGIONAL_DATA', displayName: id, importedRows: 5, rejectedRows: 1, sourceUri: `https://example.test/${id}.csv`, sourceSha256: id.repeat(20), retrievedAt: '2026-09-13', observationPeriodStart: null, observationPeriodEnd: null, partialDataset: true, validationSummary: '{}', unconvertedByColumn: {} });
const schema: AnalyticalSchema = {
  datasets: [dataset('budget'), dataset('actuals')], templates: [],
  columns: ['budget', 'actuals'].flatMap(id => ['amount', 'report_fy', 'fund_number', 'funds_center_number', 'account_number', 'dept_name', ...(id === 'budget' ? ['budget_cycle'] : [])].map(columnName => ({ tableName: `analysis_${id}`, columnName, logicalType: columnName === 'amount' ? 'NUMBER' : 'CATEGORY', originalName: columnName, nullable: true, dictionary: null }))),
};

describe('budget comparison selection', () => {
  it('requires two compatible roles and passes explicit fiscal scope into the executable comparison', () => {
    const compare = vi.fn<(sql: string) => Promise<boolean>>(() => Promise.resolve(true));
    render(<BudgetComparison schema={schema} onCompare={compare} />);
    expect(screen.getByRole('button', { name: 'Compare budget and actuals' })).toHaveProperty('disabled', true);
    const actuals = screen.getByLabelText('Actuals snapshot') as HTMLSelectElement;
    expect([...actuals.options].map(o => o.value)).toEqual(['', 'actuals']);
    fireEvent.change(screen.getByLabelText('Budget snapshot'), { target: { value: 'budget' } });
    fireEvent.change(actuals, { target: { value: 'actuals' } });
    fireEvent.change(screen.getByLabelText('Comparison fiscal year'), { target: { value: '23' } });
    fireEvent.click(screen.getByRole('button', { name: 'Compare budget and actuals' }));
    expect(compare).toHaveBeenCalledOnce();
    expect(compare.mock.calls[0][0]).toContain("CAST(report_fy AS VARCHAR) = '23'");
    expect(screen.getAllByText(/Partial or unverified coverage/)).toHaveLength(2);
  });
});
