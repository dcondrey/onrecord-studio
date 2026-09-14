import { useState } from 'react';
import type { AnalyticalSchema } from '../../types';
import { budgetComparisonCandidates, buildBudgetComparison, type BudgetComparisonSelection } from '../../utils/budgetComparison';

export default function BudgetComparison({ schema, onCompare }: { schema: AnalyticalSchema; onCompare: (sql: string) => void }) {
  const [selection, setSelection] = useState<BudgetComparisonSelection>({ budgetSnapshotId: '', actualsSnapshotId: '', fiscalYear: '', cycle: 'adopted', accountClass: 'expense' });
  const [error, setError] = useState('');
  const budgets = budgetComparisonCandidates(schema, 'budget');
  const actuals = budgetComparisonCandidates(schema, 'actuals');
  const update = (patch: Partial<BudgetComparisonSelection>) => { setSelection({ ...selection, ...patch }); setError(''); };
  const chosen = schema.datasets.filter((d) => [selection.budgetSnapshotId, selection.actualsSnapshotId].includes(d.snapshotId));
  return <section aria-label="Budget versus actuals" className="space-y-3">
    <p className="text-xs leading-relaxed text-slate-400">Compare City operating allocations with reported actuals for one fiscal-year code. Each side is summed by year, fund, funds center, and account before matching. Missing counterparts and missing amounts remain visible.</p>
    <div className="grid gap-2 sm:grid-cols-2">
      {(['budget', 'actuals'] as const).map((role) => <label key={role} className="text-xs text-slate-300">
        {role === 'budget' ? 'Budget snapshot' : 'Actuals snapshot'}
        <select aria-label={`${role === 'budget' ? 'Budget' : 'Actuals'} snapshot`} className="analysis-input mt-1 w-full" value={role === 'budget' ? selection.budgetSnapshotId : selection.actualsSnapshotId} onChange={(event) => update(role === 'budget' ? { budgetSnapshotId: event.target.value } : { actualsSnapshotId: event.target.value })}>
          <option value="">Choose an imported snapshot</option>
          {(role === 'budget' ? budgets : actuals).map((d) => <option key={d.snapshotId} value={d.snapshotId}>{d.displayName} · {d.sourceSha256.slice(0, 8)}</option>)}
        </select>
      </label>)}
      <label className="text-xs text-slate-300">Fiscal-year code<input aria-label="Comparison fiscal year" className="analysis-input mt-1 w-full" value={selection.fiscalYear} placeholder="25 or 2025 — use the file’s code" onChange={(event) => update({ fiscalYear: event.target.value })} /></label>
      <label className="text-xs text-slate-300">Fund number (optional)<input aria-label="Comparison fund" className="analysis-input mt-1 w-full" value={selection.fundNumber ?? ''} placeholder="Exact fund code" onChange={(event) => update({ fundNumber: event.target.value })} /></label>
      <select aria-label="Comparison budget cycle" className="analysis-input" value={selection.cycle} onChange={(event) => update({ cycle: event.target.value as BudgetComparisonSelection['cycle'] })}><option value="adopted">Adopted allocations</option><option value="proposed">Proposed allocations</option></select>
      <select aria-label="Comparison account class" className="analysis-input" value={selection.accountClass} onChange={(event) => update({ accountClass: event.target.value as BudgetComparisonSelection['accountClass'] })}><option value="expense">Expense accounts (5…)</option><option value="revenue">Revenue accounts (4…)</option></select>
    </div>
    {chosen.map((d) => <p key={d.snapshotId} className="break-all text-xs text-slate-400">{d.displayName}: {d.sourceUri ?? 'Source URL not recorded'} · {d.importedRows.toLocaleString()} rows · {d.rejectedRows.toLocaleString()} rejected · {d.partialDataset ? 'Partial or unverified coverage' : 'Declared complete'}</p>)}
    {(!budgets.length || !actuals.length) && <p className="text-xs text-amber-200">Import the City operating-budget and operating-actuals CSVs first. The City budget transparency source includes their Finance collection.</p>}
    <p className="text-xs text-slate-400">Confirm both snapshots cover the same period and accounting scope. Positive variance means actuals exceed the selected allocation. This comparison does not establish program effectiveness or reconcile interagency transfers.</p>
    <button type="button" className="analysis-button" disabled={!selection.budgetSnapshotId || !selection.actualsSnapshotId || !selection.fiscalYear.trim()} onClick={() => {
      try { onCompare(buildBudgetComparison(schema, selection)); setError(''); } catch (reason) { setError(String(reason)); }
    }}>Compare budget and actuals</button>
    {error && <p role="alert" className="text-xs text-rose-300">{error}</p>}
  </section>;
}
