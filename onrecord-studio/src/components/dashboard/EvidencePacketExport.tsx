import { useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { EvidencePacketExportResult } from '../../types/report';

interface EvidencePacketExportProps {
  /** Limits the packet to one portal's findings. */
  portalKey?: string | null;
  /** Limits the packet to exactly these findings. Takes precedence over `portalKey`. */
  findingIds?: readonly string[];
}

function scopeKey(portalKey: string | null | undefined, findingIds: readonly string[] | undefined) {
  return JSON.stringify([portalKey ?? null, findingIds ?? null]);
}

/**
 * Writes a citable evidence packet (Markdown plus JSON sharing one content hash) into the managed
 * report directory. Every claim keeps its basis label; evidence gaps are listed apart from claims.
 */
export default function EvidencePacketExport({ portalKey = null, findingIds }: EvidencePacketExportProps) {
  const gate = useLatestRequestGate();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [result, setResult] = useState<EvidencePacketExportResult | null>(null);
  const currentScope = scopeKey(portalKey, findingIds);
  const currentScopeRef = useRef(currentScope);
  currentScopeRef.current = currentScope;

  // A packet requested for one scope must not be shown as the packet for another.
  useEffect(() => {
    gate.invalidate();
    setBusy(false);
    setError('');
    setResult(null);
  }, [currentScope, gate]);

  async function exportPacket() {
    const token = gate.begin();
    const requestedScope = currentScope;
    setBusy(true);
    setError('');
    setResult(null);
    try {
      const value = await invokeCommand('export_evidence_packet', {
        request: {
          portalKey: portalKey ?? null,
          findingIds: findingIds ? [...findingIds] : null,
          targetPath: null,
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
    <section className="app-panel p-4" aria-label="Evidence packet">
      <h3 className="font-semibold text-slate-200">Evidence packet</h3>
      <p className="mt-2 text-xs text-slate-400">
        Exports a Markdown file and a JSON file a reporter can cite. Each claim states its basis, its
        sources with digests and quoted excerpts, how the detector reached it, and what it does not
        show. Gaps are listed apart from claims. The content hash lets anyone regenerate and compare;
        the packet is not signed.
      </p>
      <button
        type="button"
        className="focus-ring mt-3 rounded border border-slate-600 px-3 py-1.5 text-sm text-slate-100 disabled:opacity-50"
        disabled={busy}
        onClick={() => void exportPacket()}
      >
        {busy ? 'Exporting…' : 'Export evidence packet'}
      </button>
      {error && <p role="alert" className="mt-3 text-sm text-rose-300">{error}</p>}
      {result && (
        <div role="status" className="mt-3 rounded border border-slate-700 bg-slate-950/60 p-3 text-xs text-slate-300">
          <p className="font-semibold text-slate-200">
            Wrote {result.claimCount} claim(s); {result.gapCount} gap(s) listed separately; {result.withheldCount} item(s) withheld.
          </p>
          <p className="mt-1 break-all">{result.markdownPath}</p>
          <p className="mt-1 break-all">{result.jsonPath}</p>
          <p className="mt-1 break-all">Content SHA-256 {result.contentSha256}</p>
        </div>
      )}
    </section>
  );
}
