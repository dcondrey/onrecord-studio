import { useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AnnotationImportPreview } from '../../types';
import { SignerTrustNotice } from '../dashboard/CaseSharing';
import { useContributors } from '../dashboard/useContributors';
import { downloadText, KIND_LABELS, STATUS_LABELS } from './annotationLogic';
import { annotationButton as button, annotationField as field } from './annotationStyles';

const MAXIMUM_PACKET_BYTES = 16 * 1024 * 1024;

function PreviewReview({
  preview,
  portalKey,
  documentId,
  onApply,
  busy,
}: {
  preview: AnnotationImportPreview;
  portalKey: string;
  documentId: number;
  onApply: () => void;
  busy: boolean;
}) {
  const contributors = useContributors();
  const sameDocument = preview.portalKey === portalKey && preview.documentId === documentId;
  const textDiffers = preview.packetTextSha256 !== null && preview.localTextSha256 !== null && preview.packetTextSha256 !== preview.localTextSha256;
  const textUnknown = preview.packetTextSha256 === null || preview.localTextSha256 === null;
  return (
    <div className="space-y-1.5" aria-label="Annotation import preview">
      <p>
        From <span className="text-fg">{preview.exporter ?? 'an exporter who set no name'}</span> (the name is their claim), exported {preview.exportedAt.slice(0, 10)} with version {preview.appVersion}.
      </p>
      <SignerTrustNotice signerKeyId={preview.signerKeyId} contributors={contributors} />
      {!sameDocument && <p role="alert" className="text-danger">This packet is for {preview.portalKey} / document {preview.documentId}, not this document. Open that document to import it.</p>}
      {textDiffers && <p className="text-warning">The sender's document text differs from yours, so some of these marks may re-anchor or be orphaned here.</p>}
      {!textDiffers && textUnknown && <p className="text-warning">A text digest is missing on one side, so it cannot be shown that both of you read the same text.</p>}
      <p role="status">{preview.toAdd.length} to add, {preview.alreadyPresent} already here, {preview.conflicts.length} with the same id but different content (yours are kept).</p>
      <ul className="space-y-1">
        {preview.toAdd.map((item) => (
          <li key={item.id} className="rounded border border-line p-1.5">
            {KIND_LABELS[item.kind]}{item.tag ? `: ${item.tag}` : ''} · {STATUS_LABELS[item.status].toLowerCase()} · {item.author ?? 'author not recorded'}
            {item.quote && <blockquote className="border-l border-line-strong pl-2 text-muted">{item.quote}</blockquote>}
          </li>
        ))}
      </ul>
      {preview.conflicts.map((conflict) => (
        <p key={conflict.local.id} className="text-warning">Not imported: {conflict.incoming.id.slice(0, 8)} differs from your copy.</p>
      ))}
      <button type="button" className={button} disabled={busy || !sameDocument || preview.toAdd.length === 0} onClick={onApply}>Add these annotations</button>
    </div>
  );
}

/** Signed export and reviewed import of this document's annotations. Imports add; nothing local is overwritten. */
export default function AnnotationExchange({
  portalKey,
  documentId,
  hasOwnMarks,
  onImported,
}: {
  portalKey: string;
  documentId: number;
  hasOwnMarks: boolean;
  onImported: () => void;
}) {
  const gate = useLatestRequestGate();
  const [payload, setPayload] = useState('');
  const [preview, setPreview] = useState<AnnotationImportPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const run = async (work: (request: number) => Promise<void>) => {
    const request = gate.begin();
    setError(null);
    setStatus(null);
    setBusy(true);
    try {
      await work(request);
    } catch (reason) {
      if (gate.isCurrent(request)) setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(request)) setBusy(false);
    }
  };

  const exportPacket = () => run(async (request) => {
    const text = await invokeCommand('export_document_annotations', { portalKey, documentId });
    if (!gate.isCurrent(request)) return;
    downloadText(`annotations-${portalKey}-${documentId}.json`, text);
    setStatus('Signed packet downloaded. It carries your annotations of this document with their quotes and notes.');
  });

  const readFile = (file: File) => run(async (request) => {
    if (file.size > MAXIMUM_PACKET_BYTES) throw new Error('Annotation packets are at most 16 MB.');
    const text = await file.text();
    if (!gate.isCurrent(request)) return;
    setPreview(null);
    setPayload(text);
  });

  const previewImport = () => run(async (request) => {
    const result = await invokeCommand('preview_annotation_import', { payload });
    if (gate.isCurrent(request)) setPreview(result);
  });

  const apply = () => run(async (request) => {
    if (!preview) return;
    const result = await invokeCommand('import_annotation_packet', { payload, packetSha256: preview.packetSha256 });
    if (!gate.isCurrent(request)) return;
    setPreview(null);
    setPayload('');
    setStatus(`Added ${result.added}, ${result.alreadyPresent} already here, ${result.conflicts} left as yours.`);
    onImported();
  });

  return (
    <details className="rounded border border-line p-2">
      <summary className="font-semibold text-fg">Share and import annotations</summary>
      <div className="mt-2 space-y-2">
        <p className="text-muted">Export signs your annotations of this document with this installation's key. A colleague previews the packet, sees whether they trust the key, and adds what is new. The signature shows the file is unchanged since signing, not who holds the key.</p>
        <button type="button" className={button} disabled={busy || !hasOwnMarks} onClick={() => void exportPacket()}>Export signed packet</button>
        <label className="block">
          Open a packet file
          <input type="file" accept=".json,application/json" className="mt-1 block w-full" onChange={(event) => { const file = event.target.files?.[0]; if (file) void readFile(file); }} />
        </label>
        <label className="block">
          Packet JSON
          <textarea rows={3} className={`${field} font-mono`} value={payload} onChange={(event) => { gate.invalidate(); setBusy(false); setPreview(null); setPayload(event.target.value); }} />
        </label>
        <button type="button" className={button} disabled={busy || payload.trim() === ''} onClick={() => void previewImport()}>Preview packet</button>
        {preview && <PreviewReview preview={preview} portalKey={portalKey} documentId={documentId} busy={busy} onApply={() => void apply()} />}
        {error && <p role="alert" className="text-danger">{error}</p>}
        {status && <p role="status" className="text-success">{status}</p>}
      </div>
    </details>
  );
}
