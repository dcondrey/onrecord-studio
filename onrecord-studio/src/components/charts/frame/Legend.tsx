import type { KeyboardEvent, ReactNode } from 'react';
import { COMPARISON_DASH, VIZ } from '../core/claim';
import type { LegendFocus } from '../hooks/useLegendFocus';
import { useSvgId } from './VizDefs';

export type SwatchKind = 'line' | 'dashed-line' | 'rect' | 'dot' | 'hollow' | 'ring' | 'hatch' | 'flag' | 'slot-dashed' | 'slot-dotted' | 'diamond' | 'ring-dotted' | 'ramp' | 'hollow-diamond' | 'triangle' | 'outline-rect' | 'tint';
/** A mark colour, or a step of the ordinal ramp (a stacked bar's segments). */
export type SwatchTone = Exclude<keyof typeof VIZ, 'surface' | 'grid' | 'focus'> | 'ord-1' | 'ord-2' | 'ord-3' | 'ord-4';

export interface LegendItem {
  key: string;
  label: string;
  swatch: SwatchKind;
  /** The mark colour the swatch stands for. Default ink. The label text is never coloured. */
  tone?: SwatchTone;
  /** A key drawn by the figure itself, in place of `swatch`, when no standard swatch is the mark. */
  icon?: ReactNode;
}

/** A 16×12 key for one legend entry or tooltip row, drawn like the mark it stands for. */
export function LegendSwatch({ kind, tone = 'ink' }: { kind: SwatchKind; tone?: SwatchTone }) {
  const hatchId = useSvgId('viz-legend-hatch');
  const colour = tone.startsWith('ord-') ? `var(--viz-${tone})` : VIZ[tone as keyof typeof VIZ];
  if (kind === 'ramp') {
    return (
      <svg width={30} height={12} viewBox="0 0 30 12" aria-hidden="true" className="shrink-0 overflow-visible">
        {[1, 2, 3, 4, 5].map((step) => <rect key={step} x={(step - 1) * 6} y={2} width={6} height={8} style={{ fill: `var(--viz-seq-${step})` }} />)}
      </svg>
    );
  }
  return (
    <svg width={16} height={12} viewBox="0 0 16 12" aria-hidden="true" className="shrink-0 overflow-visible">
      {kind === 'line' && <line x1={1} y1={6} x2={15} y2={6} style={{ stroke: colour, strokeWidth: 2, strokeLinecap: 'round' }} />}
      {kind === 'dashed-line' && <line x1={1} y1={6} x2={15} y2={6} style={{ stroke: colour, strokeWidth: 2, strokeDasharray: COMPARISON_DASH }} />}
      {kind === 'rect' && <rect x={2} y={2} width={12} height={8} rx={2} style={{ fill: colour }} />}
      {kind === 'dot' && <circle cx={8} cy={6} r={4} style={{ fill: colour }} />}
      {kind === 'hollow' && <circle cx={8} cy={6} r={3.5} style={{ fill: VIZ.surface, stroke: colour, strokeWidth: 1.5 }} />}
      {kind === 'ring' && <circle cx={8} cy={6} r={5} style={{ fill: 'none', stroke: colour, strokeWidth: 1.5 }} />}
      {kind === 'ring-dotted' && (
        <>
          <circle cx={8} cy={5} r={3.5} style={{ fill: 'none', stroke: colour, strokeWidth: 1.5 }} />
          <line x1={1} y1={11} x2={15} y2={11} style={{ stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: '1 2' }} />
        </>
      )}
      {kind === 'flag' && <path d="M4 11V1.5h8l-2 2.5 2 2.5H5.5" style={{ fill: colour, stroke: colour, strokeWidth: 1, strokeLinejoin: 'round' }} />}
      {kind === 'hatch' && (
        <>
          <defs>
            <pattern id={hatchId} width={4} height={4} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
              <line x1={0} y1={0} x2={0} y2={4} style={{ stroke: VIZ.mark, strokeWidth: 1.5 }} />
            </pattern>
          </defs>
          <rect x={2} y={1.5} width={12} height={9} style={{ fill: `url(#${hatchId})` }} />
        </>
      )}
      {kind === 'diamond' && <polygon points="8,1.5 12.5,6 8,10.5 3.5,6" style={{ fill: colour }} />}
      {kind === 'hollow-diamond' && <polygon points="8,1.75 12.25,6 8,10.25 3.75,6" style={{ fill: VIZ.surface, stroke: colour, strokeWidth: 1.5 }} />}
      {kind === 'triangle' && <polygon points="8,1.5 12.5,10 3.5,10" style={{ fill: colour }} />}
      {kind === 'tint' && (
        <>
          <rect x={2} y={2} width={12} height={8} rx={1.5} style={{ fill: VIZ.track, opacity: 0.6 }} />
          <rect x={2} y={2} width={12} height={8} rx={1.5} style={{ fill: colour, opacity: 0.12 }} />
        </>
      )}
      {kind === 'outline-rect' && <rect x={2.75} y={2.75} width={10.5} height={6.5} rx={1.5} style={{ fill: 'none', stroke: colour, strokeWidth: 1.5 }} />}
      {(kind === 'slot-dashed' || kind === 'slot-dotted') && (
        <rect x={2.5} y={2.5} width={11} height={7} rx={1.5} style={{ fill: 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: kind === 'slot-dashed' ? '3 2' : '1 2' }} />
      )}
    </svg>
  );
}

/**
 * The frame's legend. With `focus`, each series entry is a toggle: hover spotlights its series,
 * a click keeps it, Escape clears. Absence entries are explanations, never toggles.
 */
export function Legend({ items, focus }: { items: readonly LegendItem[]; focus?: LegendFocus }) {
  if (items.length === 0) return null;
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && focus?.focused) {
      event.stopPropagation();
      focus.clear();
    }
  };
  return (
    <ul className="flex flex-wrap items-center gap-x-4 gap-y-1 text-caption text-fg-secondary" aria-label="Legend" onKeyDown={onKeyDown}>
      {items.map((item) => {
        const interactive = focus && !item.key.startsWith('absence-');
        const content = (
          <>
            {item.icon ?? <LegendSwatch kind={item.swatch} tone={item.tone} />}
            <span>{item.label}</span>
          </>
        );
        return (
          <li key={item.key} className="flex items-center">
            {interactive ? (
              <button
                type="button"
                aria-pressed={focus.sticky === item.key}
                className="focus-ring -mx-1 flex h-6 items-center gap-1.5 rounded px-1 hover:text-fg"
                style={{ opacity: focus.opacityFor(item.key) === 1 ? 1 : 0.6 }}
                onPointerEnter={() => focus.hover(item.key)}
                onPointerLeave={() => focus.hover(null)}
                onClick={() => focus.toggle(item.key)}
              >
                {content}
              </button>
            ) : (
              <span className="flex h-6 items-center gap-1.5">{content}</span>
            )}
          </li>
        );
      })}
    </ul>
  );
}
