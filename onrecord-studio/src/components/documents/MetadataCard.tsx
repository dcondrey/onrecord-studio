import { useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { ProvenanceVerificationRecord } from '../../types';

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
  const [latestVerification, setLatestVerification] = useState<ProvenanceVerificationRecord | null>(null);
  const [verificationError, setVerificationError] = useState<string | null>(null);
  const shownDigest = useMemo(() => formatDigest(sha256, digestFormat), [digestFormat, sha256]);

  useEffect(() => {
    let current = true;
    setLatestVerification(null);
    setVerificationError(null);
    void invokeCommand('list_document_provenance_verifications', { documentId, portalKey, limit: 1 })
      .then((records) => { if (current) setLatestVerification(records[0] ?? null); })
      .catch((reason) => { if (current) setVerificationError(String(reason)); });
    return () => { current = false; };
  }, [documentId, portalKey]);

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
    <section className="space-y-2 rounded-xl border border-slate-800 bg-slate-900/20 p-4 font-mono text-label" aria-label="Document integrity metadata">
      <div className="text-caption font-bold uppercase tracking-wider text-slate-400">Evidence integrity</div>
      <MetadataRow label="MIME type" value={contentType ?? 'Not reported'} />
      <MetadataRow label="Visibility" value={visibility} />
      <MetadataRow label="Download" value={downloadStatus} />
      <MetadataRow label="File size" value={formatFileSize(fileSize)} />
      <MetadataRow label="Latest verification" value={verificationLabel(latestVerification, verificationError)} />
      <div className="space-y-1 border-t border-slate-800/60 pt-2">
        <div className="flex items-center justify-between gap-2">
          <span className="text-slate-400">Recorded SHA-256</span>
          <div className="flex items-center gap-1">
            <select aria-label="Digest format" value={digestFormat} onChange={(event) => setDigestFormat(event.target.value as DigestFormat)} className="rounded border border-slate-800 bg-slate-950 px-1.5 py-1 text-caption text-slate-400"><option value="hex">Hex</option><option value="colon">Colon hex</option><option value="base64">Base64</option></select>
            <button type="button" disabled={!digestIsValid} onClick={() => void copyDigest()} className="rounded border border-slate-800 px-1.5 py-1 text-caption text-slate-400 disabled:opacity-40">Copy</button>
          </div>
        </div>
        <span className={`block select-text break-all rounded border p-2 text-caption ${digestIsValid ? 'border-emerald-400/20 bg-emerald-500/[0.06] text-emerald-300' : 'border-amber-400/20 bg-amber-500/[0.06] text-amber-300'}`}>
          {shownDigest ?? 'No source-file digest has been recorded'}
        </span>
        {copyStatus && <span role="status" className="block text-right text-caption text-slate-400">{copyStatus}</span>}
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

function verificationLabel(record: ProvenanceVerificationRecord | null, error: string | null) {
  if (error) return 'History unavailable';
  if (!record) return 'No recorded check';
  const state = record.provenanceConfidence?.split('_').join(' ').toLocaleLowerCase() ?? 'unclassified';
  const timestamp = new Date(record.verifiedAt);
  return `${state} · ${Number.isNaN(timestamp.getTime()) ? record.verifiedAt : timestamp.toLocaleString()}`;
}

function MetadataRow({ label, value }: { label: string; value: string }) {
  return <div className="flex items-center justify-between gap-4 border-b border-slate-800/40 py-1"><span className="text-slate-400">{label}</span><span className="truncate text-right font-medium text-slate-300">{value}</span></div>;
}

function formatFileSize(value: number | null) {
  if (value === null || !Number.isFinite(value) || value < 0) return 'Not reported';
  if (value < 1024) return `${value.toLocaleString()} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
import { isSha256Hex } from '../../utils/crypto';
