import { useRef } from 'react';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { HatchProvider, VizDefs, useHatchFill, useSvgId } from '../frame/VizDefs';
import { TooltipContent } from '../frame/Tooltip';
import { VIZ } from '../core/claim';
import { scaleLinear } from '../core/scale';
import { useChartSize } from '../hooks/useChartSize';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { LegendSwatch, type SwatchKind, type SwatchTone } from '../frame/Legend';
import { useTooltip } from '../hooks/useTooltip';
import { MarkGroup } from '../marks/shared';

/*
 * F14 DocumentMinimap: where the marked passages sit in the whole document, by character offset.
 * Each kind of mark has its own sub-column, so position and texture carry the type, not colour
 * alone: finding citations (violet, a lead the app surfaced), your annotations (ink), find matches
 * (outline) and redaction markers (hatch: text outside what can be read). A mark without a recorded
 * position is counted in words, never placed at the top as if it were offset 0.
 */

export type MinimapKind = 'finding' | 'annotation' | 'match' | 'redaction';

export interface MinimapMark {
  key: string;
  kind: MinimapKind;
  /** Source character offsets (code points) in the whole document; null when not recorded. */
  start: number | null;
  end: number | null;
  label: string;
  quote?: string | null;
}

const KIND_ORDER: readonly MinimapKind[] = ['finding', 'annotation', 'match', 'redaction'];
const KIND_WORDS: Record<MinimapKind, [string, string]> = {
  finding: ['cited passage', 'cited passages'],
  annotation: ['annotation', 'annotations'],
  match: ['match', 'matches'],
  redaction: ['redaction marker', 'redaction markers'],
};

function counted(n: number, kind: MinimapKind): string {
  return `${n.toLocaleString('en-US')} ${KIND_WORDS[kind][n === 1 ? 0 : 1]}`;
}

export function placedMarks(marks: readonly MinimapMark[]): MinimapMark[] {
  return marks.filter((mark) => mark.start !== null).sort((a, b) => (a.start as number) - (b.start as number) || KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind));
}

/** '3 cited passages, 1 annotation, 4 matches across 12,000 characters; 1 citation without a recorded position.' */
export function minimapHeadline(marks: readonly MinimapMark[], total: number): string {
  const placed = placedMarks(marks);
  const clauses = KIND_ORDER.map((kind) => [kind, placed.filter((mark) => mark.kind === kind).length] as const).filter(([, n]) => n > 0).map(([kind, n]) => counted(n, kind));
  const unplacedCitations = marks.filter((mark) => mark.start === null && mark.kind === 'finding').length;
  const unplacedAnnotations = marks.filter((mark) => mark.start === null && mark.kind === 'annotation').length;
  const missing = [
    unplacedCitations > 0 ? `${unplacedCitations.toLocaleString('en-US')} citation${unplacedCitations === 1 ? '' : 's'}` : null,
    unplacedAnnotations > 0 ? `${unplacedAnnotations.toLocaleString('en-US')} annotation${unplacedAnnotations === 1 ? '' : 's'}` : null,
  ].filter(Boolean).join(' and ');
  const head = clauses.length > 0 ? `${clauses.join(', ')} across ${total.toLocaleString('en-US')} characters` : `No marked passage across ${total.toLocaleString('en-US')} characters`;
  return `${head}${missing ? `; ${missing} without a recorded position` : ''}.`;
}

function MarkFill({ kind, x, y, width, height }: { kind: MinimapKind; x: number; y: number; width: number; height: number }) {
  const hatch = useHatchFill();
  if (kind === 'match') return <rect x={x + 0.5} y={y + 0.5} width={Math.max(0, width - 1)} height={Math.max(1, height - 1)} style={{ fill: VIZ.surface, stroke: VIZ.ink, strokeWidth: 1 }} />;
  if (kind === 'redaction') return <rect x={x} y={y} width={width} height={height} style={{ fill: hatch, stroke: VIZ.mark, strokeWidth: 0.75 }} />;
  return <rect x={x} y={y} width={width} height={height} rx={1} style={{ fill: kind === 'finding' ? VIZ.lead : VIZ.ink }} />;
}

function tip(mark: MinimapMark) {
  return <TooltipContent title={mark.label} rows={[{ key: 'at', value: (mark.start as number).toLocaleString('en-US'), label: 'characters in' }]} note={mark.quote ? `“${mark.quote.slice(0, 80)}${mark.quote.length > 80 ? '…' : ''}”` : undefined} />;
}

