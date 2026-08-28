interface MetadataCardProps {
  sha256: string | null;
  contentType: string | null;
  visibility: string;
  downloadStatus: string;
  fileSize: number | null;
}

export default function MetadataCard({ sha256, contentType, visibility, downloadStatus, fileSize }: MetadataCardProps) {
  // FEATURE(integrity): Add a copyable digest format selector and provenance
  // verification status beside the recorded hash.
  const digestIsValid = sha256 !== null && isSha256Hex(sha256);

  return (
    <section className="space-y-2 rounded-xl border border-slate-800 bg-slate-900/20 p-4 font-mono text-[10px]" aria-label="Document integrity metadata">
      <div className="text-[9px] font-bold uppercase tracking-wider text-slate-500">Evidence integrity</div>
      <MetadataRow label="MIME type" value={contentType ?? 'Not reported'} />
      <MetadataRow label="Visibility" value={visibility} />
      <MetadataRow label="Download" value={downloadStatus} />
      <MetadataRow label="File size" value={formatFileSize(fileSize)} />
      <div className="space-y-1 border-t border-slate-800/60 pt-2">
        <span className="block text-slate-500">Recorded SHA-256</span>
        <span className={`block select-text break-all rounded border p-2 text-[9px] ${digestIsValid ? 'border-emerald-400/20 bg-emerald-500/[0.06] text-emerald-300' : 'border-amber-400/20 bg-amber-500/[0.06] text-amber-300'}`}>
          {sha256 ?? 'No source-file digest has been recorded'}
        </span>
      </div>
    </section>
  );
}

function MetadataRow({ label, value }: { label: string; value: string }) {
  return <div className="flex items-center justify-between gap-4 border-b border-slate-800/40 py-1"><span className="text-slate-500">{label}</span><span className="truncate text-right font-medium text-slate-300">{value}</span></div>;
}

function formatFileSize(value: number | null) {
  if (value === null || !Number.isFinite(value) || value < 0) return 'Not reported';
  if (value < 1024) return `${value.toLocaleString()} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  if (value < 1024 * 1024 * 1024) return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  return `${(value / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
import { isSha256Hex } from '../../utils/crypto';
