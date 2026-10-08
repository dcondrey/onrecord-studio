import { useMemo, type CSSProperties, type ReactNode } from 'react';
import { ShapeGlyph, type GlyphShape } from '../../ui/shapeGlyph';
import { VIZ, type Claim } from '../core/claim';
import { fitsInside, measureText } from '../core/layout';
import { ABSENCE_LABEL, type Absence } from '../core/nulls';
import ChartFrame, { type ChartStatus } from '../frame/ChartFrame';
import type { LegendItem } from '../frame/Legend';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinCell, TwinColumn } from '../frame/TableTwin';
import { TooltipContent, type TooltipRow } from '../frame/Tooltip';
import { useHatchFill } from '../frame/VizDefs';
import { useRovingFocus, type RovingItemProps } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import type { SeqStep } from '../marks/HeatCell';
import { MIN_HIT, MUTED_LABEL_CLASS } from '../marks/shared';

/** A cell glyph: the shared shape vocabulary, plus a check for 'observed and unchanged'. */
export type StateGlyph = GlyphShape | 'check';

interface CellBase {
  /** Words printed in the cell when they fit ('3 changed', 'not observed'). */
  text: string;
  /**
   * A shorter form printed when `text` does not fit: the bare count ('3', '480K') or a 1–3
   * character code the legend spells out. Without either fitting, the words move to the title.
   */
  short?: string;
  /** The cell's accessible name: a full sentence. */
  name: string;
  glyph: StateGlyph;
  /** The legend entry for this cell's look; cells sharing a label share one entry. */
  legend?: string;
  /** The table twin's words; default `text`. */
  tableText?: string;
  tip?: { rows: TooltipRow[]; note?: string };
  /** Enter, Space or a click on this cell does something. */
  actionable?: boolean;
  /** Columns this cell covers, from its own; the row's entries it covers are `null`. Default 1. */
  span?: number;
}

/**
 * - state: an observed state. `lead` is a violet fill (a lead the app surfaced), `positive` a teal
 *   glyph on a neutral cell (observed and healthy), `neutral` an outline, `quiet` bare words (a
 *   measured zero in a count grid, left unshaded so it does not read as a small count).
 * - count: a measured count on the sequential slate ramp.
 * - gap: an absence, drawn as its kind of empty cell. With `lead` the empty cell is outlined in
 *   violet: a record the app found missing and worth asking for.
 */
export type StateGridCell =
  | (CellBase & { kind: 'state'; tone: 'lead' | 'positive' | 'neutral' | 'quiet' })
  | (CellBase & { kind: 'count'; value: number; step: SeqStep })
  | (CellBase & { kind: 'gap'; absence: Absence; lead?: boolean });

export interface StateGridColumn {
  key: string;
  label: string;
  /** A header glyph (finding severity shapes). */
  glyph?: GlyphShape;
}

export interface StateGridRow {
  key: string;
  label: string;
  /** A second, muted line under the label (a watch's kind). */
  sublabel?: string;
  /** Consecutive rows with the same group print one group heading above them. */
  group?: string;
  /** The right-hand column: words, and an optional thin bar of `value` of `max`. */
  trailing?: { text: string; value?: number; max?: number };
  /** One per column; `null` where a cell to the left spans over it. */
  cells: (StateGridCell | null)[];
}

export interface StateGridProps {
  title: string;
  headline: string | null;
  claim?: Claim;
  sources: ChartSources;
  summary: string;
  status: ChartStatus;
  emptyText?: string;
  error?: string | null;
  /** The row-header column's name in the table twin and the grid ('Watch', 'Table'). */
  rowHeader: string;
  trailingHeader?: string;
  columns: StateGridColumn[];
  rows: StateGridRow[];
  /** Entries the cells cannot state themselves (a ramp's meaning); cell entries follow. */
  legend?: LegendItem[];
  controls?: ReactNode;
  storageKey: string;
  /** The selected cells, outlined in ink (selection never uses violet). */
  selected?: readonly { row: number; col: number }[];
  onActivate?(row: number, col: number): void;
  /** Makes the column headers the grid's first row of toggles. */
  onColumnActivate?(col: number): void;
  /** The column whose header toggle is on. */
  pressedColumn?: number | null;
  onEscape?(): void;
  className?: string;
}

