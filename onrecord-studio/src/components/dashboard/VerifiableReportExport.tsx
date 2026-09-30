import { useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { VerifiableReportExportResult } from '../../types/report';

interface VerifiableReportExportProps {
  /** Limits the report to one portal's findings. */
  portalKey?: string | null;
  /** Limits the report to exactly these findings. Takes precedence over `portalKey`. */
  findingIds?: readonly string[];
}

function scopeKey(portalKey: string | null | undefined, findingIds: readonly string[] | undefined) {
  return JSON.stringify([portalKey ?? null, findingIds ?? null]);
}

/**
 * Writes a signed, self-verifying HTML report of findings into the managed report directory.
 * Inferred-redaction hypotheses are never included; the exported file lists what was withheld.
 */
export default function VerifiableReportExport({ portalKey = null, findingIds }: VerifiableReportExportProps) {
  const gate = useLatestRequestGate();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [result, setResult] = useState<VerifiableReportExportResult | null>(null);
  const currentScope = scopeKey(portalKey, findingIds);
  const currentScopeRef = useRef(currentScope);
  currentScopeRef.current = currentScope;

  // A report requested for one scope must not be shown as the report for another.
  useEffect(() => {
    gate.invalidate();
    setBusy(false);
    setError('');
    setResult(null);
  }, [currentScope, gate]);

  async function exportReport() {
    const token = gate.begin();
    const requestedScope = currentScope;
    setBusy(true);
    setError('');
    setResult(null);
    try {
      const value = await invokeCommand('export_verifiable_report', {
        request: {
          portalKey: portalKey ?? null,
          findingIds: findingIds ? [...findingIds] : null,
          targetPath: null,
          authorName: null,
        },
      });
      if (!gate.isCurrent(token) || currentScopeRef.current !== requestedScope) return;
      setResult(value);
    } catch (reason) {
      if (!gate.isCurrent(token) || currentScopeRef.current !== requestedScope) return;
      setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(token)) setBusy(false);
    }
  }

  return (
    <section className="app-panel p-4" aria-label="Verifiable report">
      <h3 className="font-semibold text-slate-200">Verifiable report</h3>
      <p className="mt-2 text-xs text-slate-400">
        Exports one HTML file that checks its own signature in the reader&apos;s browser. Quoted text is
        masked for personal information, hypotheses about redacted identities are left out, and the
        file lists what was withheld. The signature shows the file is unchanged; it does not identify
        who holds the signing key.
      </p>
      <button
        type="button"
        className="focus-ring mt-3 rounded border border-slate-600 px-3 py-1.5 text-sm text-slate-100 disabled:opacity-50"
        disabled={busy}
        onClick={() => void exportReport()}
      >
        {busy ? 'Exporting…' : 'Export verifiable report'}
      </button>
      {error && <p role="alert" className="mt-3 text-sm text-rose-300">{error}</p>}
      {result && (
        <div role="status" className="mt-3 rounded border border-slate-700 bg-slate-950/60 p-3 text-xs text-slate-300">
          <p className="font-semibold text-slate-200">
            Wrote {result.claimCount} claim(s); {result.withheldCount} item(s) withheld.
          </p>
          <p className="mt-1 break-all">{result.outputPath}</p>
          <p className="mt-1 break-all">File SHA-256 {result.fileSha256}</p>
          <p className="mt-1 break-all">Signer key {result.signerKeyId} ({result.identityAssurance})</p>
          <p className="mt-1 text-slate-400">
            Verify from a terminal with: node scripts/verify-report.mjs &lt;file&gt;
          </p>
        </div>
      )}
    </section>
  );
}
