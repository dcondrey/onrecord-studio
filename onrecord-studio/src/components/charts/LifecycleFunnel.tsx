import React from 'react';
import { useAppStore } from '../../store/useAppStore';

export default function LifecycleFunnel() {
  const { metrics } = useAppStore();

  const baseCount = Math.max(10, metrics.pagesProcessed * 50);
  const stages = [
    { title: 'Total Ingested Cases', count: baseCount, percentage: 100, color: 'bg-indigo-500' },
    { title: 'Contains Attachments', count: Math.floor(baseCount * 0.65), percentage: 65, color: 'bg-cyan-500' },
    { title: 'Text Layer Extracted', count: metrics.documentsDownloaded, percentage: Math.min(100, Math.floor((metrics.documentsDownloaded / (baseCount || 1)) * 100)), color: 'bg-purple-500' },
    { title: 'Flagged for Manual Audit', count: metrics.anomaliesFound, percentage: Math.min(100, Math.floor((metrics.anomaliesFound / (baseCount || 1)) * 100)), color: 'bg-amber-500' }
  ];

  return (
    <div className="p-4 bg-slate-950/20 rounded-xl space-y-4 select-none h-full">
      <div className="text-[10px] font-bold text-slate-500 uppercase tracking-wider">
        Operational Lifecycle Funnel
      </div>
      <div className="space-y-2.5 pt-2">
        {stages.map((stage, i) => (
          <div key={i} className="space-y-1">
            <div className="flex justify-between items-center text-[10px] font-mono">
              <span className="text-slate-400 font-semibold">{stage.title}</span>
              <span className="text-slate-500">{stage.count} records ({stage.percentage}%)</span>
            </div>
            <div className="w-full h-2 bg-slate-900 rounded-full overflow-hidden border border-slate-800/40">
              <div 
                className={`${stage.color} h-full rounded-full transition-all duration-500 ease-out`}
                style={{ width: `${Math.max(3, stage.percentage)}%` }}
              />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