const KIND_SWATCH: Record<MinimapKind, { swatch: SwatchKind; tone: SwatchTone; label: string }> = {
  finding: { swatch: 'rect', tone: 'lead', label: 'finding citation' },
  annotation: { swatch: 'rect', tone: 'ink', label: 'annotation or case citation' },
  match: { swatch: 'outline-rect', tone: 'ink', label: 'find match' },
  redaction: { swatch: 'hatch', tone: 'mark', label: 'redaction marker' },
};

/** The gutter's key: one swatch per kind present, in sub-column order left to right. */
export function MinimapKey({ marks }: { marks: readonly MinimapMark[] }) {
  const present = KIND_ORDER.filter((kind) => marks.some((mark) => mark.kind === kind && mark.start !== null));
  if (present.length === 0) return null;
  return (
    <ul aria-label="Marks beside the text, by column" className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-caption text-muted" data-testid="minimap-key">
      <li>Marks beside the text, left to right:</li>
      {present.map((kind) => (
        <li key={kind} className="flex items-center gap-1">
          <LegendSwatch kind={KIND_SWATCH[kind].swatch} tone={KIND_SWATCH[kind].tone} />
          {KIND_SWATCH[kind].label}
        </li>
      ))}
    </ul>
  );
}

/**
 * The viewer's vertical gutter: height is the whole document, a translucent band is the loaded
 * text, and each mark is a tick in its kind's sub-column. A navigation landmark with one tab stop;
 * the arrow keys move through the marks in document order and Enter opens one.
 */
export function MinimapGutter({ marks, total, loaded, activeKey, onActivate }: {
  marks: readonly MinimapMark[];
  total: number;
  /** The loaded window, in source characters. */
  loaded: { start: number; end: number };
  activeKey?: string | null;
  onActivate(mark: MinimapMark): void;
}) {
  const box = useRef<HTMLDivElement>(null);
  const { height } = useChartSize(box, { fallbackWidth: 16 });
  const hatchId = useSvgId('viz-minimap-hatch');
  const tooltip = useTooltip();
  const placed = placedMarks(marks);
  const roving = useRovingFocus({ shape: { rows: Math.max(1, placed.length) }, onActivate: (i) => placed[i] && onActivate(placed[i]), onEscape: tooltip.hide });
  const width = 16;
  const column = 3;
  const y = scaleLinear([0, Math.max(1, total)], [2, Math.max(4, height - 2)], { nice: false });
  return (
    <nav aria-label="Marks in this document" className="relative w-4 shrink-0 border-l border-line" style={{ background: 'var(--viz-surface)' }}>
      <div ref={box} className="absolute inset-0">
        {height > 0 && (
          <svg width={width} height={height} role="group" aria-label={`${placed.length.toLocaleString('en-US')} marks in document order`} className="block overflow-visible">
            <VizDefs hatchId={hatchId} />
            <HatchProvider value={hatchId}>
              <rect x={0} y={y(loaded.start)} width={width} height={Math.max(2, y(loaded.end) - y(loaded.start))} style={{ fill: VIZ.ink, opacity: 0.08 }} />
              {placed.map((mark, index) => {
                const top = y(mark.start as number);
                const bottom = y(Math.max(mark.start as number, mark.end ?? (mark.start as number)));
                const left = 1 + KIND_ORDER.indexOf(mark.kind) * (column + 0.5);
                const tickHeight = Math.max(2, bottom - top);
                return (
                  <MarkGroup
                    key={mark.key}
                    box={{ x: left, y: top, width: column, height: tickHeight }}
                    interaction={{
                      name: `${mark.label}, at character ${(mark.start as number).toLocaleString('en-US')}`,
                      item: roving.getItemProps(index),
                      onPointerMove: (event) => tooltip.show(tip(mark), { clientX: event.clientX, clientY: event.clientY }),
                      onPointerLeave: tooltip.hide,
                      onFocusMark: (focusBox) => tooltip.show(tip(mark), focusBox),
                      onBlurMark: tooltip.hide,
                      onClick: () => onActivate(mark),
                    }}
                  >
                    <MarkFill kind={mark.kind} x={left} y={top} width={column} height={tickHeight} />
                    {activeKey === mark.key && <rect x={0.5} y={top - 1.5} width={width - 1} height={tickHeight + 3} rx={1} pointerEvents="none" style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 1.5 }} />}
                  </MarkGroup>
                );
              })}
            </HatchProvider>
          </svg>
        )}
      </div>
      {tooltip.node}
    </nav>
  );
}