const ROW_HEIGHT = 30;
const ROW_HEIGHT_TWO_LINES = 38;
const STACKED_LABEL = 16;
const GROUP_HEIGHT = 22;
const HEADER_FLAT = 24;
const HEADER_MAX_LABEL = 112;
const HEADER_ANGLE = 40;
const COS = Math.cos((HEADER_ANGLE * Math.PI) / 180);
const SIN = Math.sin((HEADER_ANGLE * Math.PI) / 180);
/** Below this width a row's label moves onto its own line above its cells. */
const STACK_BELOW = 420;
const TRAILING_MAX = 110;
const TRAILING_MIN = 44;
const CELL_MAX = 132;
const CELL_CAP_FOR_WORDS = 72;
/** Glyph centre and text start inside a cell, from its left edge. */
const GLYPH_X = 8;
const TEXT_X = 15;
const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';
const ROW_FONT = '12px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';

/** Shortens a label to `width` px, adding an ellipsis; the full label stays in the <title>. */
function clip(text: string, width: number, font?: string): string {
  if (measureText(text, font) <= width) return text;
  let end = text.length;
  while (end > 1 && measureText(`${text.slice(0, end)}…`, font) > width) end -= 1;
  return `${text.slice(0, end)}…`;
}

/** Whether words fit a cell of inner width `w` beside its glyph. */
function wordsFit(words: string, w: number): boolean {
  return measureText(words) + TEXT_X + 4 <= w;
}

/** The words a cell prints at inner width `w`: full, then short, then none (title only). */
export function cellWords(cell: StateGridCell, w: number): string | null {
  if (wordsFit(cell.text, w)) return cell.text;
  if (cell.short && wordsFit(cell.short, w)) return cell.short;
  return null;
}

/** The words CellMark prints in a cell box of `width` × `height`, or null when it prints none. */
function drawnWords(cell: StateGridCell, width: number, height: number): string | null {
  return height - 2 >= 14 ? cellWords(cell, Math.max(0, width - 2)) : null;
}

/**
 * A cell's accessible name. Voice control addresses a control by the words it shows, so a name
 * that does not already contain the printed words leads with them.
 */
function cellName(name: string, words: string | null): string {
  return words && !name.toLowerCase().includes(words.toLowerCase()) ? `${words}: ${name}` : name;
}

interface Layout {
  stacked: boolean;
  gridLeft: number;
  labelWidth: number;
  trailingWidth: number;
  cellWidth: number;
  headerHeight: number;
  rotated: boolean;
  rowY: number[];
  groupY: { label: string; y: number }[];
  rowHeight: number;
  /** Space above the cells in a row: the stacked label line. */
  cellTop: number;
  height: number;
}

