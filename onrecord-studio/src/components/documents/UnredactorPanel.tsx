import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { EvidenceBundle, EvidenceClass, RedactionIntegrityReport, RedactionPageFinding } from '../../types';
import { cn } from '../../utils/cn';

export default function UnredactorPanel({ documentId, portalKey }: { documentId: number; portalKey: string }) {
  // NOTE(documents): Do not display speculative candidates beside preserved text.
  const [isScanning, setIsScanning] = useState(false);
  const [report, setReport] = useState<RedactionIntegrityReport | null>(null);
  const [evidence, setEvidence] = useState<EvidenceBundle | null>(null);
  const [evidenceClass, setEvidenceClass] = useState<EvidenceClass>('preserved');
  const [error, setError] = useState<string | null>(null);
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const reviewStorageKey = `onrecord:redaction-review:${portalKey}:${documentId}`;
  const [reviewNotes, setReviewNotes] = useState<Record<string, string>>(() => readReviewNotes(reviewStorageKey));
  const scanGeneration = useRef(0);
  const coverageLimited = report !== null && !report.coverageComplete;

  useEffect(() => {
    scanGeneration.current += 1;
    setIsScanning(false);
    setReport(null);
    setEvidence(null);
    setError(null);
    setElapsedSeconds(0);
    setReviewNotes(readReviewNotes(reviewStorageKey));
  }, [documentId, portalKey]);

  useEffect(() => {
    if (!isScanning) return;
    const started = Date.now();
    const timer = window.setInterval(() => setElapsedSeconds(Math.floor((Date.now() - started) / 1_000)), 250);
    return () => window.clearInterval(timer);
  }, [isScanning]);

  const scan = async () => {
    const generation = ++scanGeneration.current;
    setIsScanning(true);
    setReport(null);
    setEvidence(null);
    setError(null);
    setElapsedSeconds(0);
    try {
      const [result, bundle] = await Promise.all([
        invokeCommand('verify_document_redaction_integrity', { documentId, portalKey }),
        invokeCommand('audit_document_residual_evidence', { documentId, portalKey, includeHypotheses: true }),
      ]);
      if (generation === scanGeneration.current) {
        setReport(result);
        setEvidence(bundle);
        setEvidenceClass(firstPopulatedEvidenceClass(bundle));
      }
    } catch (reason) {
      if (generation === scanGeneration.current) setError(String(reason));
    } finally {
      if (generation === scanGeneration.current) setIsScanning(false);
    }
  };

  return (
    <section className="flex max-h-96 flex-col overflow-hidden rounded-xl border border-slate-800 bg-slate-900/20">
      <div className="flex items-center justify-between border-b border-slate-800 px-4 py-3">
        <div>
          <div className="text-[10px] font-bold uppercase tracking-[0.18em] text-slate-500">PDF overlay integrity</div>
          <div className="mt-0.5 text-[9px] text-slate-600">Conservative vector, text-layer, and metadata inspection; permanently removed content cannot be recovered</div>
        </div>
        <button type="button" onClick={() => void scan()} disabled={isScanning} className="rounded-lg border border-indigo-400/25 bg-indigo-500/10 px-3 py-2 text-[10px] font-semibold text-indigo-300 transition hover:bg-indigo-500/15 disabled:cursor-not-allowed disabled:opacity-50">
          {isScanning ? `Inspecting… ${elapsedSeconds}s` : 'Run integrity scan'}
        </button>
      </div>
      {isScanning && <div className="h-1 overflow-hidden bg-slate-950" role="progressbar" aria-label="Redaction evidence collectors are running"><div className="h-full w-1/3 animate-pulse rounded-full bg-indigo-400" /></div>}
      <div className="min-h-0 flex-1 overflow-y-auto p-4">
        {error && <div role="alert" className="rounded-lg border border-rose-500/20 bg-rose-500/10 p-3 text-[10px] text-rose-300">{error}</div>}
        {report ? (
          <div className="space-y-3">
            <div className={cn('rounded-lg border p-3 text-[10px] font-medium', report.requiresManualReview ? 'border-amber-500/25 bg-amber-500/10 text-amber-200' : 'border-emerald-500/20 bg-emerald-500/10 text-emerald-200')}>
              {report.isLeakyRedaction
                ? 'Positioned text intersects a dark mask or unapplied redaction annotation and requires manual review.'
                : coverageLimited
                  ? 'The bounded pass found no qualifying intersection, but some page geometry could not be evaluated conclusively.'
                  : report.requiresManualReview
                    ? 'Unapplied redaction annotations remain in the PDF and require manual review.'
                  : 'No qualifying mask/text intersection was detected by this bounded structural pass.'}
            </div>
            <dl className="grid grid-cols-2 gap-3 rounded-lg border border-slate-800 bg-slate-950/50 p-3 text-[9px]">
              <Metric label="Pages analyzed" value={report.scannedPageCount.toLocaleString()} />
              <Metric label="Potential clashes" value={report.boundingBoxClashesCount.toLocaleString()} />
              <Metric label="Text redaction density" value={`${(report.calculatedRedactionRatio * 100).toFixed(2)}%`} />
              <Metric label="Metadata properties" value={report.metadataLeaks.length.toLocaleString()} />
              <Metric label="Structural signals" value={report.isolatedStructuralAnomalies.length.toLocaleString()} />
              <Metric label="Analysis profile" value={report.analysisVersion} />
              <Metric label="Source SHA-256" value={`${report.sourceSha256.slice(0, 16)}…`} title={report.sourceSha256} />
            </dl>
            {report.extractedHiddenText && (
              <div>
                <div className="mb-1.5 text-[9px] font-semibold uppercase tracking-wider text-slate-600">Quarantined PII-masked intersecting text fragments</div>
                <div className="max-h-48 overflow-y-auto whitespace-pre-wrap rounded-lg border border-slate-800 bg-slate-950 p-3 font-mono text-[10px] leading-relaxed text-slate-400">{report.extractedHiddenText}</div>
              </div>
            )}
            {report.metadataLeaks.length > 0 && <ForensicList title="PII-masked metadata" values={report.metadataLeaks} />}
            {evidence && <EvidenceTabs bundle={evidence} activeClass={evidenceClass} onSelect={setEvidenceClass} />}
            {report.pageFindings.length > 0 && <PageFindings findings={report.pageFindings} notes={reviewNotes} onNoteChange={(key, value) => {
              const next = { ...reviewNotes, [key]: value };
              if (!value.trim()) delete next[key];
              setReviewNotes(next);
              persistReviewNotes(reviewStorageKey, next);
            }} />}
            {report.isolatedStructuralAnomalies.length > 0 && <ForensicList title="Structural signals" values={report.isolatedStructuralAnomalies} />}
          </div>
        ) : !error && (
          <div className="grid min-h-40 place-items-center text-center text-[10px] leading-relaxed text-slate-600">Run the bounded integrity scan to inspect potential visual-mask and live-text co-location.</div>
        )}
      </div>
    </section>
  );
}