function StripPlot({ marks, total, loaded, width, focus }: { marks: readonly MinimapMark[]; total: number; loaded: { start: number; end: number } | null; width: number; focus: { start: number; end: number } | null }) {
  const tooltip = useTooltip();
  const x = scaleLinear([0, Math.max(1, total)], [8, width - 8], { nice: false });
  const placed = placedMarks(marks);
  const row = (kind: MinimapKind) => 8 + KIND_ORDER.indexOf(kind) * 6;
  return (
    <g>
      <rect x={x(0)} y={6} width={x(total) - x(0)} height={28} rx={2} style={{ fill: VIZ.track, opacity: 0.6 }} />
      {loaded && <rect x={x(loaded.start)} y={6} width={Math.max(2, x(loaded.end) - x(loaded.start))} height={28} style={{ fill: VIZ.ink, opacity: 0.08 }} />}
      {placed.map((mark) => {
        const left = x(mark.start as number);
        const tickWidth = Math.max(2, x(Math.max(mark.start as number, mark.end ?? (mark.start as number))) - left);
        return (
          <MarkGroup key={mark.key} box={{ x: left, y: row(mark.kind), width: tickWidth, height: 4 }} interaction={{ name: mark.label, onPointerMove: (event) => tooltip.show(tip(mark), { clientX: event.clientX, clientY: event.clientY }), onPointerLeave: tooltip.hide }}>
            <MarkFill kind={mark.kind} x={left} y={row(mark.kind)} width={tickWidth} height={4} />
          </MarkGroup>
        );
      })}
      {focus && (
        <g data-focus-span="">
          <rect x={x(focus.start) - 1} y={3} width={Math.max(3, x(focus.end) - x(focus.start)) + 2} height={34} rx={2} style={{ fill: 'none', stroke: VIZ.ink, strokeWidth: 1.5 }} />
          <text x={Math.min(width - 4, Math.max(4, x(focus.start)))} y={48} dy="0.32em" textAnchor={x(focus.start) > width / 2 ? 'end' : 'start'} className="fill-fg-secondary text-[11px]">this citation</text>
        </g>
      )}
      <Axis scale={x} kind="count" orient="bottom" offset={58} tickCount={3} />
      <text x={width - 8} y={84} dy="0.32em" textAnchor="end" className="fill-muted text-[11px]">characters</text>
      {tooltip.node}
    </g>
  );
}

/** The inspector's one-row strip: where a cited span falls in the whole document. */
export default function DocumentMinimap({ marks, total, loaded, focus, storageKey }: {
  marks: readonly MinimapMark[];
  total: number;
  loaded: { start: number; end: number } | null;
  /** The span the inspector was opened on; null when the citation recorded no position. */
  focus: { start: number; end: number } | null;
  storageKey: string;
}) {
  const headline = focus
    ? `This citation sits at characters ${focus.start.toLocaleString('en-US')} to ${focus.end.toLocaleString('en-US')} of ${total.toLocaleString('en-US')}. ${minimapHeadline(marks, total)}`
    : `This citation recorded no position in the document. ${minimapHeadline(marks, total)}`;
  const placed = placedMarks(marks);
  return (
    <ChartFrame
      title="Where it falls in the document"
      headline={headline}
      sources={{ tables: ['documents', 'findings'], note: 'positions are character offsets in the stored text' }}
      summary={headline}
      status={total > 0 ? 'ready' : 'empty'}
      emptyText="The stored text is empty, so no position can be shown."
      table={{
        caption: 'Marked passages by position',
        columns: [{ key: 'kind', label: 'Mark' }, { key: 'label', label: 'Label' }, { key: 'start', label: 'Start', kind: 'count' }, { key: 'end', label: 'End', kind: 'count' }],
        rows: [
          ...(focus ? [{ kind: 'This citation', label: '', start: focus.start, end: focus.end }] : []),
          ...placed.map((mark) => ({ kind: KIND_WORDS[mark.kind][0], label: mark.label, start: mark.start, end: mark.end ?? mark.start })),
          ...marks.filter((mark) => mark.start === null).map((mark) => ({ kind: KIND_WORDS[mark.kind][0], label: mark.label, start: { state: 'not-recorded' as const, reason: 'no recorded position' }, end: null })),
        ],
      }}
      legend={[
        ...(focus ? [{ key: 'focus', label: 'This citation', swatch: 'ring' as const, tone: 'ink' as const }] : []),
        ...(placed.some((mark) => mark.kind === 'finding') ? [{ key: 'finding', label: 'Finding citations', swatch: 'rect' as const, tone: 'lead' as const }] : []),
        { key: 'document', label: 'Whole document', swatch: 'rect', tone: 'track' },
        ...(loaded ? [{ key: 'loaded', label: 'Text shown in context below', swatch: 'tint' as const, tone: 'ink' as const }] : []),
      ]}
      minHeight={92}
      storageKey={storageKey}
    >
      {({ width }) => <StripPlot marks={marks} total={total} loaded={loaded} width={width} focus={focus} />}
    </ChartFrame>
  );
}