function layoutFor(width: number, props: StateGridProps): Layout {
  const { rows, columns } = props;
  const cols = Math.max(1, columns.length);
  const stacked = width < STACK_BELOW;
  const twoLines = !stacked && rows.some((row) => row.sublabel);
  const cellTop = stacked ? STACKED_LABEL : 0;
  const rowHeight = (twoLines ? ROW_HEIGHT_TWO_LINES : ROW_HEIGHT) + cellTop;
  const widest = Math.max(0, ...rows.map((row) => Math.max(measureText(row.label, ROW_FONT), row.sublabel ? measureText(row.sublabel) : 0)));
  // Cells are sized first: the narrowest a cell may be is what its short words need (a cell's
  // inner width is 2px less, its words start at TEXT_X and keep 4px clear), and the trailing
  // column, then the row labels (clipped, full in their titles), give way to that.
  const minCell = Math.min(CELL_CAP_FOR_WORDS, Math.max(24, ...rows.flatMap((row) => row.cells.flatMap((cell) => (cell && (cell.span ?? 1) === 1 ? [Math.ceil(measureText(cell.short ?? cell.text)) + TEXT_X + 7] : [])))));
  const trailingNeed = rows.some((row) => row.trailing)
    ? Math.round(Math.min(TRAILING_MAX, Math.max(40, ...rows.map((row) => (row.trailing ? measureText(row.trailing.text) : 0))) + 16))
    : 0;
  const naturalLabel = Math.round(Math.max(88, Math.min(widest + 12, width * 0.32, 240)));
  const labelWidth = stacked ? 0 : Math.round(Math.max(88, Math.min(naturalLabel, width - Math.min(trailingNeed, TRAILING_MIN) - minCell * cols)));
  const longest = Math.min(HEADER_MAX_LABEL, Math.max(0, ...columns.map((column) => measureText(column.label))));
  const size = (gridLeft: number) => {
    let trailingWidth = trailingNeed;
    if (trailingWidth > 0 && (width - gridLeft - trailingWidth) / cols < minCell) {
      trailingWidth = Math.round(Math.max(Math.min(trailingNeed, TRAILING_MIN), width - gridLeft - minCell * cols));
    }
    const cellWidth = Math.max(24, Math.min(CELL_MAX, (width - gridLeft - trailingWidth) / cols));
    const headerRoom = cellWidth - (columns.some((column) => column.glyph) ? 18 : 6);
    const rotated = columns.some((column) => !fitsInside(column.label, headerRoom));
    return { trailingWidth, cellWidth, rotated };
  };
  let gridLeft = labelWidth;
  let sized = size(gridLeft);
  // A rotated header leans up and to the left of its column's centre, so the first column's label
  // needs that much room before the grid; the label column gives it, or, stacked, a left margin.
  // +6: the header is drawn medium weight, a little wider than measured.
  const lean = sized.rotated ? Math.ceil(COS * longest - sized.cellWidth / 2) + 6 : 0;
  if (lean > gridLeft) {
    gridLeft = lean;
    sized = size(gridLeft);
  }
  const headerHeight = sized.rotated ? Math.ceil(longest * SIN) + 23 : HEADER_FLAT;
  const rowY: number[] = [];
  const groupY: { label: string; y: number }[] = [];
  let y = headerHeight;
  rows.forEach((row, index) => {
    if (row.group && row.group !== rows[index - 1]?.group) {
      groupY.push({ label: row.group, y });
      y += GROUP_HEIGHT;
    }
    rowY.push(y);
    y += rowHeight;
  });
  return { stacked, gridLeft, labelWidth, ...sized, headerHeight, rowY, groupY, rowHeight, cellTop, height: y + 4 };
}

const DASH: Record<Absence['state'], string | undefined> = { 'not-recorded': '3 2', 'not-covered': '1 2', excluded: undefined };

interface Paint {
  rect: CSSProperties;
  /** Whether the rect is stroked, so it is inset by half the stroke. */
  stroked: boolean;
  glyph: string;
  text: string;
  tone?: string;
}

