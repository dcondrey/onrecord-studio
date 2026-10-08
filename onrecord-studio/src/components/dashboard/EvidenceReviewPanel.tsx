import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { FindingReviewInput, PublicRelationship, ResearchReport, ReviewWorkspace } from '../../types';
import { reviewPacket } from '../../utils/review';
import { enumLabel } from '../objects/labels';

const control = 'analysis-input mt-1 w-full';
const button = 'analysis-button';

export default function EvidenceReviewPanel({ reportId }: { reportId: string }) {
  const [result, setWorkspace] = useState<ReviewWorkspace | null>(null);
  const workspace = result?.report.id === reportId ? result : null;
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [packet, setPacket] = useState('');
  const gate = useLatestRequestGate();
  useEffect(() => {
    const request = gate.begin(); setWorkspace(null); setError(''); setBusy(true); setPacket('');
    void invokeCommand('get_review_workspace', { reportId }).then(value => {
      if (!gate.isCurrent(request)) return;
      setWorkspace(value);
    }).catch(reason => {
      if (!gate.isCurrent(request)) return;
      setError(String(reason));
    }).finally(() => {
      if (!gate.isCurrent(request)) return;
      setBusy(false);
    });
    return gate.invalidate;
  }, [gate, reportId]);
  const mutate = async (work: () => Promise<ReviewWorkspace>) => {
    const request = gate.begin(); setBusy(true); setError(''); setPacket('');
    try {
      const value = await work();
      if (!gate.isCurrent(request)) return;
      setWorkspace(value);
    } catch (reason) {
      if (!gate.isCurrent(request)) return;
      setError(String(reason));
    } finally {
      if (gate.isCurrent(request)) setBusy(false);
    }
  };
  return <section aria-label="Evidence review" className="space-y-4">
    <div className="flex flex-wrap items-start justify-between gap-3"><div><h3 className="text-lg font-semibold text-fg">Challenge the case</h3><p className="mt-1 max-w-3xl text-sm text-muted">Recompute the quantities, test the explanation, and leave a review another investigator can follow. Reviewer names are local attribution, not authenticated identities.</p></div><button className={button} disabled={busy || !workspace} onClick={() => void mutate(() => invokeCommand('replay_review_evidence', { reportId }))}>Verify sources and replay calculations</button></div>
    {error && <p role="alert" className="text-sm text-danger">{error}</p>}
    {busy && <p role="status" className="text-sm text-muted">Checking investigation evidence…</p>}
    {workspace && <>
      {workspace.challenges.map(challenge => <article key={`${reportId}:${challenge.findingIndex}`} className="rounded-xl border border-line-strong p-4">
        <h4 className="font-semibold text-fg">{challenge.title}</h4>
        <p className="mt-2 text-sm text-fg-secondary">{workspace.report.synthesis.findings[challenge.findingIndex].interpretation}</p>
        <ul className="mt-3 list-disc space-y-1 pl-5 text-label leading-4 text-muted">{challenge.checks.map(check => <li key={check}>{check}</li>)}</ul>
        {challenge.blockers.length > 0 && <div className="mt-3 rounded-lg border border-warning/30 p-3 text-label leading-4 text-warning"><p className="font-semibold">Resolve before supporting this conclusion</p><ul className="mt-2 list-disc space-y-1 pl-5">{challenge.blockers.map(blocker => <li key={blocker}>{blocker}</li>)}</ul></div>}
        <details className="mt-3 text-sm"><summary className="text-accent">Source quotations and alternatives</summary><div className="mt-2 space-y-3">{workspace.report.synthesis.findings[challenge.findingIndex].citations.map((cite, index) => { const source = workspace.report.evidence.find(e => e.id === cite.evidenceId); return <blockquote key={index} className="border-l border-accent/60 pl-3 text-label leading-4 text-fg-secondary"><p className="whitespace-pre-wrap break-words">{cite.quote}</p>{source?.sourceUrl && <a className="mt-1 block text-accent underline" href={source.sourceUrl} target="_blank" rel="noreferrer">{source.label}</a>}</blockquote>; })}<ul className="list-disc pl-5 text-label leading-4 text-muted">{workspace.report.synthesis.findings[challenge.findingIndex].alternativeExplanations.map(value => <li key={value}>{value}</li>)}</ul></div></details>
        <FindingForm key={`${reportId}:${challenge.findingIndex}:${workspace.reviews.find(r => r.input.findingIndex === challenge.findingIndex)?.revision ?? 0}`} report={workspace.report} index={challenge.findingIndex} workspace={workspace} busy={busy} save={input => mutate(() => invokeCommand('save_finding_review', { input }))} />
      </article>)}
      <details className="rounded-xl border border-line-strong p-4"><summary className="font-semibold text-fg">Review history ({workspace.reviews.length} contributions)</summary><ol className="mt-3 space-y-3">{workspace.reviews.map(r => <li key={r.id} className="text-label leading-4 text-fg-secondary"><strong>{r.input.reviewer} · {r.input.verdict} · revision {r.revision}</strong><p>{r.createdAt} · Finding {r.input.findingIndex + 1}{r.input.assignedTo && ` · assigned to ${r.input.assignedTo}`}</p><p>{r.input.reasoning}</p></li>)}</ol></details>
      <section className="rounded-xl border border-line-strong p-4"><h4 className="font-semibold text-fg">Public roles, vendors, programs and disclosures</h4><p className="mt-2 text-sm text-muted">Record explicit relationships and their effective periods. Stable identifiers let reviewed records connect across investigations. A name match or documented connection does not establish misconduct.</p>
        {workspace.identityWarnings.map(w => <p className="mt-2 text-label leading-4 text-warning" key={w}>{w}</p>)}
        {workspace.connections.map((path, i) => <div key={i} className="mt-3 rounded-lg border border-accent/30 p-3 text-sm"><p className="font-medium text-accent">{path.subject} → {path.via} → {path.object}</p><p className="mt-1 text-fg-secondary">{path.explanation}</p>{path.limitations.map(l => <p key={l} className="mt-1 text-label leading-4 text-warning">{l}</p>)}</div>)}
        {workspace.relationships.map(relationship => <details key={`${relationship.id}:${relationship.revision}`} className="mt-3 rounded-lg border border-line p-3"><summary className="text-sm text-fg">{relationship.subject.name} → {relationship.object.name} · {relationship.status}</summary><RelationshipForm report={workspace.report} initial={relationship} busy={busy} save={value => mutate(() => invokeCommand('save_public_relationship', { relationship: value, expectedRevision: relationship.revision }))} /></details>)}
        <details className="mt-4"><summary className="text-sm text-accent">Add a cited public relationship</summary><RelationshipForm key={reportId} report={workspace.report} busy={busy} save={value => mutate(() => invokeCommand('save_public_relationship', { relationship: value, expectedRevision: 0 }))} /></details>
      </section>
      <button className={button} onClick={() => setPacket(reviewPacket(workspace))}>Prepare review packet</button>
      {packet && <label className="block text-sm text-fg-secondary">Review packet — select and copy<textarea readOnly rows={12} value={packet} className={`${control} font-mono text-label leading-4`} onFocus={event => event.target.select()} /></label>}
    </>}
  </section>;
}

