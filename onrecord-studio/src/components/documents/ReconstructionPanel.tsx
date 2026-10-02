import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { ReconstructionCitation, ReconstructionFinding, ReconstructionReport } from '../../types';

export default function ReconstructionPanel({ documentId, portalKey }: { documentId: number; portalKey: string }) {
  const identity = `${portalKey}:${documentId}`;
  const gate = useLatestRequestGate();
  const [report, setReport] = useState<ReconstructionReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [contextQuote, setContextQuote] = useState('');
  const [fixedWidth, setFixedWidth] = useState(false);
  useEffect(() => {
    gate.invalidate();
    setReport(null); setError(null); setBusy(false); setContextQuote(''); setFixedWidth(false);
  }, [identity, gate]);

  async function compare() {
    const request = gate.begin();
    setBusy(true); setError(null); setReport(null);
    try {
      const result = await invokeCommand('reconstruct_document_context', { documentId, portalKey, options: { contextQuote: contextQuote.trim() || null, fixedWidthMasks: fixedWidth } });
      if (!gate.isCurrent(request)) return;
      setReport(result);
    } catch (reason) {
      if (!gate.isCurrent(request)) return;
      setError(String(reason));
    } finally {
      if (gate.isCurrent(request)) setBusy(false);
    }
  }

  return <section className="rounded-xl border border-highlight/30 bg-canvas/70 p-4 text-label leading-4">
    <div className="flex flex-wrap items-start justify-between gap-3"><div><h3 className="font-semibold text-fg">Connect redacted clues across records</h3><p className="mt-1 max-w-2xl text-muted">Compare public role holders, complementary number fragments, and unredacted versions of the same passage. Uses retained text even after originals are released.</p></div><button type="button" onClick={() => void compare()} disabled={busy} className="rounded-lg border border-highlight/30 bg-highlight/20 px-3 py-2 font-semibold text-highlight disabled:opacity-50">{busy ? 'Comparing records…' : 'Compare related records'}</button></div>
    <details className="mt-3 text-muted"><summary className="cursor-pointer">Additional context and mask convention</summary><label className="mt-2 block">Optional exact quotation containing the public role or record context<textarea disabled={busy} value={contextQuote} maxLength={1000} onChange={(e) => setContextQuote(e.target.value)} className="mt-1 min-h-16 w-full rounded border border-control bg-surface p-2 text-fg" placeholder="Copy a title, signature or record key from this document’s extracted text." /></label><label className="mt-2 flex items-start gap-2"><input disabled={busy} type="checkbox" checked={fixedWidth} onChange={(e) => setFixedWidth(e.target.checked)} /><span>I have verified that each X, *, ?, or block in these number masks represents exactly one character. Otherwise, mask lengths remain unknown.</span></label></details>
    {error && <p role="alert" className="mt-3 text-danger">{error}</p>}
    {report && <div className="mt-4 space-y-3"><p className="text-muted">Compared {report.documentsCompared} records across {report.passagesCompared} passages. Results are evidence-backed inferences; they do not alter the original record.</p>{report.findings.length === 0 && <p className="rounded border border-line-strong p-3 text-fg-secondary">No supported reconstruction target was detected in the retained text. Image-only masks may need OCR; add related releases, dated staff directories or transaction registers to expand the evidence.</p>}<div className="max-h-[32rem] space-y-3 overflow-y-auto">{report.findings.map((finding) => <Finding key={finding.id} finding={finding} />)}</div><details className="text-muted"><summary className="cursor-pointer">Coverage and search trail</summary><ul className="mt-2 list-disc space-y-1 pl-5">{report.limitations.map((item) => <li key={item}>{item}</li>)}</ul><p className="mt-2">Search clues: {report.searchTerms.join(' · ') || 'No searchable clues'}</p></details></div>}
  </section>;
}

function Finding({ finding }: { finding: ReconstructionFinding }) {
  return <article className="rounded-lg border border-line-strong bg-surface/70 p-3"><div className="flex items-start justify-between gap-3"><h4 className="font-semibold text-fg">{finding.subject}</h4><span className="rounded bg-warning/10 px-2 py-1 text-label uppercase text-warning">{finding.status}</span></div><p className="mt-2 text-muted">{finding.explanation}</p><Citation citation={finding.observation} />{finding.candidates.map((candidate, index) => <div key={`${candidate.value}:${index}`} className="mt-3 rounded border border-line-strong p-3"><p className={candidate.contradicted ? 'font-semibold text-danger' : 'font-semibold text-highlight'}>{candidate.value}{candidate.contradicted && ' — contradicted by the constraints'}</p><ul className="mt-2 list-disc space-y-1 pl-4 text-fg-secondary">{candidate.supports.map((support) => <li key={support}>{support}</li>)}</ul>{candidate.limitations.map((limit) => <p key={limit} className="mt-2 text-warning">{limit}</p>)}{candidate.evidence.map((citation) => <Citation key={citation.id} citation={citation} />)}</div>)}<p className="mt-3 text-fg-secondary">Next step: {finding.nextStep}</p><details className="mt-2 text-muted"><summary className="cursor-pointer">Records request to resolve this clue</summary><textarea aria-label="Reconstruction records request draft" readOnly value={finding.requestDraft} onFocus={(event) => event.currentTarget.select()} className="mt-2 min-h-32 w-full rounded border border-control bg-canvas p-2 text-fg" /></details></article>;
}

function Citation({ citation }: { citation: ReconstructionCitation }) {
  const safeUrl = citation.sourceUrl && /^https?:\/\//i.test(citation.sourceUrl) ? citation.sourceUrl : null;
  return <blockquote className="mt-2 border-l-2 border-line-strong pl-3 text-muted"><p className="whitespace-pre-wrap">{citation.quote}</p><footer className="mt-1 text-label">{safeUrl ? <a href={safeUrl} target="_blank" rel="noreferrer" className="text-highlight underline">{citation.title}</a> : citation.title} · character {citation.characterOffset}<details className="mt-1"><summary className="cursor-pointer">Source identity</summary><p className="break-all">SHA-256: {citation.sourceSha256 || 'Not recorded'}</p><p className="break-all">Extraction: {citation.extractionSha256 || 'Not recorded'}</p></details></footer></blockquote>;
}
