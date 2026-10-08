import type { StoryLead } from '../../../types';
import { COUNTED_FACTOR, EVIDENCE_FACTOR, FACTOR_ORDER, FACTOR_SCALE_MAX_BP, factorLabel, isKnownFactor, points } from '../../../utils/attentionFactors';
import { absent, measured, type Measured } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import ChartFrame from '../frame/ChartFrame';
import type { LegendItem } from '../frame/Legend';
import { TooltipContent } from '../frame/Tooltip';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Bar } from '../marks/Bar';
import { MUTED_LABEL_CLASS } from '../marks/shared';
import { RowLabel, type RowPlacement } from './BulletRows';
import { fitText } from './ScoreBreakdown';

/*
 * Why one lead ranks where it does: each ranking factor as a bar on one shared points scale, so
 * equal points are equal length across factors and across leads. A factor the finding's record
 * does not carry is stated in words, never drawn as a zero bar; an unassessed evidence strength is
 * the not-recorded slot. The counted-violation factor is the only crimson bar: it is the one factor
 * that rests on a counted legal or policy measure. The score ranks reading order and nothing more.
 */

const BAND = 12;
const GAP = 12;
const NOT_IN_RECORD = 'not in this finding’s record';

/** Factor names are short, so they keep a label column at every width the stage takes. */
function layoutOf(rows: readonly FactorRow[], width: number) {
  const plotX = Math.min(156, Math.max(112, Math.round(width * 0.24)));
  const placed: RowPlacement[] = rows.map((row, index) => {
    const y = 4 + index * (BAND + GAP);
    return { key: row.key, label: fitText(row.label, plotX - 12), labelY: y + BAND / 2, markY: y, inline: true };
  });
  return { rows: placed, plotX, height: 4 + rows.length * (BAND + GAP) };
}

interface FactorRow {
  key: string;
  label: string;
  value: Measured | null;
  evidence: string | null;
  counted: boolean;
}

function rowsOf(lead: StoryLead): FactorRow[] {
  const emitted = lead.publicInterest.factors.map((factor) => factor.name);
  const names = [...FACTOR_ORDER, ...emitted.filter((name) => !isKnownFactor(name))];
  return names.map((name) => {
    const factor = lead.publicInterest.factors.find((candidate) => candidate.name === name) ?? null;
    const unassessed = name === EVIDENCE_FACTOR && lead.confidence.kind === 'UNASSESSED';
    return {
      key: name,
      label: isKnownFactor(name) ? factorLabel(name) : `${factorLabel(name)} (not recognised)`,
      value: !factor ? null : unassessed ? absent('not-recorded', 'The finding’s confidence was not assessed') : measured(factor.basisPoints),
      evidence: factor?.evidence ?? null,
      counted: name === COUNTED_FACTOR,
    };
  });
}

/** One sentence: the score and the factors that carry most of it. */
export function attentionHeadline(lead: StoryLead, rank?: number): string {
  const top = lead.publicInterest.factors.filter((factor) => factor.basisPoints > 0).sort((a, b) => b.basisPoints - a.basisPoints).slice(0, 2);
  const lead1 = `${rank ? `#${rank} ` : ''}scores ${points(lead.publicInterest.totalBasisPoints)}`;
  return top.length === 0
    ? `${lead1.charAt(0).toUpperCase()}${lead1.slice(1)}; no factor added points.`
    : `${lead1.charAt(0).toUpperCase()}${lead1.slice(1)}, most from ${top.map((factor) => `${factor.name} (+${points(factor.basisPoints)})`).join(' and ')}.`;
}

const LEGEND: LegendItem[] = [
  { key: 'factor', label: 'Factor points (a lead the app surfaced)', swatch: 'rect', tone: 'lead' },
  { key: 'counted', label: 'Counted violation (legal claim)', swatch: 'rect', tone: 'counted' },
];

export default function AttentionFactors({ lead, rank }: { lead: StoryLead; rank?: number }) {
  const rows = rowsOf(lead);
  const max = Math.max(FACTOR_SCALE_MAX_BP, ...lead.publicInterest.factors.map((factor) => factor.basisPoints));
  const headline = attentionHeadline(lead, rank);
  const plotHeight = (width: number) => layoutOf(rows, width).height;
  return (
    <ChartFrame
      title="Why it ranks here"
      headline={headline}
      claim="descriptive"
      sources={{ tables: ['findings'], note: 'attention score ranks reading order; it is not newsworthiness and not a claim about any record' }}
      summary={`${headline} ${rows.map((row) => `${row.label}: ${row.value === null ? NOT_IN_RECORD : row.value.state === 'value' ? `+${points(row.value.value)}` : 'not assessed'}`).join('; ')}.`}
      status="ready"
      table={{
        caption: 'Ranking factors with the points and the fact behind each',
        columns: [{ key: 'factor', label: 'Factor' }, { key: 'points', label: 'Points', align: 'end' }, { key: 'evidence', label: 'Fact behind it' }],
        rows: rows.map((row) => ({
          factor: row.label,
          points: row.value === null ? NOT_IN_RECORD : row.value.state === 'value' ? `+${points(row.value.value)}` : row.value,
          evidence: row.evidence ?? NOT_IN_RECORD,
        })),
      }}
      legend={rows.some((row) => row.counted && row.value !== null) ? LEGEND : LEGEND.slice(0, 1)}
      minHeight={plotHeight(600)}
      plotHeight={plotHeight}
      interactive={{ roledescription: 'ranking factors' }}
      storageKey="attention-factors"
    >
      {({ width }) => <FactorPlot rows={rows} width={width} max={max} />}
    </ChartFrame>
  );
}

function FactorPlot({ rows, width, max }: { rows: readonly FactorRow[]; width: number; max: number }) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: rows.length }, onActivate: () => {}, onEscape: tooltip.hide });
  const layout = layoutOf(rows, width);
  const x = scaleLinear([0, max], [layout.plotX, Math.max(layout.plotX + 40, width - 36)], { nice: false });
  return (
    <g>
      {layout.rows.map((placement, index) => {
        const row = rows[index];
        const value = row.value;
        const text = value === null ? NOT_IN_RECORD : value.state === 'value' ? `+${points(value.value)}` : 'not assessed';
        const name = `${row.label}: ${text}${row.evidence ? `. ${row.evidence}` : ''}`;
        const tip = <TooltipContent title={`${row.label}: ${text}`} rows={[]} note={row.evidence ?? undefined} />;
        const interaction = {
          name,
          item: roving.getItemProps(index),
          onPointerMove: (event: React.PointerEvent<SVGGElement>) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
          onPointerLeave: tooltip.hide,
          onFocusMark: (box: DOMRect) => tooltip.show(tip, box),
          onBlurMark: tooltip.hide,
        };
        return (
          <g key={row.key}>
            <RowLabel row={placement} title={row.key} />
            {value === null ? (
              <g role="img" aria-label={name} className="viz-focusable" {...roving.getItemProps(index)}>
                <text x={layout.plotX} y={placement.markY + BAND / 2} dy="0.32em" className={`${MUTED_LABEL_CLASS} italic`}>{NOT_IN_RECORD}</text>
              </g>
            ) : (
              <Bar
                band={{ pos: placement.markY, thickness: BAND }}
                value={value}
                scale={x}
                claim={row.counted ? 'counted' : 'descriptive'}
                role={row.counted ? 'default' : 'lead'}
                label={value.state === 'value' ? text : undefined}
                interaction={interaction}
                motionKey={`attention-${row.key}`}
              />
            )}
          </g>
        );
      })}
      {tooltip.node}
    </g>
  );
}
