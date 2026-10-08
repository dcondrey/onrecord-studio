import type { CaseworkLead } from '../../../types/casework';
import ChartFrame from '../frame/ChartFrame';
import { TooltipContent } from '../frame/Tooltip';
import { measureText } from '../core/layout';
import { measured } from '../core/nulls';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Segments } from '../marks/Segments';

/*
 * F4 ScoreBreakdown: why each casework lead ranks where it does. Each factor is a segment as wide
 * as its maximum, filled to its score in violet (a lead the app surfaced), so the factor that
 * drives a lead is visible at a glance. The score prioritises review; it is not confidence and it
 * is not a finding of wrongdoing, which the title says.
 */

const ROW_LABEL_CLASS = 'fill-fg text-[12px] font-medium';
const ROW = 46;
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

function BreakdownPlot({ rows, width, onOpen }: { rows: readonly LeadRow[]; width: number; onOpen(index: number): void }) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: Math.max(1, rows.length) }, onActivate: onOpen, onEscape: tooltip.hide });
  const widest = Math.max(1, ...rows.map((row) => row.maximum));
  const scoreRoom = Math.max(...rows.map((row) => measureText(`${row.score} of ${row.maximum}`)), 40) + 12;
  const unit = Math.max(4, (width - scoreRoom - 4 - 2 * Math.max(0, ...rows.map((row) => row.factors.length - 1))) / widest);
  return (
    <g>
      {rows.map((row, index) => {
        const y = index * ROW;
        const name = `${row.title}: review priority ${row.score} of ${row.maximum}: ${row.factors.map((factor) => `${factor.label} ${factor.score} of ${factor.maximum}`).join(', ')}.`;
        const tip = (
          <TooltipContent
            title={row.title}
            rows={row.factors.map((factor) => ({ key: factor.label, value: `${factor.score}/${factor.maximum}`, label: factor.label }))}
            note={row.factors.map((factor) => `${factor.label}: ${factor.reason}`).join(' ')}
          />
        );
        const barWidth = row.maximum * unit + 2 * Math.max(0, row.factors.length - 1);
        return (
          <g key={row.key}>
            <text x={0} y={y + 10} dy="0.32em" className={ROW_LABEL_CLASS}>
              <title>{row.title}</title>
              {fitText(row.title, width - 4, '600 12px -apple-system, BlinkMacSystemFont, sans-serif')}
            </text>
            <Segments
              x={0}
              y={y + 22}
              height={BAR_H}
              unit={unit}
              segments={row.factors.map((factor) => ({ key: factor.label, max: factor.maximum, value: measured(Math.max(0, Math.min(factor.maximum, factor.score))), label: `${factor.label}: ${factor.score} of ${factor.maximum}. ${factor.reason}` }))}
              interaction={{
                name,
                item: roving.getItemProps(index),
                onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
                onPointerLeave: tooltip.hide,
                onFocusMark: (box) => tooltip.show(tip, box),
                onBlurMark: tooltip.hide,
                onClick: () => onOpen(index),
              }}
            />
            <text x={barWidth + 8} y={y + 22 + BAR_H / 2} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">{`${row.score} of ${row.maximum}`}</text>
          </g>
        );
      })}
      {tooltip.node}
    </g>
  );
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
      minHeight={Math.max(1, rows.length) * ROW}
      interactive={{ roledescription: 'score breakdown' }}
      storageKey="casework-score-breakdown"
    >
      {({ width }) => <BreakdownPlot rows={rows} width={width} onOpen={onOpen} />}
    </ChartFrame>
  );
}
