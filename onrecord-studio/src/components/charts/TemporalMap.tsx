import React from 'react';

export default function TemporalMap() {
  return (
    <div className="p-4 bg-slate-950/20 rounded-xl flex flex-col h-full select-none">
      <div className="text-[10px] font-bold text-slate-500 uppercase tracking-wider mb-3">
        Spatial Density Mapping Grid
      </div>
      <div className="flex-1 relative bg-slate-950/80 border border-slate-900 rounded-lg overflow-hidden flex items-center justify-center border-dashed border-slate-800">
        {/* Synthetic spatial grid map canvas */}
        <div className="absolute inset-0 bg-[radial-gradient(#1e293b_1px,transparent_1px)] [background-size:12px_12px] opacity-40" />
        
        {/* Representative municipal cluster hotspots */}
        <div className="absolute top-1/4 left-1/3 h-12 w-12 rounded-full bg-indigo-500/10 border border-indigo-400/20 flex items-center justify-center animate-pulse" />
        <div className="absolute bottom-1/3 right-1/4 h-16 w-16 rounded-full bg-cyan-500/10 border border-cyan-400/20 flex items-center justify-center animate-pulse duration-1000" />
        
        <span className="text-[10px] font-mono text-slate-600 z-10 uppercase tracking-wider">
          GIS Overlay Engine Node Active
        </span>
      </div>
    </div>
  );
}
