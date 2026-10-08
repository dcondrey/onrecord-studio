import { useState } from 'react';
import { formatNativeError } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AnalyticalSchema } from '../../types';
import { budgetComparisonCandidates, buildBudgetComparison, type BudgetComparisonSelection } from '../../utils/budgetComparison';
import InlineError from '../ui/InlineError';

/**
 * `onCompare` runs the built query and resolves true once its result is on screen, false when a
 * newer query superseded it; a rejection is this comparison's failure, reported here beside the
 * button with a retry that runs the same comparison again.
 */
export default function BudgetComparison({ schema, onCompare }: { schema: AnalyticalSchema; onCompare: (sql: string) => Promise<boolean> }) {
  const [selection, setSelection] = useState<BudgetComparisonSelection>({ budgetSnapshotId: '', actualsSnapshotId: '', fiscalYear: '', cycle: 'adopted', accountClass: 'expense' });
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<{ query: string; message: string } | null>(null);
  const gate = useLatestRequestGate();
  const compare = (query: string) => {
    const request = gate.begin();
    setBusy(true);
    setFailure(null);
    onCompare(query)
      .catch((reason: unknown) => { if (gate.isCurrent(request)) setFailure({ query, message: formatNativeError(reason) }); })
      .finally(() => { if (gate.isCurrent(request)) setBusy(false); });
  };
  const budgets = budgetComparisonCandidates(schema, 'budget');
  const actuals = budgetComparisonCandidates(schema, 'actuals');
  // With one imported snapshot on a side there is nothing to choose, so it stands chosen.
  const chosenBudget = selection.budgetSnapshotId || (budgets.length === 1 ? budgets[0].snapshotId : '');
  const chosenActuals = selection.actualsSnapshotId || (actuals.length === 1 ? actuals[0].snapshotId : '');
  const effective = { ...selection, budgetSnapshotId: chosenBudget, actualsSnapshotId: chosenActuals };
  const update = (patch: Partial<BudgetComparisonSelection>) => { setSelection({ ...selection, ...patch }); setError(''); setFailure(null); };
  const chosen = schema.datasets.filter((d) => [chosenBudget, chosenActuals].includes(d.snapshotId));
  // The panel around it is the "Budget versus actuals" region; a second region of that name would repeat it.
  return <div className="space-y-3">
    <p className="text-label leading-relaxed text-muted">Compare City operating allocations with reported actuals for one fiscal-year code. Each side is summed by year, fund, funds center, and account before matching. Missing counterparts and missing amounts remain visible.</p>
    <div className="grid gap-2 sm:grid-cols-2">
      {(['budget', 'actuals'] as const).map((role) => <label key={role} className="text-label leading-4 text-fg-secondary">
        {role === 'budget' ? 'Budget snapshot' : 'Actuals snapshot'}
        <select aria-label={`${role === 'budget' ? 'Budget' : 'Actuals'} snapshot`} className="analysis-input mt-1 w-full" value={role === 'budget' ? chosenBudget : chosenActuals} onChange={(event) => update(role === 'budget' ? { budgetSnapshotId: event.target.value } : { actualsSnapshotId: event.target.value })}>
          <option value="">Choose an imported snapshot</option>
          {(role === 'budget' ? budgets : actuals).map((d) => <option key={d.snapshotId} value={d.snapshotId}>{d.displayName} · {d.sourceSha256.slice(0, 8)}</option>)}
        </select>
      </label>)}
      <label className="text-label leading-4 text-fg-secondary">Fiscal-year code<input aria-label="Comparison fiscal year" className="analysis-input mt-1 w-full" value={selection.fiscalYear} placeholder="25 or 2025 — use the file’s code" onChange={(event) => update({ fiscalYear: event.target.value })} /></label>
      <label className="text-label leading-4 text-fg-secondary">Fund number (optional)<input aria-label="Comparison fund" className="analysis-input mt-1 w-full" value={selection.fundNumber ?? ''} placeholder="Exact fund code" onChange={(event) => update({ fundNumber: event.target.value })} /></label>
      <select aria-label="Comparison budget cycle" className="analysis-input" value={selection.cycle} onChange={(event) => update({ cycle: event.target.value as BudgetComparisonSelection['cycle'] })}><option value="adopted">Adopted allocations</option><option value="proposed">Proposed allocations</option></select>
      <select aria-label="Comparison account class" className="analysis-input" value={selection.accountClass} onChange={(event) => update({ accountClass: event.target.value as BudgetComparisonSelection['accountClass'] })}><option value="expense">Expense accounts (5…)</option><option value="revenue">Revenue accounts (4…)</option></select>
    </div>
    {chosen.map((d) => <p key={d.snapshotId} className="break-all text-label leading-4 text-muted">{d.displayName}: {d.sourceUri ?? 'Source URL not recorded'} · {d.importedRows.toLocaleString()} rows · {d.rejectedRows.toLocaleString()} rejected · {d.partialDataset ? 'Partial or unverified coverage' : 'Declared complete'}</p>)}
    {(!budgets.length || !actuals.length) && <p className="text-label leading-4 text-warning">Import the City operating-budget and operating-actuals CSVs first. The City budget transparency source includes their Finance collection.</p>}
    <p className="text-label leading-4 text-muted">Confirm both snapshots cover the same period and accounting scope. Positive variance means actuals exceed the selected allocation. This comparison does not establish program effectiveness or reconcile interagency transfers.</p>
    <button type="button" className="analysis-button" disabled={busy || !chosenBudget || !chosenActuals || !selection.fiscalYear.trim()} onClick={() => {
      let query: string;
      try { query = buildBudgetComparison(schema, effective); } catch (reason) { setError(String(reason)); return; }
      setError('');
      compare(query);
    }}>{busy ? 'Comparing…' : 'Compare budget and actuals'}</button>
    {busy && <p role="status" className="text-label leading-4 text-muted">Comparing budget and actuals. The result opens in the SQL analysis panel below.</p>}
    {error && <p role="alert" className="text-label leading-4 text-danger">{error}</p>}
    {failure && <InlineError summary="The comparison couldn’t be run." detail={failure.message} onRetry={() => compare(failure.query)} />}
  </div>;
}