/** One paint per cell look, shared by the grid and its legend swatches so each swatch is the drawn mark. */
function paintOf(cell: StateGridCell, hatch: string): Paint {
  if (cell.kind === 'count') {
    return { rect: { fill: `var(--viz-seq-${cell.step})` }, stroked: false, glyph: `var(--viz-on-seq-${cell.step})`, text: `var(--viz-on-seq-${cell.step})` };
  }
  if (cell.kind === 'gap') {
    if (cell.lead) {
      return { rect: { fill: 'none', stroke: VIZ.lead, strokeWidth: 1.5, strokeDasharray: DASH[cell.absence.state] }, stroked: true, glyph: VIZ.lead, text: 'rgb(var(--fg))', tone: 'lead' };
    }
    return {
      rect: { fill: cell.absence.state === 'excluded' ? hatch : 'none', stroke: VIZ.baseline, strokeWidth: 1, strokeDasharray: DASH[cell.absence.state] },
      stroked: true, glyph: 'rgb(var(--muted))', text: 'rgb(var(--muted))',
    };
  }
  switch (cell.tone) {
    case 'lead':
      return { rect: { fill: VIZ.lead }, stroked: false, glyph: 'var(--viz-on-color)', text: 'var(--viz-on-color)', tone: 'lead' };
    case 'positive':
      return { rect: { fill: VIZ.track }, stroked: false, glyph: VIZ.positive, text: 'rgb(var(--fg))', tone: 'positive' };
    case 'quiet':
      return { rect: { fill: 'none' }, stroked: false, glyph: VIZ.context, text: 'rgb(var(--muted))', tone: 'quiet' };
    case 'neutral':
      return { rect: { fill: 'none', stroke: VIZ.mark, strokeWidth: 1 }, stroked: true, glyph: VIZ.ink, text: 'rgb(var(--fg))', tone: 'neutral' };
  }
}

function CellGlyph({ glyph, cx, cy, colour }: { glyph: StateGlyph; cx: number; cy: number; colour: string }) {
  if (glyph === 'check') {
    return <path d={`M${cx - 4},${cy} l2.8,3 l5.2,-6`} style={{ fill: 'none', stroke: colour, strokeWidth: 2, strokeLinecap: 'round', strokeLinejoin: 'round' }} />;
  }
  return <ShapeGlyph shape={glyph} cx={cx} cy={cy} radius={3.5} style={glyph === 'ring' ? { fill: 'none', stroke: colour, strokeWidth: 1.5 } : { fill: colour }} />;
}

/**
 * A gap's words with its reason. A gap with its own legend label says what it is in its own words
 * ('not expected: …'); only an unlabelled gap falls back to the generic absence label.
 */
function gapText(cell: StateGridCell & { kind: 'gap' }, reason = cell.absence.reason): string {
  return `${cell.legend ? cell.text : ABSENCE_LABEL[cell.absence.state]}: ${reason}`;
}

function cellTitle(cell: StateGridCell, words: string | null): string | null {
  const parts = cell.kind === 'gap' ? [gapText(cell)] : [words === cell.text ? null : cell.text];
  if (cell.kind === 'gap' && !cell.legend && words !== cell.text) parts.unshift(cell.text);
  const shown = parts.filter(Boolean);
  if (shown.length === 0) return null;
  return shown.join('. ');
}

/** One cell's drawing: its fill or empty outline, its glyph, and its words (full, short, or title). */
function CellMark({ cell, x, y, width, height }: { cell: StateGridCell; x: number; y: number; width: number; height: number }) {
  const hatch = useHatchFill();
  const paint = paintOf(cell, hatch);
  const w = Math.max(0, width - 2);
  const h = Math.max(0, height - 2);
  const words = drawnWords(cell, width, height);
  const title = cellTitle(cell, words);
  const inset = paint.stroked ? 0.75 : 0;
  return (
    <g>
      {title && <title>{title}</title>}
      <rect
        x={x + 1 + inset}
        y={y + 1 + inset}
        width={Math.max(0, w - 2 * inset)}
        height={Math.max(0, h - 2 * inset)}
        rx={2}
        data-tone={paint.tone}
        data-absence={cell.kind === 'gap' ? cell.absence.state : undefined}
        style={paint.rect}
      />
      {h >= 10 && <CellGlyph glyph={cell.glyph} cx={x + 1 + (words ? GLYPH_X : w / 2)} cy={y + 1 + h / 2} colour={paint.glyph} />}
      {words && (
        <text x={x + 1 + TEXT_X} y={y + 1 + h / 2} dy="0.32em" className="viz-num text-[11px]" style={{ fill: paint.text }}>{words}</text>
      )}
    </g>
  );
}

