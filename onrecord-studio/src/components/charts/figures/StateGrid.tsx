import { useMemo, type ReactNode } from 'react';
import { ShapeGlyph, type GlyphShape } from '../../ui/shapeGlyph';
import { VIZ, type Claim } from '../core/claim';
import { fitsInside, measureText } from '../core/layout';
import { measured, type Absence } from '../core/nulls';
import ChartFrame, { type ChartStatus } from '../frame/ChartFrame';
import type { LegendItem } from '../frame/Legend';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinCell, TwinColumn } from '../frame/TableTwin';
import { TooltipContent, type TooltipRow } from '../frame/Tooltip';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { HeatCell, type SeqStep } from '../marks/HeatCell';
import { MarkGroup, MUTED_LABEL_CLASS, type MarkInteraction } from '../marks/shared';

/** A cell glyph: the shared shape vocabulary, plus a check for 'observed and unchanged'. */
export type StateGlyph = GlyphShape | 'check';

interface CellBase {
  /** Short words printed in the cell when they fit ('changed 3', 'not observed'). */
  text: string;
  /** The cell's accessible name: a full sentence. */
  name: string;
  glyph: StateGlyph;
  /** The table twin's words; default `text`. */
  tableText?: string;
  tip?: { rows: TooltipRow[]; note?: string };
  /** Enter, Space or a click on this cell does something. */
  actionable?: boolean;
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
  cells: StateGridCell[];
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
  legend?: LegendItem[];
  controls?: ReactNode;
  storageKey: string;
  /** The selected cells, outlined in ink (selection never uses violet). */
  selected?: readonly { row: number; col: number }[];
  onActivate?(row: number, col: number): void;
  onColumnActivate?(col: number): void;
  onEscape?(): void;
  className?: string;
}

const ROW_HEIGHT = 30;
const ROW_HEIGHT_TWO_LINES = 38;
const GROUP_HEIGHT = 22;
const HEADER_FLAT = 24;
const HEADER_MAX_LABEL = 112;
const HEADER_ANGLE = 40;
const ROW_LABEL_CLASS = 'fill-fg-secondary text-[12px]';
const ROW_FONT = '12px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';

/** Shortens a label to `width` px, adding an ellipsis; the full label stays in the <title>. */
function clip(text: string, width: number, font?: string): string {
  if (measureText(text, font) <= width) return text;
  let end = text.length;
  while (end > 1 && measureText(`${text.slice(0, end)}…`, font) > width) end -= 1;
  return `${text.slice(0, end)}…`;
}

interface Layout {
  labelWidth: number;
  trailingWidth: number;
  cellWidth: number;
  headerHeight: number;
  rotated: boolean;
  rowY: number[];
  groupY: { label: string; y: number }[];
  rowHeight: number;
  height: number;
}

function layoutFor(width: number, props: StateGridProps): Layout {
  const { rows, columns } = props;
  const twoLines = rows.some((row) => row.sublabel);
  const rowHeight = twoLines ? ROW_HEIGHT_TWO_LINES : ROW_HEIGHT;
  const widest = Math.max(0, ...rows.map((row) => Math.max(measureText(row.label, ROW_FONT), row.sublabel ? measureText(row.sublabel) : 0)));
  const labelWidth = Math.round(Math.max(88, Math.min(widest + 12, width * 0.32, 240)));
  const trailingWidth = rows.some((row) => row.trailing)
    ? Math.round(Math.min(Math.max(...rows.map((row) => (row.trailing ? measureText(row.trailing.text) : 0)), 40) + 16, width * 0.2, 150))
    : 0;
  const cellWidth = Math.max(24, Math.min(132, (width - labelWidth - trailingWidth) / Math.max(1, columns.length)));
  const headerRoom = cellWidth - (columns.some((column) => column.glyph) ? 18 : 6);
  const rotated = columns.some((column) => !fitsInside(column.label, headerRoom));
  const longest = Math.min(HEADER_MAX_LABEL, Math.max(0, ...columns.map((column) => measureText(column.label))));
  const headerHeight = rotated ? Math.ceil(longest * Math.sin((HEADER_ANGLE * Math.PI) / 180)) + 20 : HEADER_FLAT;
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
  return { labelWidth, trailingWidth, cellWidth, headerHeight, rotated, rowY, groupY, rowHeight, height: y + 4 };
}

