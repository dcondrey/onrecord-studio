import React from 'react';
import { useAppStore } from '../../store/useAppStore';

export default function AnomalyRadar() {
  const { metrics } = useAppStore();

  // Baseline mock data vectors calibrated against the global ingestion telemetry state
  const indicators = [
    { label: 'Statutory Delay', value: Math.min(100, metrics.pagesProcessed * 12) },
    { label: 'Redaction Density', value: Math.min(100, metrics.documentsDownloaded * 8) },
    { label: 'Chronology Anomaly', value: Math.min(100, metrics.anomaliesFound * 25) },
    { label: 'Arbitrary Closure', value: Math.min(100, metrics.anomaliesFound * 15) },
    { label: 'Data Omission', value: Math.min(100, metrics.pagesProcessed * 4) }
  ];

  const totalPoints = indicators.length;
  const radius = 60;
  const center = 100;

  // Generate radar ring grid coordinates
  const getCoordinates = (index: number, scaleValue: number) => {
    const angle = (Math.PI * 2 / totalPoints) * index - Math.PI / 2;
    const distance = (scaleValue / 100) * radius;
    return {
      x: center + distance * Math.cos(angle),
      y: center + distance * Math.sin(angle)
    };
  };

  const polyPoints = indicators
    .map((ind, i) => {
      const { x, y } = getCoordinates(i, ind.value || 10);
      return `${x},${y}`;
    })
    .join(' ');

  return (
    <div className="flex flex-col items-center justify-center p-4 h-full bg-slate-950/20 rounded-xl select-none">
      <div className="text-[10px] font-bold text-slate-500 uppercase tracking-wider mb-4">
        Anomaly Dimension Radar
      </div>
      <svg viewBox="0 0 200 200" className="w-40 h-40 overflow-visible">
        {/* Draw background concentric reference web rings */}
        {[25, 50, 75, 100].map((ring, idx) => (
          <circle
            key={idx}
            cx={center}
            cy={center}
            r={(ring / 100) * radius}
            fill="none"
            stroke="rgba(51, 65, 85, 0.3)"
            strokeWidth="1"
            strokeDasharray="2,2"
          />
        ))}

        {/* Draw multidimensional parameter coordinate axes */}
        {indicators.map((_, i) => {
          const outer = getCoordinates(i, 100);
          return (
            <line
              key={i}
              x1={center}
              y1={center}
              x2={outer.x}
              y2={outer.y}
              stroke="rgba(51, 65, 85, 0.4)"
              strokeWidth="1"
            />
          );
        })}

        {/* Render live reactive data vector canvas */}
        {polyPoints && (
          <polygon
            points={polyPoints}
            fill="rgba(99, 102, 241, 0.15)"
            stroke="rgba(99, 102, 241, 0.8)"
            strokeWidth="1.5"
            className="transition-all duration-500 ease-in-out"
          />
        )}

        {/* Text anchors mapping axis names */}
        {indicators.map((ind, i) => {
          const textPos = getCoordinates(i, 130);
          const isLeft = textPos.x < center;
          return (
            <text
              key={i}
              x={textPos.x}
              y={textPos.y + 3}
              fontSize="7"
              fontWeight="600"
              fontFamily="monospace"
              fill="rgba(148, 163, 184, 0.8)"
              textAnchor={isLeft ? "end" : "start"}
            >
              {ind.label}
            </text>
          );
        })}
      </svg>
    </div>
  );
}
