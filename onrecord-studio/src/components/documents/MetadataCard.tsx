// MetadataCard.tsx placeholder file contents mapping asset hashes
import React from 'react';

interface MetadataCardProps {
  sha256?: string;
  contentType?: string;
  visibility?: string;
}

export default function MetadataCard({
  sha256 = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
  contentType = 'application/pdf',
  visibility = 'public_access'
}: MetadataCardProps) {
  return (
    <div className="p-4 border border-slate-800 bg-slate-900/20 backdrop-blur-md rounded-xl font-mono text-[10px] space-y-2 select-none">
      <div className="text-slate-500 font-bold uppercase tracking-wider text-[9px]">Data Node Integrity</div>
      <div className="flex justify-between items-center py-0.5 border-b border-slate-800/40">
        <span className="text-slate-500">MIME Type:</span>
        <span className="text-slate-300 font-medium">{contentType}</span>
      </div>
      <div className="flex justify-between items-center py-0.5 border-b border-slate-800/40">
        <span className="text-slate-500">Visibility:</span>
        <span className="text-emerald-400 font-medium lowercase tracking-wide">{visibility}</span>
      </div>
      <div className="space-y-0.5">
        <span className="text-slate-500 block">SHA-256 Checksum:</span>
        <span className="text-indigo-400 break-all select-text font-medium text-[9px] bg-slate-950/60 p-1.5 rounded border border-slate-800/50 block">
          {sha256}
        </span>
      </div>
    </div>
  );
}
