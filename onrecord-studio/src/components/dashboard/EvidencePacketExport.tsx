import { useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AnnotationPreview, EvidencePacketExportResult } from '../../types/report';

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
  const previewGate = useLatestRequestGate();
  const [includeAnnotations, setIncludeAnnotations] = useState(false);
  const [preview, setPreview] = useState<AnnotationPreview | null>(null);
  const [previewError, setPreviewError] = useState('');
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

  // The count shown must describe the selection that will be exported, so it is re-read whenever
  // the scope or the checkbox changes and a late answer for an earlier scope is dropped.
  useEffect(() => {
    previewGate.invalidate();
    setPreview(null);
    setPreviewError('');
    if (!includeAnnotations) return;
    const token = previewGate.begin();
    invokeCommand('preview_packet_annotations', {
      request: { portalKey: portalKey ?? null, findingIds: findingIds ? [...findingIds] : null },
    })
      .then((value) => {
        if (previewGate.isCurrent(token)) setPreview(value);
      })
      .catch((reason: unknown) => {
        if (previewGate.isCurrent(token)) setPreviewError(formatNativeError(reason));
      });
  }, [currentScope, includeAnnotations, previewGate]);

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
          includeAnnotations,
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
      <h3 className="font-semibold text-fg">Evidence packet</h3>
      <p className="mt-2 text-label leading-4 text-muted">
        Exports a Markdown file and a JSON file a reporter can cite. Each claim states its basis, its
        sources with digests and quoted excerpts, how the detector reached it, and what it does not
        show. Gaps are listed apart from claims. Your annotations are left out unless you tick the box below. The content hash lets anyone regenerate and compare;
        the packet is not signed.
      </p>
      <label className="mt-3 flex items-start gap-2 text-sm text-fg">
        <input
          type="checkbox"
          className="mt-1"
          checked={includeAnnotations}
          onChange={(event) => setIncludeAnnotations(event.target.checked)}
        />
        <span>Include my annotations on cited documents</span>
      </label>
      {includeAnnotations && (
        <div className="mt-2 rounded border border-warning/30 bg-warning/5 p-3 text-label leading-4 text-warning">
          <p>
            Adds a separate section, labelled as the reviewer&apos;s reading and not part of the record. For
            each annotation on a document the claims cite it writes the code, author, status, anchor state,
            the quoted passage and your note, with the document&apos;s citation and text digest, into both the
            Markdown and the JSON file. Notes can hold private commentary: anything in a note is written.
            Annotations on documents that are not public are not written. Orphaned annotations are kept and
            labelled.
          </p>
          {previewError && <p role="alert" className="mt-2 text-danger">{previewError}</p>}
          {!previewError && !preview && <p className="mt-2">Counting annotations…</p>}
          {preview && (
            <p role="status" className="mt-2 font-semibold">
              {preview.included} annotation(s) will be written, {preview.orphaned} of them orphaned
              {preview.notPublicOmitted > 0 ? `; ${preview.notPublicOmitted} on non-public documents will not be written` : ''}.
            </p>
          )}
        </div>
      )}
      <button
        type="button"
        className="focus-ring mt-3 rounded border border-control px-3 py-1.5 text-sm text-fg disabled:opacity-50"
        disabled={busy || (includeAnnotations && !preview)}
        onClick={() => void exportPacket()}
      >
        {busy ? 'Exporting…' : 'Export evidence packet'}
      </button>
      {error && <p role="alert" className="mt-3 text-sm text-danger">{error}</p>}
      {result && (
        <div role="status" className="mt-3 rounded border border-line-strong bg-canvas/70 p-3 text-label leading-4 text-fg-secondary">
          <p className="font-semibold text-fg">
            Wrote {result.claimCount} claim(s); {result.gapCount} gap(s) listed separately; {result.withheldCount} item(s) withheld.
          </p>
          <p className="mt-1 break-all">{result.markdownPath}</p>
          <p className="mt-1 break-all">{result.jsonPath}</p>
          {result.annotationCount !== null && (
            <p className="mt-1">Wrote {result.annotationCount} reviewer annotation(s) in a separate section.</p>
          )}
          <p className="mt-1 break-all">Content SHA-256 {result.contentSha256}</p>
        </div>
      )}
    </section>
  );
}