function CellGlyph({ glyph, cx, cy, colour }: { glyph: StateGlyph; cx: number; cy: number; colour: string }) {
  if (glyph === 'check') {
    return <path d={`M${cx - 4},${cy} l2.8,3 l5.2,-6`} style={{ fill: 'none', stroke: colour, strokeWidth: 2, strokeLinecap: 'round', strokeLinejoin: 'round' }} />;
  }
  return <ShapeGlyph shape={glyph} cx={cx} cy={cy} radius={3.5} style={glyph === 'ring' ? { stroke: colour, strokeWidth: 1.5 } : { fill: colour }} />;
}

/** A state cell or a violet 'needed' gap: drawn here because the heat cell only fills ramp steps. */
function StateCellMark({ cell, x, y, width, height, interaction }: { cell: StateGridCell; x: number; y: number; width: number; height: number; interaction: MarkInteraction }) {
  const inset = 1;
  const w = Math.max(0, width - 2 * inset);
  const h = Math.max(0, height - 2 * inset);
  const lead = (cell.kind === 'state' && cell.tone === 'lead') || (cell.kind === 'gap' && cell.lead);
  const filled = cell.kind === 'state' && cell.tone === 'lead';
  const quiet = cell.kind === 'state' && cell.tone === 'quiet';
  const textColour = filled ? 'var(--viz-on-color)' : quiet ? 'rgb(var(--muted))' : 'rgb(var(--fg))';
  const glyphColour = filled ? 'var(--viz-on-color)' : cell.kind === 'state' && cell.tone === 'positive' ? VIZ.positive : lead ? VIZ.lead : quiet ? VIZ.context : VIZ.ink;
  const showText = fitsInside(cell.text, w - 20) && h >= 14;
  const rect = filled
    ? { fill: VIZ.lead }
    : cell.kind === 'gap'
      ? { fill: 'none', stroke: VIZ.lead, strokeWidth: 1.5, strokeDasharray: cell.absence.state === 'not-covered' ? '1 2' : '3 2' }
      : cell.kind === 'state' && cell.tone === 'positive'
        ? { fill: VIZ.track }
        : cell.kind === 'state' && cell.tone === 'quiet'
          ? { fill: 'none' }
          : { fill: 'none', stroke: VIZ.mark, strokeWidth: 1 };
  return (
    <MarkGroup interaction={interaction} box={{ x: x + inset, y: y + inset, width: w, height: h }} ringRadius={4}>
      {!showText && <title>{cell.text}</title>}
      <rect x={x + inset + 0.75} y={y + inset + 0.75} width={Math.max(0, w - 1.5)} height={Math.max(0, h - 1.5)} rx={2} data-tone={lead ? 'lead' : cell.kind === 'state' ? cell.tone : undefined} style={rect} />
      <CellGlyph glyph={cell.glyph} cx={x + inset + (showText ? 10 : w / 2)} cy={y + inset + h / 2} colour={glyphColour} />
      {showText && (
        <text x={x + inset + 19} y={y + inset + h / 2} dy="0.32em" className="viz-num text-[11px]" style={{ fill: textColour }}>{cell.text}</text>
      )}
    </MarkGroup>
  );
}

/** The table twin's text for a cell: an absence prints its state and reason, never a blank. */
function twinCell(cell: StateGridCell): TwinCell {
  if (cell.kind === 'gap') return { ...cell.absence, reason: cell.tableText ?? cell.absence.reason };
  return cell.tableText ?? cell.text;
}