function FindingForm({ report, index, workspace, busy, save }: { report: ResearchReport; index: number; workspace: ReviewWorkspace; busy: boolean; save: (value: FindingReviewInput) => Promise<void> }) {
  const latest = workspace.reviews.find(r => r.input.findingIndex === index);
  const [input, setInput] = useState<FindingReviewInput>(latest ? { ...latest.input, expectedRevision: latest.revision } : { reportId: report.id, findingIndex: index, expectedRevision: 0, reviewer: '', assignedTo: '', verdict: 'OPEN', reasoning: '', falsificationTest: report.synthesis.findings[index].nextTest, counterSearch: '', counterEvidence: [] });
  const [counterId, setCounterId] = useState('');
  const [quote, setQuote] = useState('');
  const field = (key: 'reviewer'|'assignedTo'|'reasoning'|'falsificationTest'|'counterSearch', label: string, multiline = false) => <label className="block text-label leading-4 text-fg-secondary">{label}{multiline ? <textarea required rows={2} className={control} value={input[key]} onChange={e => setInput({ ...input, [key]: e.target.value })} /> : <input required={key === 'reviewer'} className={control} value={input[key]} maxLength={120} onChange={e => setInput({ ...input, [key]: e.target.value })} />}</label>;
  return <form className="mt-4 space-y-3" onSubmit={e => { e.preventDefault(); void save(input); }}><fieldset disabled={busy} className="space-y-3"><div className="grid gap-3 md:grid-cols-3">{field('reviewer','Reviewer')}{field('assignedTo','Assign next review to')}<label className="text-label leading-4 text-fg-secondary">Disposition<select className={control} value={input.verdict} onChange={e => setInput({ ...input, verdict: e.target.value as FindingReviewInput['verdict'] })}><option value="OPEN">Open lead</option><option value="SUPPORTED">Supported after review</option><option value="CONTESTED">Contested</option><option value="REJECTED">Rejected</option></select></label></div>{field('reasoning','Reasoning and limits',true)}{field('falsificationTest','What result would disprove the interpretation?',true)}{field('counterSearch','Counterevidence search performed and result',true)}<details><summary className="text-label leading-4 text-accent">Attach counterevidence ({input.counterEvidence.length})</summary><label className="mt-2 block text-label leading-4 text-fg-secondary">Evidence<select value={counterId} className={control} onChange={e => { setCounterId(e.target.value); setQuote(''); }}><option value="">Choose retained evidence</option>{report.evidence.map(e => <option key={e.id} value={e.id}>{e.label}</option>)}</select></label>{counterId && <><pre className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap break-words text-label leading-4 text-muted">{report.evidence.find(e => e.id === counterId)?.text}</pre><label className="block text-label leading-4 text-fg-secondary">Exact quote or complete SQL row<textarea className={control} rows={2} value={quote} onChange={e => setQuote(e.target.value)} /></label><button type="button" className={`${button} mt-2`} disabled={quote.trim().length < 12} onClick={() => { setInput({ ...input, counterEvidence: [...input.counterEvidence, { evidenceId: counterId, quote }] }); setCounterId(''); setQuote(''); }}>Attach quotation</button></>}{input.counterEvidence.map((cite,i) => <div key={i} className="mt-2 text-label leading-4 text-muted">{cite.quote}<button type="button" className="focus-ring ml-2 rounded text-danger" aria-label={`Remove counterevidence quotation ${i + 1}`} onClick={() => setInput({ ...input, counterEvidence: input.counterEvidence.filter((_,j) => i !== j) })}>Remove</button></div>)}</details><button className={button} type="submit">Save review</button></fieldset></form>;
}

function RelationshipForm({ report, initial, busy, save }: { report: ResearchReport; initial?: PublicRelationship; busy: boolean; save: (value: PublicRelationship) => Promise<void> }) {
  const emptyEntity = { key: '', name: '', kind: 'AGENCY', jurisdiction: '' };
  const [value, setValue] = useState<PublicRelationship>(initial ?? { id: '', reportId: report.id, revision: 0, subject: { ...emptyEntity }, predicate: 'FUNDS', object: { ...emptyEntity }, validFrom: null, validTo: null, citations: [], status: 'CANDIDATE', reviewer: '', note: '' });
  const [evidenceId, setEvidenceId] = useState(value.citations[0]?.evidenceId ?? '');
  const [quote, setQuote] = useState(value.citations[0]?.quote ?? '');
  const entityFields = (side: 'subject'|'object') => <fieldset className="space-y-2 rounded-lg border border-line p-3"><legend className="text-label leading-4 text-muted">{side === 'subject' ? 'From' : 'To'}</legend>{(['name','key','jurisdiction'] as const).map(key => <label key={key} className="block text-label leading-4 text-fg-secondary">{key === 'key' ? 'Public registry or agency identifier' : key === 'name' ? 'Name as cited' : 'Jurisdiction / issuing agency'}<input required className={control} value={value[side][key]} onChange={e => setValue({ ...value, [side]: { ...value[side], [key]: e.target.value } })} /></label>)}<label className="block text-label leading-4 text-fg-secondary">Type<select className={control} value={value[side].kind} onChange={e => setValue({ ...value, [side]: { ...value[side], kind: e.target.value } })}>{['PUBLIC_OFFICIAL','PUBLIC_ROLE','AGENCY','VENDOR','CONTRACT','PROGRAM','DISCLOSURE'].map(kind => <option key={kind} value={kind}>{enumLabel(kind)}</option>)}</select></label></fieldset>;
  return <form className="mt-3" onSubmit={e => { e.preventDefault(); void save({ ...value, citations: [{ evidenceId, quote }] }); }}><fieldset disabled={busy} className="space-y-3"><div className="grid gap-3 md:grid-cols-2">{entityFields('subject')}{entityFields('object')}</div><div className="grid gap-3 md:grid-cols-3"><label className="text-label leading-4 text-fg-secondary">Relationship<select className={control} value={value.predicate} onChange={e => setValue({ ...value, predicate: e.target.value })}>{['HOLDS_PUBLIC_ROLE','AWARDED_CONTRACT','VENDOR_FOR','DISCLOSED_INTEREST','ALIAS_OF','FUNDS','DELIVERS'].map(p => <option key={p} value={p}>{enumLabel(p)}</option>)}</select></label>{(['validFrom','validTo'] as const).map(key => <label key={key} className="text-label leading-4 text-fg-secondary">{key === 'validFrom' ? 'Effective from' : 'Effective through'} (blank = unknown)<input type="date" className={control} value={value[key] ?? ''} onChange={e => setValue({ ...value, [key]: e.target.value || null })} /></label>)}</div><label className="block text-label leading-4 text-fg-secondary">Retained source<select required className={control} value={evidenceId} onChange={e => { setEvidenceId(e.target.value); setQuote(''); }}><option value="">Choose a source</option>{report.evidence.map(e => <option key={e.id} value={e.id}>{e.label}</option>)}</select></label>{evidenceId && <pre className="max-h-48 overflow-auto whitespace-pre-wrap break-words text-label leading-4 text-muted">{report.evidence.find(e => e.id === evidenceId)?.text}</pre>}<label className="block text-label leading-4 text-fg-secondary">Exact quotation establishing the relationship<textarea required rows={3} className={control} value={quote} onChange={e => setQuote(e.target.value)} /></label><div className="grid gap-3 md:grid-cols-2"><label className="text-label leading-4 text-fg-secondary">Reviewer<input required className={control} value={value.reviewer} onChange={e => setValue({ ...value, reviewer: e.target.value })} /></label><label className="text-label leading-4 text-fg-secondary">Review state<select className={control} value={value.status} onChange={e => setValue({ ...value, status: e.target.value })}><option value="CANDIDATE">Candidate</option><option value="CONFIRMED">Confirmed from cited public records</option><option value="REJECTED">Rejected</option></select></label></div><label className="block text-label leading-4 text-fg-secondary">Reasoning and identity limits<textarea required rows={2} className={control} value={value.note} onChange={e => setValue({ ...value, note: e.target.value })} /></label><button className={button}>Save relationship review</button></fieldset></form>;
}
