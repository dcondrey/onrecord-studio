export type LadderMarker = 'diamond' | 'ring' | 'dashed' | 'square' | 'triangle';

export interface LadderRung {
  label: string;
  note?: string;
  marker: LadderMarker;
  /** A rung this build cannot produce, drawn as an outline only. */
  unreachable?: boolean;
}

const ROW = 46;
const TOP = 34;
const WIDTH = 560;

function Marker({ kind, x, y, unreachable }: { kind: LadderMarker; x: number; y: number; unreachable?: boolean }) {
  const common = { className: 'stroke-current', fill: unreachable ? 'none' : 'currentColor', fillOpacity: unreachable ? 0 : 0.18, strokeWidth: 2 } as const;
  switch (kind) {
    case 'diamond':
      return <polygon {...common} fillOpacity={unreachable ? 0 : 0.85} points={`${x},${y - 11} ${x + 11},${y} ${x},${y + 11} ${x - 11},${y}`} />;
    case 'triangle':
      return <polygon {...common} points={`${x},${y - 11} ${x + 11},${y + 9} ${x - 11},${y + 9}`} />;
    case 'square':
      return <rect {...common} x={x - 9} y={y - 9} width={18} height={18} strokeDasharray="4 3" />;
    case 'dashed':
      return <circle {...common} cx={x} cy={y} r={10} strokeDasharray="4 3" />;
    default:
      return <circle {...common} cx={x} cy={y} r={10} fillOpacity={unreachable ? 0 : 0.45} />;
  }
}

/**
 * A vertical ladder, strongest statement first. Every rung pairs a shape with words, so no meaning
 * depends on colour, and the whole figure has a text alternative for assistive technology.
 */
export default function LadderDiagram({
  id,
  title,
  description,
  rungs,
  topLabel,
  bottomLabel,
}: {
  id: string;
  title: string;
  description: string;
  rungs: readonly LadderRung[];
  topLabel: string;
  bottomLabel: string;
}) {
  const height = TOP + rungs.length * ROW + 30;
  return (
    <figure className="my-4">
      <svg
        role="img"
        aria-labelledby={`${id}-title ${id}-desc`}
        viewBox={`0 0 ${WIDTH} ${height}`}
        className="h-auto w-full max-w-[560px] text-fg"
      >
        <title id={`${id}-title`}>{title}</title>
        <desc id={`${id}-desc`}>{description}</desc>
        <text x={8} y={18} className="fill-current" fontSize={12} fontWeight={600}>{topLabel}</text>
        <line x1={28} y1={TOP} x2={28} y2={TOP + rungs.length * ROW - 8} className="stroke-current" strokeOpacity={0.35} strokeWidth={2} />
        {rungs.map((rung, index) => {
          const y = TOP + index * ROW + ROW / 2 - 4;
          return (
            <g key={rung.label} opacity={rung.unreachable ? 0.8 : 1}>
              <rect x={52} y={y - 19} width={WIDTH - 60} height={38} rx={8} className="fill-current stroke-current" fillOpacity={0.06} strokeOpacity={0.35} strokeDasharray={rung.unreachable ? '5 4' : undefined} />
              <Marker kind={rung.marker} x={28} y={y} unreachable={rung.unreachable} />
              <text x={66} y={rung.note ? y - 2 : y + 5} className="fill-current" fontSize={14} fontWeight={600}>{rung.label}</text>
              {rung.note && <text x={66} y={y + 14} className="fill-current" fontSize={12} fillOpacity={0.85}>{rung.note}</text>}
            </g>
          );
        })}
        <text x={8} y={height - 6} className="fill-current" fontSize={12} fontWeight={600}>{bottomLabel}</text>
      </svg>
    </figure>
  );
}