function Metric({ label, value, title }: { label: string; value: string; title?: string }) {
  return <div><dt className="text-slate-600">{label}</dt><dd title={title} className="mt-1 truncate font-mono text-slate-300">{value}</dd></div>;
}

function ForensicList({ title, values }: { title: string; values: string[] }) {
  return (
    <div>
      <div className="mb-1.5 text-[9px] font-semibold uppercase tracking-wider text-slate-600">{title}</div>
      <ul className="max-h-40 space-y-1 overflow-y-auto rounded-lg border border-slate-800 bg-slate-950 p-3 font-mono text-[9px] text-slate-400">
        {values.map((value, index) => <li className="break-words" key={`${index}-${value}`}>{value}</li>)}
      </ul>
    </div>
  );
}

function PageFindings({ findings, notes, onNoteChange }: { findings: RedactionPageFinding[]; notes: Record<string, string>; onNoteChange: (key: string, value: string) => void }) {
  return (
    <div>
      <div className="mb-1.5 text-[9px] font-semibold uppercase tracking-wider text-slate-600">Page-level findings</div>
      <div className="max-h-48 space-y-1.5 overflow-y-auto rounded-lg border border-slate-800 bg-slate-950 p-2">
        {findings.map((finding) => (
          <div key={finding.pageNumber} className="rounded border border-slate-800 px-2.5 py-2 text-[9px] text-slate-400">
            <div className="flex items-center justify-between gap-3">
              <span className="font-semibold text-slate-300">Page {finding.pageNumber}</span>
              <span className={finding.coverageComplete ? 'text-emerald-400' : 'text-amber-300'}>{finding.coverageComplete ? 'Complete pass' : 'Limited pass'}</span>
            </div>
            <div className="mt-1 font-mono leading-relaxed">
              {finding.potentialIntersections} intersection{finding.potentialIntersections === 1 ? '' : 's'} · {finding.darkOverlayIntersections} dark overlay · {finding.redactionAnnotationIntersections} annotation · {finding.unappliedRedactionAnnotations} unapplied annotation{finding.unappliedRedactionAnnotations === 1 ? '' : 's'} · {finding.invisibleTextIntersections} invisible text · {finding.formInvocationsAnalyzed} form invocation{finding.formInvocationsAnalyzed === 1 ? '' : 's'} analyzed
            </div>
            {finding.intersections.length > 0 && (
              <details className="mt-1.5 border-t border-slate-800 pt-1.5">
                <summary className="cursor-pointer text-slate-500">Intersection coordinates</summary>
                <ul className="mt-1 space-y-1 font-mono text-[8px] text-slate-500">
                  {finding.intersections.map((intersection, index) => (
                    <li key={`${intersection.kind}-${index}`} className="space-y-1.5 rounded border border-slate-800 p-2">
                      <div>{intersection.kind} · text [{formatBounds(intersection.textBounds)}] · mask [{formatBounds(intersection.maskBounds)}]</div>
                      <label className="block text-[8px] text-slate-600">Review note (geometry only; no recovered text)<textarea value={notes[reviewKey(finding.pageNumber, intersection.kind, index)] ?? ''} onChange={(event) => onNoteChange(reviewKey(finding.pageNumber, intersection.kind, index), event.target.value.slice(0, 2_000))} className="mt-1 block min-h-14 w-full rounded border border-slate-800 bg-slate-950 p-2 text-[9px] text-slate-300" placeholder="Record reviewer disposition or follow-up…" /></label>
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}

const EVIDENCE_CLASSES: EvidenceClass[] = ['preserved', 'historical', 'derived', 'hypothesis'];

function EvidenceTabs({ bundle, activeClass, onSelect }: { bundle: EvidenceBundle; activeClass: EvidenceClass; onSelect: (value: EvidenceClass) => void }) {
  const findings = bundle.findings.filter((finding) => finding.class === activeClass);
  return <section className="rounded-lg border border-slate-800 bg-slate-950/50 p-3"><div className="flex flex-wrap gap-1" role="tablist" aria-label="Evidence class">{EVIDENCE_CLASSES.map((value) => { const count = bundle.findings.filter((finding) => finding.class === value).length; return <button key={value} type="button" role="tab" aria-selected={activeClass === value} onClick={() => onSelect(value)} className={activeClass === value ? 'rounded border border-indigo-400/30 bg-indigo-500/10 px-2 py-1 text-[8px] font-semibold uppercase text-indigo-200' : 'rounded border border-slate-800 px-2 py-1 text-[8px] uppercase text-slate-500'}>{value} · {count}</button>; })}</div><div className="mt-2 space-y-1.5">{findings.length === 0 ? <p className="text-[9px] text-slate-600">No {activeClass} findings were reported by the collectors that completed.</p> : findings.map((finding, index) => <article key={`${finding.kind}-${finding.valueSha256 ?? index}`} className="rounded border border-slate-800 p-2 text-[9px]"><div className="font-semibold text-slate-300">{finding.kind.split('_').join(' ').toLocaleLowerCase()} · {finding.confidence}</div>{finding.maskedPreview && <pre className="mt-1 max-h-20 overflow-auto whitespace-pre-wrap font-mono text-slate-400">{finding.maskedPreview}</pre>}<p className="mt-1 text-slate-500">{finding.explanation}</p></article>)}</div></section>;
}

function firstPopulatedEvidenceClass(bundle: EvidenceBundle): EvidenceClass {
  return EVIDENCE_CLASSES.find((value) => bundle.findings.some((finding) => finding.class === value)) ?? 'preserved';
}

function reviewKey(pageNumber: number, kind: string, index: number) {
  return `${pageNumber}:${kind}:${index}`;
}

function readReviewNotes(key: string): Record<string, string> {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem(key) ?? '{}');
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).filter((entry): entry is [string, string] => typeof entry[1] === 'string'));
  } catch {
    return {};
  }
}

function persistReviewNotes(key: string, notes: Record<string, string>) {
  try {
    window.localStorage.setItem(key, JSON.stringify(notes));
  } catch {
    // Review notes remain available for this session when browser storage is disabled or full.
  }
}

function formatBounds(bounds: [number, number, number, number]) {
  return bounds.map((value) => value.toFixed(2)).join(', ');
}
