import { useRef } from 'react';
import type { CorpusEntityEdge, CorpusEntityNeighborhood, RelationshipKind } from '../../../types';
import ChartFrame from '../frame/ChartFrame';
import { TooltipContent } from '../frame/Tooltip';
import { paintFor, VIZ } from '../core/claim';
import { formatDate, formatValue } from '../core/format';
import { measureText } from '../core/layout';
import { parseDate, scaleLinear } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { LABEL_CLASS, MUTED_LABEL_CLASS, MarkGroup, hBarPath } from '../marks/shared';
import { fitText } from './ScoreBreakdown';

/*
 * F9 NeighbourBars: who and what appears with a subject. Each bar is the documents shared with the
 * subject on a track of all the subject's documents ('12 of 38 documents'). Explicit relationships
 * (signed by, approved, cites…) come first under their own subheads; co-occurrence comes last,
 * because appearing in the same documents is not a relationship. An observed edge is solid; a
 * computed one is an outline; an asserted one is an outline with the word 'asserted'.
 */

const ROW_LABEL_CLASS = 'fill-fg text-[12px]';
const LABEL_FONT_12 = '12px -apple-system, BlinkMacSystemFont, sans-serif';

const KIND_ORDER: readonly RelationshipKind[] = ['SIGNED_BY', 'APPROVED', 'CITES', 'PROMISES', 'DELIVERED', 'SUPERSEDES', 'AMENDS', 'CO_OCCURRENCE'];
const KIND_HEAD: Record<RelationshipKind, string> = {
  SIGNED_BY: 'Signed by',
  APPROVED: 'Approved',
  CITES: 'Cites',
  PROMISES: 'Promises',
  DELIVERED: 'Delivered',
  SUPERSEDES: 'Supersedes',
  AMENDS: 'Amends',
  CO_OCCURRENCE: 'Appears in the same documents (not a stated relationship)',
};

function docs(n: number): string {
  return `${n.toLocaleString('en-US')} document${n === 1 ? '' : 's'}`;
}

function span(edge: CorpusEntityEdge): string | null {
  const from = edge.validFrom ?? null;
  const until = edge.validUntil ?? null;
  if (from === null && until === null) return null;
  const f = from ? formatDate(parseDate(from), 'day') : 'start unknown';
  const u = until ? formatDate(parseDate(until), 'day') : 'open ended';
  return `${f} to ${u}`;
}

export function neighbourHeadline(neighborhood: CorpusEntityNeighborhood, total: number | null): string {
  const { center, edges } = neighborhood;
  if (edges.length === 0) return `${center.displayValue} appears in ${docs(center.documentCount)}; no other name appears with it yet.`;
  const top = [...edges].sort((a, b) => b.sharedDocumentCount - a.sharedDocumentCount || a.neighbor.displayValue.localeCompare(b.neighbor.displayValue))[0];
  const truncation = neighborhood.truncated ? ` First ${edges.length}${total !== null ? ` of ${total.toLocaleString('en-US')}` : ''} neighbours shown.` : '';
  return `${center.displayValue} appears in ${docs(center.documentCount)}; most often with ${top.neighbor.displayValue} (${top.sharedDocumentCount.toLocaleString('en-US')} shared, ${KIND_HEAD[top.kind].split(' (')[0].toLowerCase()}).${truncation}`;
}

interface Layout {
  heads: { key: string; y: number; text: string }[];
  rows: { edge: CorpusEntityEdge; labelY: number; barY: number; spanText: string | null }[];
  height: number;
}

const BAR_H = 8;
/** Longest track, px, so a count stays near its name on a wide window. */
const TRACK_MAX = 480;

function layout(edges: readonly CorpusEntityEdge[], compact: boolean): Layout {
  const heads: Layout['heads'] = [];
  const rows: Layout['rows'] = [];
  let y = 0;
  for (const kind of KIND_ORDER) {
    const group = edges.filter((edge) => edge.kind === kind).sort((a, b) => b.sharedDocumentCount - a.sharedDocumentCount || a.neighbor.displayValue.localeCompare(b.neighbor.displayValue));
    if (group.length === 0) continue;
    heads.push({ key: kind, y: y + 8, text: KIND_HEAD[kind] });
    y += 22;
    for (const edge of group) {
      const spanText = span(edge);
      rows.push({ edge, labelY: y + 7, barY: y + 17, spanText });
      y += 17 + BAR_H + (spanText ? 14 : 0) + (compact ? 8 : 10);
    }
    y += 4;
  }
  return { heads, rows, height: y + 4 };
}