/**
 * F8: rows × columns of states. Every cell carries a glyph and words (in the cell, or its title
 * when they do not fit), so no state rests on colour; absence is drawn as its kind of empty cell,
 * never as a zero. The grid is one tab stop, moved through with the arrow keys in two dimensions.
 */
export default function StateGrid(props: StateGridProps) {
  const { rows, columns, selected, onActivate, onEscape } = props;
  const tooltip = useTooltip();
  const roving = useRovingFocus({
    shape: { rows: Math.max(1, rows.length), cols: Math.max(1, columns.length) },
    onActivate: (i, j = 0) => {
      if (rows[i]?.cells[j]?.actionable) onActivate?.(i, j);
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
        columns.forEach((column, j) => {
          const cell = row.cells[j];
          record[`c:${column.key}`] = cell ? twinCell(cell) : null;
        });
        if (props.trailingHeader) record.trailing = row.trailing?.text ?? null;
        return record;
      }),
    };
  }, [columns, rows, props.headline, props.rowHeader, props.title, props.trailingHeader]);

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
        legend={props.legend}
        controls={props.controls}
        storageKey={props.storageKey}
        className={props.className}
        minHeight={layoutFor(640, props).height}
        plotHeight={(width) => layoutFor(width, props).height}
        interactive={{ roledescription: 'state grid' }}
      >
        {({ width }) => {
          const layout = layoutFor(width, props);
          const { labelWidth, cellWidth, rowHeight, headerHeight } = layout;
          const gridRight = labelWidth + cellWidth * columns.length;
          return (
            <g role="grid" aria-label={props.title} aria-rowcount={rows.length + 1} aria-colcount={columns.length + 1}>
              <g role="row">
                <g role="columnheader"><text x={0} y={headerHeight - 8} className={MUTED_LABEL_CLASS}>{props.rowHeader}</text></g>
                {columns.map((column, j) => {
                  const cx = labelWidth + j * cellWidth;
                  const label = clip(column.label, layout.rotated ? HEADER_MAX_LABEL : cellWidth - (column.glyph ? 18 : 6));
                  const textX = layout.rotated ? cx + cellWidth / 2 : cx + (column.glyph ? 16 : 4);
                  const content = (
                    <>
                      <title>{column.label}</title>
                      {column.glyph && <ShapeGlyph shape={column.glyph} cx={layout.rotated ? cx + cellWidth / 2 : cx + 8} cy={headerHeight - (layout.rotated ? 4 : 12)} radius={3.5} style={column.glyph === 'ring' ? { stroke: VIZ.ink, strokeWidth: 1.5 } : { fill: VIZ.ink }} />}
                      <text
                        x={textX}
                        y={headerHeight - (layout.rotated ? 12 : 8)}
                        transform={layout.rotated ? `rotate(-${HEADER_ANGLE} ${textX} ${headerHeight - 12})` : undefined}
                        className="fill-fg-secondary text-[11px] font-medium"
                      >
                        {label}
                      </text>
                    </>
                  );
                  return (
                    <g key={column.key} role="columnheader" aria-label={column.label}>
                      {props.onColumnActivate ? (
                        <g role="button" tabIndex={0} className="viz-focusable cursor-pointer" aria-label={`Filter by ${column.label}`} onClick={() => props.onColumnActivate?.(j)} onKeyDown={(event) => {
                          if (event.key === 'Enter' || event.key === ' ') {
                            event.preventDefault();
                            props.onColumnActivate?.(j);
                          }
                        }}>
                          <rect x={cx} y={0} width={cellWidth} height={headerHeight} fill="transparent" />
                          {content}
                          <rect className="viz-focus-ring" x={cx + 1} y={1} width={cellWidth - 2} height={headerHeight - 2} rx={3} style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }} />
                        </g>
                      ) : content}
                    </g>
                  );
                })}
                {props.trailingHeader && layout.trailingWidth > 0 && (
                  <g role="columnheader"><text x={gridRight + 8} y={headerHeight - 8} className={MUTED_LABEL_CLASS}>{clip(props.trailingHeader, layout.trailingWidth - 8)}</text></g>
                )}
              </g>
              {layout.groupY.map((group) => (
                <text key={`group-${group.label}`} x={0} y={group.y + GROUP_HEIGHT / 2 + 2} dy="0.32em" className="fill-fg text-[11px] font-semibold" aria-hidden="true">{group.label}</text>
              ))}
              {rows.map((row, i) => {
                const y = layout.rowY[i];
                const labelY = row.sublabel ? y + rowHeight / 2 - 6 : y + rowHeight / 2;
                return (
                  <g key={row.key} role="row">
                    <g role="rowheader" aria-label={row.sublabel ? `${row.label}, ${row.sublabel}` : row.label}>
                      <text x={0} y={labelY} dy="0.32em" className={ROW_LABEL_CLASS}>
                        <title>{row.label}</title>
                        {clip(row.label, labelWidth - 10, ROW_FONT)}
                      </text>
                      {row.sublabel && <text x={0} y={labelY + 14} dy="0.32em" className={MUTED_LABEL_CLASS}>{clip(row.sublabel, labelWidth - 10)}</text>}
                    </g>
                    {columns.map((column, j) => {
                      const cell = row.cells[j];
                      const x = labelWidth + j * cellWidth;
                      if (!cell) return <g key={column.key} role="gridcell" />;
                      const interaction: MarkInteraction = {
                        name: cell.name,
                        item: roving.getItemProps(i, j),
                        onPointerMove: (event) => tooltip.show(tip(row, column, cell), { clientX: event.clientX, clientY: event.clientY }),
                        onPointerLeave: tooltip.hide,
                        onFocusMark: (box) => tooltip.show(tip(row, column, cell), box),
                        onBlurMark: tooltip.hide,
                        onClick: cell.actionable ? () => onActivate?.(i, j) : undefined,
                      };
                      const isSelected = selected?.some((cell) => cell.row === i && cell.col === j) ?? false;
                      return (
                        <g key={column.key} role="gridcell" aria-selected={isSelected || undefined} className={cell.actionable ? 'cursor-pointer' : undefined}>
                          {cell.kind === 'count' ? (
                            <HeatCell x={x} y={y + 1} width={cellWidth} height={rowHeight - 2} value={measured(cell.value)} step={cell.step} glyph={cell.glyph === 'check' ? undefined : cell.glyph} text={cell.text} interaction={interaction} />
                          ) : cell.kind === 'gap' && !cell.lead ? (
                            <HeatCell x={x} y={y + 1} width={cellWidth} height={rowHeight - 2} value={cell.absence} glyph={cell.glyph === 'check' ? undefined : cell.glyph} text={cell.text} interaction={interaction} />
                          ) : (
                            <StateCellMark cell={cell} x={x} y={y + 1} width={cellWidth} height={rowHeight - 2} interaction={interaction} />
                          )}
                          {isSelected && <rect x={x + 0.5} y={y + 1.5} width={cellWidth - 1} height={rowHeight - 3} rx={3} pointerEvents="none" style={{ fill: 'none', stroke: VIZ.ink, strokeWidth: 2 }} />}
                        </g>
                      );
                    })}
                    {row.trailing && layout.trailingWidth > 0 && (
                      <g role="gridcell" aria-label={row.trailing.text}>
                        {row.trailing.value !== undefined && row.trailing.max !== undefined && row.trailing.max > 0 && (
                          <rect x={gridRight + 8} y={y + rowHeight - 7} width={Math.max(row.trailing.value > 0 ? 2 : 0, ((layout.trailingWidth - 16) * row.trailing.value) / row.trailing.max)} height={3} rx={1.5} style={{ fill: VIZ.mark }} />
                        )}
                        <text x={gridRight + 8} y={y + rowHeight / 2 - (row.trailing.value !== undefined ? 3 : 0)} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">
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
