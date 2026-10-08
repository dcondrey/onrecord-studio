import type { CaseworkLead } from '../../../types/casework';
import ChartFrame from '../frame/ChartFrame';
import { TooltipContent } from '../frame/Tooltip';
import { measureText } from '../core/layout';
import { measured } from '../core/nulls';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Segments } from '../marks/Segments';
import { MarkGroup } from '../marks/shared';

/*
 * F4 ScoreBreakdown: why each casework lead ranks where it does. Each factor is a segment as wide
 * as its maximum, filled to its score in violet (a lead the app surfaced), so the factor that
 * drives a lead is visible at a glance. The score prioritises review; it is not confidence and it
 * is not a finding of wrongdoing, which the title says.
 */

const ROW_LABEL_CLASS = 'fill-fg text-[12px] font-medium';
const ROW = 60;
const BAR_H = 12;

interface LeadRow {
  key: string;
  title: string;
  factors: { label: string; score: number; maximum: number; reason: string }[];
  score: number;
  maximum: number;
}

function rowsOf(leads: readonly CaseworkLead[]): LeadRow[] {
  return leads.map((lead) => {
    // A factor with no possible contribution is omitted, never drawn as an empty segment.
    const factors = lead.factors.filter((factor) => factor.maximum > 0);
    return {
      key: `${lead.reportId}:${lead.findingIndex}`,
      title: lead.title,
      factors,
      score: factors.reduce((sum, factor) => sum + Math.max(0, Math.min(factor.maximum, factor.score)), 0),
      maximum: factors.reduce((sum, factor) => sum + factor.maximum, 0),
    };
  });
}

/** A label shortened with an ellipsis to fit `width`; the full text stays in the row's <title>. */
export function fitText(text: string, width: number, font?: string): string {
  if (measureText(text, font) <= width) return text;
  let end = text.length;
  while (end > 1 && measureText(`${text.slice(0, end)}…`, font) > width) end -= 1;
  return `${text.slice(0, end).trimEnd()}…`;
}

export function scoreHeadline(leads: readonly CaseworkLead[]): string {
  const rows = rowsOf(leads);
  if (rows.length === 0) return 'No lead is ranked yet.';
  const top = rows[0];
  const driver = [...top.factors].sort((a, b) => b.score - a.score || a.label.localeCompare(b.label))[0];
  return `${rows.length} lead${rows.length === 1 ? '' : 's'} ranked; top: ${top.title} (${top.score} of ${top.maximum})${driver ? `, driven by ${driver.label.toLowerCase()}` : ''}.`;
}

/** Most px the widest lead's bar may take, so its score stays near its title at any width. */
const MAX_BAR = 480;
const SEGMENT_GAP = 2;
const FACTOR_LABEL_CLASS = 'fill-fg-secondary text-[11px]';

/** Factor labels in first-seen order; a label that does not fit its segment prints this ordinal. */
function factorOrder(rows: readonly LeadRow[]): string[] {
  return [...new Set(rows.flatMap((row) => row.factors.map((factor) => factor.label)))];
}