export function neighbourPlotHeight(edges: readonly CorpusEntityEdge[], compact: boolean): number {
  return layout(edges, compact).height;
}

function NeighbourPlot({ neighborhood, width, compact, onOpen }: {
  neighborhood: CorpusEntityNeighborhood;
  width: number;
  compact: boolean;
  onOpen(edge: CorpusEntityEdge, options: { shift: boolean }): void;
}) {
  const tooltip = useTooltip();
  // Shift state of the click or key press now being handled, captured before the mark's own handler runs.
  const shiftHeld = useRef(false);
  const { heads, rows } = layout(neighborhood.edges, compact);
  const roving = useRovingFocus({ shape: { rows: Math.max(1, rows.length) }, onActivate: (i) => rows[i] && onOpen(rows[i].edge, { shift: shiftHeld.current }), onEscape: tooltip.hide });
  const total = Math.max(1, neighborhood.center.documentCount);
  const valueRoom = measureText(`${total.toLocaleString('en-US')} of ${total.toLocaleString('en-US')}`) + 14;
  const x = scaleLinear([0, total], [0, Math.max(40, Math.min(TRACK_MAX, width - valueRoom))], { nice: false });
  const captureShift = (event: { shiftKey: boolean }) => { shiftHeld.current = event.shiftKey; };
  return (
    <g onKeyDownCapture={captureShift} onPointerDownCapture={captureShift}>
      {heads.map((head) => (
        <text key={head.key} x={0} y={head.y} dy="0.32em" className="fill-muted text-[11px] font-semibold uppercase tracking-wide">{fitText(head.text, width, '600 11px -apple-system, sans-serif')}</text>
      ))}
      {rows.map((row, index) => {
        const { edge } = row;
        const shared = Math.min(edge.sharedDocumentCount, total);
        const asserted = edge.basis === 'ASSERTED';
        const outline = edge.basis !== 'OBSERVED';
        const paint = outline ? paintFor('descriptive', 'outline', 'primary') : paintFor('descriptive', 'fill', 'default');
        const value = `${edge.sharedDocumentCount.toLocaleString('en-US')} of ${neighborhood.center.documentCount.toLocaleString('en-US')}`;
        const name = `${edge.neighbor.displayValue}: ${KIND_HEAD[edge.kind].split(' (')[0]}, ${value} shared documents, ${edge.basis.toLowerCase()}${row.spanText ? `, ${row.spanText}` : ''}.`;
        const tip = (
          <TooltipContent
            title={edge.neighbor.displayValue}
            rows={[
              { key: 's', value, label: 'shared documents' },
              { key: 'c', value: formatValue(edge.confidenceBasisPoints / 10_000, { kind: 'percent', digits: 0 }), label: 'confidence' },
            ]}
            note={`${edge.basis === 'OBSERVED' ? 'Observed' : edge.basis === 'COMPUTED' ? 'Computed by this app' : 'Asserted by a source'} · weakest validation ${edge.neighbor.validationGrade.toLowerCase().replace(/_/g, ' ')}${row.spanText ? ` · ${row.spanText}` : ''}`}
          />
        );
        const end = x(shared);
        const tagText = asserted ? 'asserted' : null;
        const label = fitText(edge.neighbor.displayValue, width - (tagText ? measureText(tagText) + 12 : 0), LABEL_FONT_12);
        return (
          <MarkGroup
            key={edge.neighbor.entityId}
            box={{ x: 0, y: row.labelY - 8, width: width - 2, height: row.barY + BAR_H - row.labelY + 10 }}
            interaction={{
              name: `${name} ${compact ? 'Opens its inspector layer.' : `Opens its dossier; Shift searches Evidence for documents mentioning ${edge.neighbor.displayValue}.`}`,
              item: roving.getItemProps(index),
              onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
              onPointerLeave: tooltip.hide,
              onFocusMark: (box) => tooltip.show(tip, box),
              onBlurMark: tooltip.hide,
              onClick: () => onOpen(edge, { shift: shiftHeld.current }),
            }}
          >
            <g>
              <text x={0} y={row.labelY} dy="0.32em" className={ROW_LABEL_CLASS} style={{ textDecoration: 'underline', textDecorationColor: 'rgb(var(--line-strong))' }}>{label}</text>
              {tagText && <text x={measureText(label, LABEL_FONT_12) + 8} y={row.labelY} dy="0.32em" className={MUTED_LABEL_CLASS}>{tagText}</text>}
              <rect x={0} y={row.barY} width={x(total)} height={BAR_H} rx={2} style={{ fill: VIZ.track }} />
              {shared > 0 && (
                outline
                  ? <path d={hBarPath(0.75, Math.max(1.5, end - 0.75), row.barY + 0.75, BAR_H - 1.5)} style={{ fill: VIZ.surface, stroke: paint.stroke, strokeWidth: paint.strokeWidth }} />
                  : <path d={hBarPath(0, end, row.barY, BAR_H)} style={{ fill: paint.fill }} />
              )}
              <text x={x(total) + 6} y={row.barY + BAR_H / 2} dy="0.32em" className={LABEL_CLASS}>{value}</text>
              {row.spanText && <text x={0} y={row.barY + BAR_H + 9} dy="0.32em" className={MUTED_LABEL_CLASS}>{row.spanText}</text>}
            </g>
          </MarkGroup>
        );
      })}
      {tooltip.node}
    </g>
  );
}

