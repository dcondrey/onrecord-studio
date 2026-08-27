import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { RedactionIntegrityReport, RedactionPageFinding } from '../../types';
import { cn } from '../../utils/cn';

export default function UnredactorPanel({ documentId, portalKey }: { documentId: number; portalKey: string }) {
  // FEATURE(documents): Add typed evidence tabs for preserved, historical, derived, and
  // hypothesis findings once the native forensic collectors expose their shared contract.
  // NOTE(documents): Do not display speculative candidates beside preserved text.
  // PERF(documents): A bounded scan is safe for now; add progress reporting
  // and worker scheduling before allowing repeated scans across many pages.
  // FEATURE(documents): Add a review workflow that lets investigators attach
  // notes to each quarantined intersection without exposing raw sensitive text.
  const [isScanning, setIsScanning] = useState(false);
  const [report, setReport] = useState<RedactionIntegrityReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const scanGeneration = useRef(0);
  const coverageLimited = report !== null && !report.coverageComplete;

  useEffect(() => {
    scanGeneration.current += 1;
    setIsScanning(false);
    setReport(null);
    setError(null);
  }, [documentId, portalKey]);

  const scan = async () => {
    const generation = ++scanGeneration.current;
    setIsScanning(true);
    setReport(null);
    setError(null);
    try {
      const result = await invokeCommand('verify_document_redaction_integrity', { documentId, portalKey });
      if (generation === scanGeneration.current) setReport(result);
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
          {isScanning ? 'Inspecting…' : 'Run integrity scan'}
        </button>
      </div>
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
            {report.pageFindings.length > 0 && <PageFindings findings={report.pageFindings} />}
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

function PageFindings({ findings }: { findings: RedactionPageFinding[] }) {
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
                    <li key={`${intersection.kind}-${index}`}>{intersection.kind} · text [{formatBounds(intersection.textBounds)}] · mask [{formatBounds(intersection.maskBounds)}]</li>
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

function formatBounds(bounds: [number, number, number, number]) {
  return bounds.map((value) => value.toFixed(2)).join(', ');
}
