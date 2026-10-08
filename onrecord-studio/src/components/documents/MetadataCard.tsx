import { useEffect, useMemo, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { ProvenanceVerificationRecord } from '../../types';
import { isSha256Hex } from '../../utils/crypto';
import { enumLabel, formatInstant } from '../objects/labels';

interface MetadataCardProps {
  documentId: number;
  portalKey: string;
  sha256: string | null;
  contentType: string | null;
  visibility: string;
  downloadStatus: string;
  fileSize: number | null;
}

type DigestFormat = 'hex' | 'colon' | 'base64';

export default function MetadataCard({ documentId, portalKey, sha256, contentType, visibility, downloadStatus, fileSize }: MetadataCardProps) {
  const digestIsValid = sha256 !== null && isSha256Hex(sha256);
  const [digestFormat, setDigestFormat] = useState<DigestFormat>('hex');
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [latestVerification, setLatestVerification] = useState<ProvenanceVerificationRecord | null | undefined>(undefined);
  const [verificationError, setVerificationError] = useState<string | null>(null);
  const shownDigest = useMemo(() => formatDigest(sha256, digestFormat), [digestFormat, sha256]);
  const verificationGate = useLatestRequestGate();

  useEffect(() => {
    const token = verificationGate.begin();
    setLatestVerification(undefined);
    setVerificationError(null);
    setCopyStatus(null);
    void invokeCommand('list_document_provenance_verifications', { documentId, portalKey, limit: 1 })
      .then((records) => { if (verificationGate.isCurrent(token)) setLatestVerification(records[0] ?? null); })
      .catch((reason) => { if (verificationGate.isCurrent(token)) setVerificationError(formatNativeError(reason)); });
    return () => verificationGate.invalidate();
  }, [documentId, portalKey, verificationGate]);

  const copyDigest = async () => {
    if (!digestIsValid || !shownDigest) return;
    try {
      await navigator.clipboard.writeText(shownDigest);
      setCopyStatus('Copied');
    } catch {
      setCopyStatus('Clipboard unavailable');
    }
  };

  return (
    <section className="space-y-2 rounded-xl border border-line bg-canvas p-4 text-body" aria-label="Document integrity metadata">
      <h3 className="text-overline uppercase text-muted">Evidence integrity</h3>
      <MetadataRow label="File type" value={contentType ? fileTypeLabel(contentType) : 'Not reported'} title={contentType ?? undefined} />
      <MetadataRow label="Visibility" value={enumLabel(visibility)} />
      <MetadataRow label="Download" value={enumLabel(downloadStatus)} />
      <MetadataRow label="File size" value={formatFileSize(fileSize)} />
      <MetadataRow label="Latest verification" value={verificationLabel(latestVerification, verificationError)} />
      <div className="space-y-1 border-t border-line pt-2">
        <div className="flex items-center justify-between gap-2">
          <span className="text-muted">Recorded SHA-256</span>
          <div className="flex items-center gap-1">
            <select aria-label="Digest format" value={digestFormat} onChange={(event) => { setDigestFormat(event.target.value as DigestFormat); setCopyStatus(null); }} className="focus-ring h-7 rounded-md border border-control bg-canvas px-2 text-caption text-fg-secondary"><option value="hex">Hex</option><option value="colon">Colon hex</option><option value="base64">Base64</option></select>
            <button type="button" aria-label="Copy SHA-256 digest" disabled={!digestIsValid} onClick={() => void copyDigest()} className="focus-ring h-7 rounded-md border border-control px-2 text-caption text-fg-secondary hover:bg-raised disabled:opacity-40">Copy</button>
          </div>
        </div>
        <span className={`block select-text break-all rounded-md border p-2 font-mono text-code ${digestIsValid ? 'border-success/30 bg-success/5 text-success' : 'border-warning/30 bg-warning/5 text-warning'}`}>
          {shownDigest ?? 'No source-file digest has been recorded'}
        </span>
        {copyStatus && <span role="status" className="block text-right text-caption text-muted">{copyStatus}</span>}
      </div>
    </section>
  );
}

function formatDigest(sha256: string | null, format: DigestFormat) {
  if (!sha256 || !isSha256Hex(sha256)) return sha256;
  const normalized = sha256.toLocaleLowerCase();
  if (format === 'colon') return normalized.match(/.{2}/g)?.join(':') ?? normalized;
  if (format === 'base64') {
    const bytes = normalized.match(/.{2}/g)?.map((pair) => Number.parseInt(pair, 16)) ?? [];
    return btoa(String.fromCharCode(...bytes));
  }
  return normalized;
}

function verificationLabel(record: ProvenanceVerificationRecord | null | undefined, error: string | null) {
  if (error) return 'History unavailable';
  if (record === undefined) return 'Reading log…';
  if (record === null) return 'No recorded check';
  const state = record.provenanceConfidence?.split('_').join(' ').toLocaleLowerCase()
    ?? (record.predatesClassification ? 'logged before classification' : 'no confidence recorded');
  return `${state} · ${formatInstant(record.verifiedAt)}`;
}

const FILE_TYPES: Record<string, string> = {
  'application/pdf': 'PDF',
  'text/plain': 'Plain text',
  'text/csv': 'CSV',
  'text/html': 'HTML',
  'application/json': 'JSON',
  'message/rfc822': 'Email',
  'application/vnd.ms-outlook': 'Outlook message',
  'application/zip': 'ZIP archive',
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet': 'Excel workbook',
  'application/vnd.ms-excel': 'Excel workbook',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document': 'Word document',
  'application/msword': 'Word document',
  'image/png': 'PNG image',
  'image/jpeg': 'JPEG image',
  'image/tiff': 'TIFF image',
};

/** The type a reader recognises; an unlisted MIME type is shown as recorded. */
function fileTypeLabel(contentType: string): string {
  return FILE_TYPES[contentType.split(';')[0].trim().toLowerCase()] ?? contentType;
}

function MetadataRow({ label, value, title }: { label: string; value: string; title?: string }) {
  return <div className="flex items-start justify-between gap-4 border-b border-line py-1"><span className="shrink-0 text-muted">{label}</span><span title={title} className="min-w-0 break-words text-right font-medium text-fg-secondary">{value}</span></div>;
}

function formatFileSize(value: number | null) {
  if (value === null || !Number.isFinite(value) || value < 0) return 'Not reported';
  if (value < 1024) return `${value.toLocaleString()} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