export default function NeighbourBars({ neighborhood, total, compact = false, onOpen, storageKey }: {
  neighborhood: CorpusEntityNeighborhood;
  /** Every neighbour, from count_corpus_entity_neighbors; null when not counted or the count failed. */
  total: number | null;
  compact?: boolean;
  onOpen(edge: CorpusEntityEdge, options: { shift: boolean }): void;
  storageKey: string;
}) {
  const headline = neighbourHeadline(neighborhood, total);
  const { edges } = neighborhood;
  const bases = new Set(edges.map((edge) => edge.basis));
  const shown = neighborhood.truncated ? `First ${edges.length}${total !== null ? ` of ${total.toLocaleString('en-US')}` : ''}` : null;
  return (
    <ChartFrame
      title={shown ? `Appears with (${shown.toLowerCase()})` : 'Appears with'}
      headline={headline}
      sources={{ tables: ['corpus_entity_mentions', 'corpus_entity_relationships', 'corpus_entity_document_counts'], note: 'co-occurrence counts, not relationships unless tagged' }}
      summary={`${headline} Bars show documents shared with ${neighborhood.center.displayValue} out of its ${docs(neighborhood.center.documentCount)}.`}
      status={edges.length === 0 ? 'empty' : 'ready'}
      emptyText="No name appears in the same documents yet."
      table={{
        caption: `Names appearing with ${neighborhood.center.displayValue}`,
        columns: [{ key: 'name', label: 'Name' }, { key: 'kind', label: 'Relationship' }, { key: 'basis', label: 'Basis' }, { key: 'shared', label: 'Shared documents', kind: 'count' }, { key: 'of', label: 'Of', kind: 'count' }, { key: 'dates', label: 'Dates' }],
        rows: edges.map((edge) => ({
          name: edge.neighbor.displayValue,
          kind: KIND_HEAD[edge.kind].split(' (')[0],
          basis: edge.basis.toLowerCase(),
          shared: edge.sharedDocumentCount,
          of: neighborhood.center.documentCount,
          // Not drawn as an absence in the plot, so not listed as one: the row is simply undated.
          dates: span(edge) ?? 'undated',
        })),
      }}
      legend={bases.size > 1 || !bases.has('OBSERVED') ? [
        ...(bases.has('OBSERVED') ? [{ key: 'observed', label: 'Observed', swatch: 'rect' as const, tone: 'mark' as const }] : []),
        ...(bases.has('COMPUTED') || bases.has('ASSERTED') ? [{ key: 'computed', label: 'Computed or asserted (outline)', swatch: 'outline-rect' as const, tone: 'ink' as const }] : []),
      ] : undefined}
      minHeight={neighbourPlotHeight(edges, compact)}
      interactive={{ roledescription: 'ranked bars' }}
      storageKey={storageKey}
    >
      {({ width }) => <NeighbourPlot neighborhood={neighborhood} width={width} compact={compact} onOpen={onOpen} />}
    </ChartFrame>
  );
}