function BreakdownPlot({ rows, width, onOpen }: { rows: readonly LeadRow[]; width: number; onOpen(index: number): void }) {
  const tooltip = useTooltip();
  const cols = Math.max(1, ...rows.map((row) => row.factors.length));
  const roving = useRovingFocus({ shape: { rows: Math.max(1, rows.length), cols }, onActivate: (i) => onOpen(i), onEscape: tooltip.hide });
  const widest = Math.max(1, ...rows.map((row) => row.maximum));
  const scoreRoom = Math.max(...rows.map((row) => measureText(`${row.score} of ${row.maximum}`)), 40) + 12;
  const gaps = SEGMENT_GAP * Math.max(0, cols - 1);
  const unit = Math.max(4, Math.min((MAX_BAR - gaps) / widest, (width - scoreRoom - 4 - gaps) / widest));
  const order = factorOrder(rows);
  return (
    <g>
      {rows.map((row, index) => {
        const y = index * ROW;
        const barY = y + 22;
        let cursor = 0;
        const placed = row.factors.map((factor) => {
          const left = cursor;
          const segmentWidth = factor.maximum * unit;
          cursor += segmentWidth + SEGMENT_GAP;
          const full = fitText(factor.label, segmentWidth - 2);
          const text = full.length >= 4 ? full : String(order.indexOf(factor.label) + 1);
          return { factor, left, segmentWidth, text };
        });
        const barWidth = Math.max(0, cursor - SEGMENT_GAP);
        return (
          <g key={row.key}>
            <text x={0} y={y + 10} dy="0.32em" className={ROW_LABEL_CLASS}>
              <title>{row.title}</title>
              {fitText(row.title, width - 4, '600 12px -apple-system, BlinkMacSystemFont, sans-serif')}
            </text>
            <Segments
              x={0}
              y={barY}
              height={BAR_H}
              unit={unit}
              segments={row.factors.map((factor) => ({ key: factor.label, max: factor.maximum, value: measured(Math.max(0, Math.min(factor.maximum, factor.score))), label: `${factor.label}: ${factor.score} of ${factor.maximum}. ${factor.reason}` }))}
            />
            {placed.map(({ factor, left, segmentWidth, text }, j) => {
              const name = `${row.title}: ${factor.label} ${factor.score} of ${factor.maximum}: ${factor.reason}`;
              const tip = <TooltipContent title={`${factor.label}: ${factor.score} of ${factor.maximum}`} rows={[]} note={`${row.title}. ${factor.reason}`} />;
              return (
                <g key={factor.label}>
                  <MarkGroup
                    box={{ x: left, y: barY, width: segmentWidth, height: BAR_H }}
                    interaction={{
                      name,
                      item: roving.getItemProps(index, j),
                      onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
                      onPointerLeave: tooltip.hide,
                      onFocusMark: (box) => tooltip.show(tip, box),
                      onBlurMark: tooltip.hide,
                      onClick: () => onOpen(index),
                    }}
                  >
                    <rect x={left} y={barY} width={segmentWidth} height={BAR_H} style={{ fill: 'transparent' }} />
                  </MarkGroup>
                  <text x={left} y={barY + BAR_H + 9} dy="0.32em" className={FACTOR_LABEL_CLASS} aria-hidden="true">{text}</text>
                </g>
              );
            })}
            <text x={barWidth + 8} y={barY + BAR_H / 2} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">{`${row.score} of ${row.maximum}`}</text>
          </g>
        );
      })}
      {tooltip.node}
    </g>
  );
}

/** The ordinal key, shown only when some factor label did not fit its segment. */
function ordinalKey(rows: readonly LeadRow[], width: number): string[] | null {
  const order = factorOrder(rows);
  const widest = Math.max(1, ...rows.map((row) => row.maximum));
  const unit = Math.max(4, Math.min((MAX_BAR - SEGMENT_GAP * 3) / widest, width / widest));
  const truncated = rows.some((row) => row.factors.some((factor) => fitText(factor.label, factor.maximum * unit - 2).length < 4));
  return truncated ? order.map((label, index) => `${index + 1} ${label}`) : null;
}

export default function ScoreBreakdown({ leads, onOpen }: { leads: readonly CaseworkLead[]; onOpen(index: number): void }) {
  const rows = rowsOf(leads);
  const headline = scoreHeadline(leads);
  return (
    <ChartFrame
      title="Review priority (not confidence or wrongdoing)"
      headline={rows.length === 0 ? null : headline}
      sources={{ tables: ['casework_cases', 'research_reports'], note: 'deterministic factor scores; each segment is as wide as its factor can score' }}
      summary={`${headline} ${rows.map((row) => `${row.title}: ${row.score} of ${row.maximum}`).join('; ')}.`}
      status={rows.length === 0 ? 'empty' : 'ready'}
      emptyText="Saved research findings will become ranked leads here."
      table={{
        caption: 'Why each lead ranks here: factor scores and reasons',
        columns: [{ key: 'lead', label: 'Lead' }, { key: 'factor', label: 'Factor' }, { key: 'score', label: 'Score', align: 'end' }, { key: 'maximum', label: 'Of', align: 'end' }, { key: 'reason', label: 'Reason' }],
        rows: rows.flatMap((row) => row.factors.map((factor) => ({ lead: row.title, factor: factor.label, score: factor.score, maximum: factor.maximum, reason: factor.reason }))),
      }}
      legend={[{ key: 'scored', label: 'Factor score (a lead the app surfaced)', swatch: 'rect', tone: 'lead' }, { key: 'room', label: 'Room to the factor\'s maximum', swatch: 'rect', tone: 'track' }]}
      minHeight={Math.max(1, rows.length) * ROW + 14}
      interactive={{ roledescription: 'score breakdown' }}
      storageKey="casework-score-breakdown"
    >
      {({ width }) => {
        const key = ordinalKey(rows, width);
        return (
          <>
            <BreakdownPlot rows={rows} width={width} onOpen={onOpen} />
            {key && <text x={0} y={rows.length * ROW + 4} dy="0.32em" className={FACTOR_LABEL_CLASS}>{key.join(' · ')}</text>}
          </>
        );
      }}
    </ChartFrame>
  );
}