/** A legend key drawn as a small copy of the cell it explains. */
function CellSwatch({ cell }: { cell: StateGridCell }) {
  const paint = paintOf(cell, 'none');
  return (
    <svg width={22} height={14} viewBox="0 0 22 14" aria-hidden="true" className="shrink-0 overflow-visible">
      <rect x={paint.stroked ? 0.75 : 0} y={paint.stroked ? 0.75 : 0} width={paint.stroked ? 20.5 : 22} height={paint.stroked ? 12.5 : 14} rx={2} style={paint.rect} />
      <CellGlyph glyph={cell.glyph} cx={11} cy={7} colour={paint.glyph} />
    </svg>
  );
}

/** One legend entry per distinct cell label, in first-seen order; an unlabelled gap says its absence. */
function cellLegend(rows: readonly StateGridRow[]): LegendItem[] {
  const seen = new Map<string, StateGridCell>();
  for (const row of rows) {
    for (const cell of row.cells) {
      if (!cell) continue;
      const label = cell.legend ?? (cell.kind === 'gap' ? ABSENCE_LABEL[cell.absence.state] : null);
      if (label && !seen.has(label)) seen.set(label, cell);
    }
  }
  return [...seen].map(([label, cell]) => ({ key: `cell-${label}`, label, swatch: 'rect', icon: <CellSwatch cell={cell} /> }));
}

/** The table twin's text for a cell: an absence prints its state and reason, never a blank. */
function twinCell(cell: StateGridCell): TwinCell {
  if (cell.kind === 'gap') {
    const reason = cell.tableText ?? cell.absence.reason;
    return cell.legend ? gapText(cell, reason) : { ...cell.absence, reason };
  }
  return cell.tableText ?? cell.text;
}

interface CellShellProps {
  name: string;
  item: RovingItemProps;
  box: { x: number; y: number; width: number; height: number };
  selected?: boolean;
  actionable?: boolean;
  onShow(at: DOMRect | { clientX: number; clientY: number }): void;
  onHide(): void;
  onClick?(): void;
  children: ReactNode;
}

/**
 * The focusable grid cell itself: its name, roving tab stop, hit area and focus ring live on the
 * role="gridcell" element, so a screen reader announces it with its row and column.
 */
function CellShell({ name, item, box, selected, actionable, onShow, onHide, onClick, children }: CellShellProps) {
  const hitHeight = Math.max(box.height, MIN_HIT);
  return (
    <g
      role="gridcell"
      aria-label={name}
      aria-selected={selected || undefined}
      className={actionable ? 'viz-focusable cursor-pointer' : 'viz-focusable'}
      tabIndex={item.tabIndex}
      ref={item.ref}
      data-focused={item['data-focused']}
      onKeyDown={item.onKeyDown}
      onFocus={(event) => {
        item.onFocus();
        onShow(event.currentTarget.getBoundingClientRect());
      }}
      onBlur={() => {
        item.onBlur();
        onHide();
      }}
      onPointerMove={(event) => onShow({ clientX: event.clientX, clientY: event.clientY })}
      onPointerLeave={onHide}
      onClick={onClick}
    >
      <rect x={box.x} y={box.y + (box.height - hitHeight) / 2} width={box.width} height={hitHeight} fill="transparent" pointerEvents="all" />
      {children}
      {selected && <rect x={box.x + 0.5} y={box.y + 0.5} width={box.width - 1} height={box.height - 1} rx={3} pointerEvents="none" style={{ fill: 'none', stroke: VIZ.ink, strokeWidth: 2 }} />}
      <rect className="viz-focus-ring" x={box.x - 2} y={box.y - 2} width={box.width + 4} height={box.height + 4} rx={4} pointerEvents="none" style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }} />
    </g>
  );
}

