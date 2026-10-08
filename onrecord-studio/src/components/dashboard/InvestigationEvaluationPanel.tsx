import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { InvestigationEvaluation, PilotAssessment } from '../../types';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';

type CheckOutcome = 'Pass' | 'Fail' | 'Not recorded';

// A check whose step was never recorded did not fail a measurement; it was not measured.
function checkOutcome(check: { passed: boolean; detail: string }): CheckOutcome {
  if (check.passed) return 'Pass';
  const detail = check.detail.toLocaleLowerCase();
  return detail.includes('not recorded') || (detail.startsWith('no ') && detail.includes('recorded')) ? 'Not recorded' : 'Fail';
}

export default function InvestigationEvaluationPanel() {
  const [evaluations, setEvaluations] = useState<InvestigationEvaluation[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(true);
  const gate = useLatestRequestGate();
  useEffect(() => {
    const request = gate.begin(); setBusy(true);
    void invokeCommand('get_investigation_evaluations', {}).then(value => { if (gate.isCurrent(request)) setEvaluations(value); }).catch(reason => { if (gate.isCurrent(request)) setError(formatNativeError(reason)); }).finally(() => { if (gate.isCurrent(request)) setBusy(false); });
    return gate.invalidate;
  }, [gate]);
  const save = async (assessment: PilotAssessment) => {
    const request = gate.begin(); setBusy(true); setError('');
    try { const value = await invokeCommand('save_pilot_assessment', { assessment }); if (gate.isCurrent(request)) setEvaluations(value); }
    catch (reason) { if (gate.isCurrent(request)) setError(formatNativeError(reason)); }
    finally { if (gate.isCurrent(request)) setBusy(false); }
  };
  return <section aria-label="Investigative usefulness evaluation" className="space-y-4">
    <div>
      <h3 className="text-heading text-fg">Does this investigation help someone act?</h3>
      <p className="mt-1 max-w-prose text-body text-muted">Record what an editor, researcher or advocate actually used, which leads failed, and the work required. These checks don’t substitute for human judgment.</p>
    </div>
    {busy && evaluations.length === 0 && <Skeleton variant="lines" lines={3} label="Evaluating saved investigations" />}
    {error && <InlineError summary="Saved investigations couldn’t be evaluated." detail={error} />}
    {!busy && !error && evaluations.length === 0 && <p className="text-body text-muted">No saved investigation or reference case is available to evaluate yet.</p>}
    {evaluations.map(evaluation => <article key={evaluation.id} className="rounded-lg border border-line p-4">
      <h4 className="text-body font-medium text-fg">{evaluation.title}</h4>
      <ul className="mt-3 space-y-3">
        {evaluation.checks.map((check, i) => {
          const outcome = checkOutcome(check);
          return <li key={i}>
            <p className="flex items-center gap-2 text-body text-fg-secondary">
              <Badge tone={outcome === 'Pass' ? 'success' : outcome === 'Fail' ? 'danger' : 'neutral'} glyph={outcome === 'Pass' ? 'circle' : outcome === 'Fail' ? 'diamond' : 'ring'} dashed={outcome === 'Not recorded'}>{outcome}</Badge>
              {check.name}
            </p>
            <p className="mt-1 text-caption text-muted">{check.detail}</p>
          </li>;
        })}
      </ul>
      {evaluation.limitations.map(limit => <p key={limit} className="mt-3 text-caption text-warning">{limit}</p>)}
      {!evaluation.id.startsWith('reference-') && <>
        <div className="mt-4 space-y-3">
          {evaluation.assessments.length === 0 ? <p className="text-body text-muted">No human assessment recorded.</p> : evaluation.assessments.map((assessment, i) => <div key={i} className="rounded-md border border-line p-3 text-body text-fg-secondary">
            <p className="tabular-nums">{assessment.assessor}: usefulness {assessment.usefulness}/5 · {assessment.supportedDiscoveries} supported discover{assessment.supportedDiscoveries === 1 ? 'y' : 'ies'} · {assessment.falseLeads} false lead{assessment.falseLeads === 1 ? '' : 's'}</p>
            <p className="mt-1 tabular-nums">{assessment.minutesSpent} minutes spent · {timeAgainstBaseline(assessment.minutesSpent, assessment.baselineMinutes)}</p>
            <p className="mt-1">{assessment.resultUsed}</p>
            <p className="mt-1 text-muted">{assessment.notes}</p>
          </div>)}
        </div>
        <AssessmentForm reportId={evaluation.id} busy={busy} save={save} />
      </>}
    </article>)}
  </section>;
}

function timeAgainstBaseline(spent: number, baseline: number | null): string {
  if (baseline === null) return 'Time saved not estimated: no baseline';
  const difference = baseline - spent;
  if (difference === 0) return `Same time as the assessor’s ${baseline}-minute estimate`;
  return difference > 0
    ? `${difference} minutes saved against the assessor’s ${baseline}-minute estimate`
    : `${-difference} minutes more than the assessor’s ${baseline}-minute estimate`;
}

function AssessmentForm({ reportId, busy, save }: { reportId: string; busy: boolean; save: (assessment: PilotAssessment) => Promise<void> }) {
  const [assessor, setAssessor] = useState(''); const [usefulness, setUsefulness] = useState('');
  const [supported, setSupported] = useState(''); const [falseLeads, setFalseLeads] = useState('');
  const [minutes, setMinutes] = useState(''); const [baseline, setBaseline] = useState('');
  const [result, setResult] = useState(''); const [notes, setNotes] = useState('');
  const control = 'analysis-input mt-1 w-full';
  return <details className="mt-4"><summary className="focus-ring cursor-pointer rounded text-body text-accent">Record a human assessment</summary><form className="mt-3" onSubmit={event => { event.preventDefault(); void save({ reportId, assessor, usefulness: Number(usefulness), supportedDiscoveries: Number(supported), falseLeads: Number(falseLeads), minutesSpent: Number(minutes), baselineMinutes: baseline === '' ? null : Number(baseline), resultUsed: result, notes }); }}><fieldset disabled={busy} className="space-y-3"><div className="grid gap-3 md:grid-cols-3"><label className="text-body text-fg-secondary">Assessor<input required className={control} value={assessor} onChange={e => setAssessor(e.target.value)} /></label><label className="text-body text-fg-secondary">Usefulness<select required value={usefulness} className={control} onChange={e => setUsefulness(e.target.value)}><option value="">Choose after review</option><option value="1">1 — No useful result</option><option value="2">2 — Minor assistance</option><option value="3">3 — Useful lead</option><option value="4">4 — Substantial case progress</option><option value="5">5 — Actionable supported discovery</option></select></label>{[{ label: 'Supported discoveries', value: supported, set: setSupported }, { label: 'False leads', value: falseLeads, set: setFalseLeads }, { label: 'Minutes actually spent', value: minutes, set: setMinutes }, { label: 'Estimated baseline minutes (optional)', value: baseline, set: setBaseline }].map(field => <label key={field.label} className="text-body text-fg-secondary">{field.label}<input required={field.set !== setBaseline} type="number" min="0" max="100000" step="1" className={control} value={field.value} onChange={e => field.set(e.target.value)} /></label>)}</div><label className="block text-body text-fg-secondary">What did this enable you to do, or why was it unhelpful?<textarea required rows={2} className={control} value={result} onChange={e => setResult(e.target.value)} /></label><label className="block text-body text-fg-secondary">Evidence of usefulness and remaining problems<textarea rows={2} className={control} value={notes} onChange={e => setNotes(e.target.value)} /></label><Button type="submit" variant="primary">Save assessment</Button></fieldset></form></details>;
}
