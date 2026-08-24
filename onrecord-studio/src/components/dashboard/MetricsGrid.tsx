import React from 'react';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';

export default function MetricsGrid() {
  const metrics = useAppStore((state) => state.metrics);

  /**
   * Translates raw data sizes into readable byte scales.
   * Matches the binary transformation calculations optimized in your Rust core.
   */
  const formatBytes = (bytes: number): string => {
    if (bytes === 0) return '0 Bytes';
    const k = 1024;
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
  };

  const cards = [
    {
      label: 'Pages Analyzed',
      value: metrics.pagesProcessed,
      subtitle: 'API segments pulled',
      color: 'text-indigo-400',
      borderColor: 'border-indigo-500/10'
    },
    {
      label: 'Data Imbibed',
      value: formatBytes(metrics.bytesTransferred),
      subtitle: 'Attachments saved to disk',
      color: 'text-cyan-400',
      borderColor: 'border-cyan-500/10'
    },
    {
      label: 'Anomalies Isolated',
      value: metrics.anomaliesFound,
      subtitle: 'Timeline non-compliance',
      color: cn(metrics.anomaliesFound > 0 ? 'text-amber-400' : 'text-slate-500'),
      borderColor: cn(metrics.anomaliesFound > 0 ? 'border-amber-500/20 bg-amber-500/[0.01]' : 'border-slate-800/80')
    }
  ];

  return (
    <section className="grid grid-cols-3 gap-4 px-6 py-4 select-none">
      {cards.map((card, idx) => (
        <div 
          key={idx} 
          className={cn(
            "border p-4 rounded-xl shadow-sm backdrop-blur-md bg-slate-900/30 transition-all duration-300",
            card.borderColor
          )}
        >
          <div className="text-[10px] font-bold text-slate-500 uppercase tracking-wider mb-0.5">
            {card.label}
          </div>
          <div className={cn("text-2xl font-extrabold tracking-tight font-mono", card.color)}>
            {card.value}
          </div>
          <div className="text-[11px] text-slate-400 font-medium mt-1 truncate">
            {card.subtitle}
          </div>
        </div>
      ))}
    </section>
  );
}
