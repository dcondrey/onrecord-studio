import React from 'react';

interface DocumentViewerProps {
  fileName?: string;
  textContent?: string;
  fileSizeFormatted?: string;
}

export default function DocumentViewer({ 
  fileName = 'responsive_records_redacted.pdf', 
  textContent = 'EXTRACTED PARSED TEXT CORE:\n\n1. Purpose: Localized emergency ordinance operations...\n2. Summary parameters: Funding allocation metrics verify systemic policy compliance markers...\n3. Encampment cleanup tracking maps indicate multi-department coordination logs between Environmental Services nodes and local administrative commands.',
  fileSizeFormatted = '1.45 MB'
}: DocumentViewerProps) {
  return (
    <div className="flex flex-col h-full bg-slate-950 border border-slate-800 rounded-xl overflow-hidden font-sans shadow-md">
      {/* Document header properties container */}
      <div className="h-10 border-b border-slate-800 px-4 bg-slate-900/40 flex items-center justify-between select-none">
        <div className="flex items-center space-x-2">
          <svg className="h-4 w-4 text-indigo-400" fill="none" viewBox="0 0 24 24" stroke="currentColor" strokeWidth={2}>
            <path strokeLinecap="round" strokeLinejoin="round" d="M7 21h10a2 2 0 002-2V9.414a1 1 0 00-.293-.707l-5.414-5.414A1 1 0 0012.586 3H7a2 2 0 00-2 2v14a2 2 0 002 2z" />
          </svg>
          <span className="text-xs font-semibold text-slate-300 truncate max-w-xs">{fileName}</span>
        </div>
        <span className="text-[10px] font-mono bg-slate-800 border border-slate-700/60 px-1.5 py-0.5 rounded text-slate-400">
          {fileSizeFormatted}
        </span>
      </div>
      
      {/* Extracted plain character layout panel */}
      <div className="flex-1 p-5 font-mono text-xs text-slate-300 overflow-y-auto leading-relaxed bg-slate-950 shadow-inner select-text whitespace-pre-wrap selection:bg-indigo-500/30">
        {textContent}
      </div>
    </div>
  );
}
