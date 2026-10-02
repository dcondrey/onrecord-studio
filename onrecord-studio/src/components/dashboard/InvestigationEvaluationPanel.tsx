import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { InvestigationEvaluation, PilotAssessment } from '../../types';

export default function InvestigationEvaluationPanel() {
  const [evaluations, setEvaluations] = useState<InvestigationEvaluation[]>([]);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const gate = useLatestRequestGate();
  useEffect(() => {
    const request = gate.begin(); setBusy(true);
    void invokeCommand('get_investigation_evaluations', {}).then(value => { if (gate.isCurrent(request)) setEvaluations(value); }).catch(reason => { if (gate.isCurrent(request)) setError(String(reason)); }).finally(() => { if (gate.isCurrent(request)) setBusy(false); });
    return gate.invalidate;
  }, [gate]);
  const save = async (assessment: PilotAssessment) => {
    const request = gate.begin(); setBusy(true); setError('');
    try { const value = await invokeCommand('save_pilot_assessment', { assessment }); if (gate.isCurrent(request)) setEvaluations(value); }
    catch (reason) { if (gate.isCurrent(request)) setError(String(reason)); }
    finally { if (gate.isCurrent(request)) setBusy(false); }
  };
  return <section aria-label="Investigative usefulness evaluation" className="space-y-4"><h3 className="text-lg font-semibold text-fg">Does this investigation help someone act?</h3><p className="max-w-3xl text-sm text-muted">Reference cases replay citation checks against retained San Diego evidence. Record what an editor, nonprofit researcher or advocate actually used, which leads failed, and the work required. These checks do not substitute for human judgment.</p>{busy && <p role="status" className="text-sm text-muted">Evaluating saved investigations…</p>}{error && <p role="alert" className="text-sm text-danger">{error}</p>}{evaluations.map(evaluation => <article key={evaluation.id} className="rounded-xl border border-line-strong p-4"><h4 className="font-semibold text-fg">{evaluation.title}</h4><ul className="mt-3 space-y-2 text-label leading-4">{evaluation.checks.map((check,i) => <li key={i} className={check.passed ? 'text-fg-secondary' : 'text-danger'}><strong>{check.passed ? 'PASS' : 'FAIL'} · {check.name}</strong><p className="mt-1 text-muted">{check.detail}</p></li>)}</ul>{evaluation.limitations.map(limit => <p key={limit} className="mt-2 text-label leading-4 text-warning">{limit}</p>)}{!evaluation.id.startsWith('reference-') && <><div className="mt-4 space-y-3">{evaluation.assessments.length === 0 ? <p className="text-sm text-muted">No human assessment recorded.</p> : evaluation.assessments.map((assessment,i) => <div key={i} className="rounded-lg border border-line p-3 text-label leading-4 text-fg-secondary"><p>{assessment.assessor}: usefulness {assessment.usefulness}/5 · {assessment.supportedDiscoveries} supported discoveries · {assessment.falseLeads} false leads</p><p className="mt-1">{assessment.minutesSpent} minutes spent · {assessment.baselineMinutes === null ? 'Time saved not estimated: no baseline' : `${assessment.baselineMinutes - assessment.minutesSpent} minutes saved against the assessor’s ${assessment.baselineMinutes}-minute estimate`}</p><p className="mt-1">{assessment.resultUsed}</p><p className="mt-1 text-muted">{assessment.notes}</p></div>)}</div><AssessmentForm reportId={evaluation.id} busy={busy} save={save} /></>}</article>)}</section>;
}

function AssessmentForm({ reportId, busy, save }: { reportId: string; busy: boolean; save: (assessment: PilotAssessment) => Promise<void> }) {
  const [assessor, setAssessor] = useState(''); const [usefulness, setUsefulness] = useState('');
  const [supported, setSupported] = useState(''); const [falseLeads, setFalseLeads] = useState('');
  const [minutes, setMinutes] = useState(''); const [baseline, setBaseline] = useState('');
  const [result, setResult] = useState(''); const [notes, setNotes] = useState('');
  const control = 'analysis-input mt-1 w-full';
  return <details className="mt-4"><summary className="text-sm text-accent">Record a human assessment</summary><form className="mt-3" onSubmit={event => { event.preventDefault(); void save({ reportId, assessor, usefulness: Number(usefulness), supportedDiscoveries: Number(supported), falseLeads: Number(falseLeads), minutesSpent: Number(minutes), baselineMinutes: baseline === '' ? null : Number(baseline), resultUsed: result, notes }); }}><fieldset disabled={busy} className="space-y-3"><div className="grid gap-3 md:grid-cols-3"><label className="text-label leading-4 text-fg-secondary">Assessor<input required className={control} value={assessor} onChange={e => setAssessor(e.target.value)} /></label><label className="text-label leading-4 text-fg-secondary">Usefulness<select required value={usefulness} className={control} onChange={e => setUsefulness(e.target.value)}><option value="">Choose after review</option><option value="1">1 — No useful result</option><option value="2">2 — Minor assistance</option><option value="3">3 — Useful lead</option><option value="4">4 — Substantial case progress</option><option value="5">5 — Actionable supported discovery</option></select></label>{[{ label: 'Supported discoveries', value: supported, set: setSupported }, { label: 'False leads', value: falseLeads, set: setFalseLeads }, { label: 'Minutes actually spent', value: minutes, set: setMinutes }, { label: 'Estimated baseline minutes (optional)', value: baseline, set: setBaseline }].map(field => <label key={field.label} className="text-label leading-4 text-fg-secondary">{field.label}<input required={field.set !== setBaseline} type="number" min="0" max="100000" step="1" className={control} value={field.value} onChange={e => field.set(e.target.value)} /></label>)}</div><label className="block text-label leading-4 text-fg-secondary">What did this enable you to do, or why was it unhelpful?<textarea required rows={2} className={control} value={result} onChange={e => setResult(e.target.value)} /></label><label className="block text-label leading-4 text-fg-secondary">Evidence of usefulness and remaining problems<textarea rows={2} className={control} value={notes} onChange={e => setNotes(e.target.value)} /></label><button className="analysis-button">Save assessment</button></fieldset></form></details>;
}