/**
 * F8: rows × columns of states. Every cell carries a glyph and words (in full, as a short form, or
 * in its title when neither fits), so no state rests on colour; absence is drawn as its kind of
 * empty cell, never as a zero. The grid is one tab stop, moved through with the arrow keys in two
 * dimensions; with column toggles the headers are its first row.
 */
export default function StateGrid(props: StateGridProps) {
  const { rows, columns, selected, onActivate, onColumnActivate, onEscape } = props;
  const headerRow = onColumnActivate ? 1 : 0;
  const tooltip = useTooltip();
  const roving = useRovingFocus({
    shape: { rows: Math.max(1, rows.length + headerRow), cols: Math.max(1, columns.length) },
    onActivate: (i, j = 0) => {
      if (headerRow && i === 0) onColumnActivate?.(j);
      else {
        const cells = rows[i - headerRow]?.cells ?? [];
        // A spanned column activates the cell spanning it.
        let k = j;
        while (k > 0 && cells[k] == null) k -= 1;
        if (cells[k]?.actionable && k + (cells[k]?.span ?? 1) > j) onActivate?.(i - headerRow, k);
      }
    },
    onEscape: () => {
      tooltip.hide();
      onEscape?.();
    },
  });

  const table = useMemo(() => {
    const twinColumns: TwinColumn[] = [
      { key: 'row', label: props.rowHeader },
      ...columns.map((column) => ({ key: `c:${column.key}`, label: column.label })),
      ...(props.trailingHeader ? [{ key: 'trailing', label: props.trailingHeader }] : []),
    ];
    return {
      caption: props.headline ?? props.title,
      columns: twinColumns,
      rows: rows.map((row) => {
        const record: Record<string, TwinCell> = { row: row.group ? `${row.group} · ${row.label}` : row.label };
        let spanning: StateGridCell | null = null;
        columns.forEach((column, j) => {
          const cell = row.cells[j] ?? null;
          if (cell) spanning = (cell.span ?? 1) > 1 ? cell : null;
          // A spanned column repeats the cell spanning it: the same file covers it.
          const shown = cell ?? spanning;
          record[`c:${column.key}`] = shown ? twinCell(shown) : null;
        });
        if (props.trailingHeader) record.trailing = row.trailing?.text ?? null;
        return record;
      }),
    };
  }, [columns, rows, props.headline, props.rowHeader, props.title, props.trailingHeader]);

  const legend = useMemo(() => [...(props.legend ?? []), ...cellLegend(rows)], [props.legend, rows]);

  /** One focus target over every column a cell spans: it answers the keys for whichever is active. */
  const itemFor = (i: number, j: number, span: number): RovingItemProps => {
    if (span <= 1) return roving.getItemProps(i, j);
    const items = Array.from({ length: span }, (_, k) => roving.getItemProps(i, j + k));
    const active = items.find((item) => item.tabIndex === 0) ?? items[0];
    return {
      ...active,
      ref: (element) => items.forEach((item) => item.ref(element)),
      'data-focused': items.some((item) => item['data-focused']) ? true : undefined,
    };
  };

  const tip = (row: StateGridRow, column: StateGridColumn, cell: StateGridCell) => (
    <TooltipContent title={`${row.label} · ${column.label}`} rows={cell.tip?.rows ?? []} note={cell.tip?.note ?? cell.name} />
  );

  return (
    <>
      <ChartFrame
        title={props.title}
        headline={props.headline}
        claim={props.claim}
        sources={props.sources}
        summary={props.summary}
        status={rows.length === 0 && props.status === 'ready' ? 'empty' : props.status}
        emptyText={props.emptyText}
        error={props.error}
        table={table}
        legend={legend}
        autoAbsenceLegend={false}
        controls={props.controls}
        storageKey={props.storageKey}
        className={props.className}
        minHeight={layoutFor(1600, props).height}
        plotHeight={(width) => layoutFor(width, props).height}
        interactive={{ roledescription: 'state grid' }}
      >
        {({ width }) => {
          const layout = layoutFor(width, props);
          const { gridLeft, cellWidth, rowHeight, headerHeight, cellTop } = layout;
          const gridRight = gridLeft + cellWidth * columns.length;
          return (
            <g role="grid" aria-label={props.title} aria-rowcount={rows.length + 1} aria-colcount={columns.length + 1}>
              <g role="row">
                {/* Stacked under rotated headers there is no label column for it to sit in: it stays a named header. */}
                <g role="columnheader" aria-label={props.rowHeader}>{!(layout.stacked && layout.rotated) && <text x={0} y={headerHeight - 8} className={MUTED_LABEL_CLASS}>{props.rowHeader}</text>}</g>
                {columns.map((column, j) => {
                  const cx = gridLeft + j * cellWidth;
                  const label = clip(column.label, layout.rotated ? HEADER_MAX_LABEL : cellWidth - (column.glyph ? 18 : 6));
                  const textX = layout.rotated ? cx + cellWidth / 2 : cx + (column.glyph ? 16 : 4);
                  const textY = headerHeight - (layout.rotated ? 15 : 8);
                  const pressed = props.pressedColumn === j;
                  const content = (
                    <>
                      <title>{column.label}</title>
                      {column.glyph && <ShapeGlyph shape={column.glyph} cx={layout.rotated ? cx + cellWidth / 2 : cx + 8} cy={headerHeight - (layout.rotated ? 7 : 12)} radius={3.5} style={column.glyph === 'ring' ? { fill: 'none', stroke: VIZ.ink, strokeWidth: 1.5 } : { fill: VIZ.ink }} />}
                      {/* Rotated labels end at the column's centre and lean up-left, over the grid and the label column, never past the right edge. */}
                      <text
                        x={textX}
                        y={textY}
                        textAnchor={layout.rotated ? 'end' : undefined}
                        transform={layout.rotated ? `rotate(${HEADER_ANGLE} ${textX} ${textY})` : undefined}
                        className={pressed ? 'fill-fg text-[11px] font-semibold' : 'fill-fg-secondary text-[11px] font-medium'}
                      >
                        {label}
                      </text>
                      {pressed && <rect x={cx + 2} y={headerHeight - 1.5} width={cellWidth - 4} height={2} rx={1} style={{ fill: VIZ.ink }} />}
                    </>
                  );
                  if (!onColumnActivate) return <g key={column.key} role="columnheader" aria-label={column.label}>{content}</g>;
                  const item = roving.getItemProps(0, j);
                  return (
                    <g key={column.key} role="columnheader" aria-label={column.label}>
                      <g
                        role="button"
                        aria-pressed={pressed}
                        aria-label={`Filter by ${column.label}`}
                        className="viz-focusable cursor-pointer"
                        tabIndex={item.tabIndex}
                        ref={item.ref}
                        data-focused={item['data-focused']}
                        onKeyDown={item.onKeyDown}
                        onFocus={item.onFocus}
                        onBlur={item.onBlur}
                        onClick={() => onColumnActivate(j)}
                      >
                        <rect x={cx} y={0} width={cellWidth} height={headerHeight} fill="transparent" />
                        {content}
                        <rect className="viz-focus-ring" x={cx + 1} y={1} width={cellWidth - 2} height={headerHeight - 2} rx={3} style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }} />
                      </g>
                    </g>
                  );
                })}
                {props.trailingHeader && layout.trailingWidth > 0 && (
                  <g role="columnheader"><text x={gridRight + 8} y={headerHeight - 8} className={MUTED_LABEL_CLASS}>{clip(props.trailingHeader, layout.trailingWidth - 8)}</text></g>
                )}
              </g>
              {layout.groupY.map((group) => (
                <text key={`group-${group.label}`} x={0} y={group.y + GROUP_HEIGHT / 2 + 2} dy="0.32em" className="fill-fg text-[11px] font-semibold" aria-hidden="true">{clip(group.label, width - 4)}</text>
              ))}
              {rows.map((row, i) => {
                const y = layout.rowY[i];
                const cellsY = y + cellTop;
                const cellsHeight = rowHeight - cellTop;
                const labelY = layout.stacked ? y + STACKED_LABEL / 2 + 1 : row.sublabel ? y + rowHeight / 2 - 6 : y + rowHeight / 2;
                const stackedLabel = row.sublabel ? `${row.label} · ${row.sublabel}` : row.label;
                const rowName = [row.group, row.label, row.sublabel].filter(Boolean).join(', ');
                return (
                  <g key={row.key} role="row">
                    <g role="rowheader" aria-label={rowName}>
                      {layout.stacked ? (
                        <text x={0} y={labelY} dy="0.32em" className={ROW_LABEL_CLASS}>
                          <title>{stackedLabel}</title>
                          {clip(stackedLabel, width - 4, ROW_FONT)}
                        </text>
                      ) : (
                        <>
                          <text x={0} y={labelY} dy="0.32em" className={ROW_LABEL_CLASS}>
                            <title>{row.label}</title>
                            {clip(row.label, layout.labelWidth - 10, ROW_FONT)}
                          </text>
                          {row.sublabel && <text x={0} y={labelY + 14} dy="0.32em" className={MUTED_LABEL_CLASS}>{clip(row.sublabel, layout.labelWidth - 10)}</text>}
                        </>
                      )}
                    </g>
                    {columns.map((column, j) => {
                      const cell = row.cells[j];
                      const x = gridLeft + j * cellWidth;
                      if (!cell) {
                        const covered = row.cells.slice(0, j).some((left, k) => left && k + (left.span ?? 1) > j);
                        return covered ? null : <g key={column.key} role="gridcell" />;
                      }
                      const span = Math.max(1, Math.min(cell.span ?? 1, columns.length - j));
                      const isSelected = selected?.some((at) => at.row === i && at.col === j) ?? false;
                      return (
                        <CellShell
                          key={column.key}
                          name={cellName(cell.name, drawnWords(cell, cellWidth * span, cellsHeight - 2))}
                          item={itemFor(i + headerRow, j, span)}
                          box={{ x, y: cellsY + 1, width: cellWidth * span, height: cellsHeight - 2 }}
                          selected={isSelected}
                          actionable={cell.actionable}
                          onShow={(at) => tooltip.show(tip(row, column, cell), at)}
                          onHide={tooltip.hide}
                          onClick={cell.actionable ? () => onActivate?.(i, j) : undefined}
                        >
                          <CellMark cell={cell} x={x} y={cellsY + 1} width={cellWidth * span} height={cellsHeight - 2} />
                        </CellShell>
                      );
                    })}
                    {row.trailing && layout.trailingWidth > 0 && (
                      <g role="gridcell" aria-label={row.trailing.text}>
                        {row.trailing.value !== undefined && row.trailing.max !== undefined && row.trailing.max > 0 && (
                          <rect x={gridRight + 8} y={cellsY + cellsHeight - 7} width={Math.max(row.trailing.value > 0 ? 2 : 0, ((layout.trailingWidth - 16) * row.trailing.value) / row.trailing.max)} height={3} rx={1.5} style={{ fill: VIZ.mark }} />
                        )}
                        <text x={gridRight + 8} y={cellsY + cellsHeight / 2 - (row.trailing.value !== undefined ? 3 : 0)} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">
                          <title>{row.trailing.text}</title>
                          {clip(row.trailing.text, layout.trailingWidth - 10)}
                        </text>
                      </g>
                    )}
                  </g>
                );
              })}
            </g>
          );
        }}
      </ChartFrame>
      {tooltip.node}
    </>
  );
}
