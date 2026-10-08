import type { StoryLead } from '../../../types';
import { comparisonHeadline, COUNTED_FACTOR, FACTOR_ORDER, FACTOR_SCALE_MAX_BP, factorLabel, isKnownFactor, isUnassessed, points } from '../../../utils/attentionFactors';
import { paintFor, VIZ } from '../core/claim';
import { absent, type Absence } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import ChartFrame from '../frame/ChartFrame';
import type { LegendItem } from '../frame/Legend';
import { TooltipContent } from '../frame/Tooltip';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { AbsentSlot, hBarPath, MarkGroup, MUTED_LABEL_CLASS } from '../marks/shared';
import { ROW_LABEL_CLASS } from './BulletRows';
import { fitText } from './ScoreBreakdown';

/*
 * Why one lead ranks where it does: each ranking factor as a bar on the ranker's shared 0–25 point
 * scale, so equal points are equal length across factors and across leads, with the compared
 * lead's points for the same factor as an ink tick. A factor the finding's record does not carry is
 * stated in words, never drawn as a zero bar; an unassessed evidence strength is the not-recorded
 * slot. The counted-violation factor is the only crimson bar: it is the one factor that rests on a
 * counted legal or policy measure. The score ranks reading order and nothing more.
 */

const BAND = 12;
const GAP = 12;
const AXIS = 22;
const TICK_OVERHANG = 4;
const NOT_IN_RECORD = 'not in this finding’s record';
const UNASSESSED: Absence = { state: 'not-recorded', reason: 'The finding’s confidence was not assessed' };

type FactorValue = { points: number; evidence: string } | 'absent' | 'unassessed';

interface FactorRow {
  key: string;
  label: string;
  value: FactorValue;
  /** The compared lead's points for this factor; null when it lacks the factor or never assessed it. */
  compare: number | null;
  counted: boolean;
}

function valueOf(lead: StoryLead, name: string): FactorValue {
  const factor = lead.publicInterest.factors.find((candidate) => candidate.name === name);
  if (!factor) return 'absent';
  if (isUnassessed(lead, name)) return 'unassessed';
  return { points: points(factor.basisPoints), evidence: factor.evidence };
}

function rowsOf(lead: StoryLead, compare: StoryLead | undefined): FactorRow[] {
  const emitted = lead.publicInterest.factors.map((factor) => factor.name);
  const names = [...FACTOR_ORDER, ...emitted.filter((name) => !isKnownFactor(name))];
  return names.map((name) => {
    const other = compare ? valueOf(compare, name) : 'absent';
    return {
      key: name,
      label: isKnownFactor(name) ? factorLabel(name) : `${factorLabel(name)} (not recognised)`,
      value: valueOf(lead, name),
      compare: typeof other === 'object' ? other.points : null,
      counted: name === COUNTED_FACTOR,
    };
  });
}

const textOf = (value: FactorValue) => (typeof value === 'object' ? `+${value.points}` : value === 'unassessed' ? 'not assessed' : NOT_IN_RECORD);

function layoutOf(rows: readonly FactorRow[], width: number) {
  const plotX = Math.min(156, Math.max(112, Math.round(width * 0.24)));
  const rowY = (index: number) => 4 + index * (BAND + GAP);
  return { plotX, rowY, plotBottom: rowY(rows.length) - GAP + 4, height: rowY(rows.length) - GAP + 4 + AXIS };
}

export default function AttentionFactors({ lead, rank, compare }: { lead: StoryLead; rank: number; compare?: { lead: StoryLead; rank: number } }) {
  const rows = rowsOf(lead, compare?.lead);
  const headline = comparisonHeadline(lead, rank, compare);
  const max = Math.max(FACTOR_SCALE_MAX_BP, ...lead.publicInterest.factors.map((factor) => factor.basisPoints), ...(compare?.lead.publicInterest.factors.map((factor) => factor.basisPoints) ?? []));
  const plotHeight = (width: number) => layoutOf(rows, width).height;
  const legend: LegendItem[] = [
    { key: 'factor', label: 'Factor points (a lead the app surfaced)', swatch: 'rect', tone: 'lead' },
    ...(rows.some((row) => row.counted && row.value !== 'absent') ? [{ key: 'counted', label: 'Counted violation (legal claim)', swatch: 'rect' as const, tone: 'counted' as const }] : []),
    ...(compare ? [{ key: 'compare', label: `#${compare.rank} for the same factor`, swatch: 'line' as const, tone: 'ink' as const }] : []),
  ];
  return (
    <ChartFrame
      title="Why it ranks here (attention score, not a finding)"
      headline={headline}
      claim="descriptive"
      sources={{ tables: ['findings'], note: 'the attention score ranks reading order; it is not newsworthiness and not a claim about any record' }}
      summary={`${headline} ${rows.map((row) => `${row.label}: ${textOf(row.value)}${compare && row.compare !== null ? `, #${compare.rank} +${row.compare}` : ''}`).join('; ')}.`}
      status="ready"
      table={{
        caption: 'Ranking factors with the points and the fact behind each',
        columns: [
          { key: 'factor', label: 'Factor' },
          { key: 'points', label: 'Points', align: 'end' },
          ...(compare ? [{ key: 'compare', label: `#${compare.rank}`, align: 'end' as const }] : []),
          { key: 'evidence', label: 'Fact behind it' },
        ],
        rows: rows.map((row) => ({
          factor: row.label,
          points: row.value === 'unassessed' ? absent('not-recorded', UNASSESSED.reason) : textOf(row.value),
          ...(compare ? { compare: row.compare === null ? NOT_IN_RECORD : `+${row.compare}` } : {}),
          evidence: typeof row.value === 'object' ? row.value.evidence : NOT_IN_RECORD,
        })),
      }}
      legend={legend}
      minHeight={plotHeight(600)}
      plotHeight={plotHeight}
      interactive={{ roledescription: 'ranking factors' }}
      storageKey="attention-factors"
    >
      {({ width }) => <FactorPlot rows={rows} width={width} max={max} compareRank={compare?.rank} />}
    </ChartFrame>
  );
}

function FactorPlot({ rows, width, max, compareRank }: { rows: readonly FactorRow[]; width: number; max: number; compareRank?: number }) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: rows.length }, onActivate: () => {}, onEscape: tooltip.hide });
  const layout = layoutOf(rows, width);
  // The largest factor ends short of the edge, so its value label prints outside the bar like the rest.
  const x = scaleLinear([0, points(max) * 1.14], [layout.plotX, Math.max(layout.plotX + 48, width - 4)], { nice: false });
  const ticks: number[] = [];
  for (let tick = 0; tick <= points(max); tick += 5) ticks.push(tick);
  const lead = paintFor('descriptive', 'fill', 'lead').fill;
  const counted = paintFor('counted', 'fill').fill;
  return (
    <g>
      <g aria-hidden="true">
        {ticks.map((tick) => (
          <g key={tick}>
            <line x1={x(tick)} x2={x(tick)} y1={0} y2={layout.plotBottom} style={{ stroke: tick === 0 ? VIZ.baseline : VIZ.grid, strokeWidth: 1 }} shapeRendering="crispEdges" />
            <text x={x(tick)} y={layout.plotBottom + 11} dy="0.32em" textAnchor="middle" className="viz-num fill-muted text-[11px]">{tick}</text>
          </g>
        ))}
        <text x={layout.plotX - 12} y={layout.plotBottom + 11} dy="0.32em" textAnchor="end" className="fill-muted text-[11px]">Points</text>
      </g>
      {rows.map((row, index) => {
        const y = layout.rowY(index);
        const value = typeof row.value === 'object' ? row.value : null;
        const text = textOf(row.value);
        const versus = compareRank === undefined ? '' : `; #${compareRank} ${row.compare === null ? 'not in its record' : `+${row.compare}`}`;
        const name = `${row.label}: ${text}${versus}${value ? `. ${value.evidence}` : ''}`;
        const tip = (
          <TooltipContent
            title={row.value === 'absent' ? row.label : `${row.label} (${row.counted ? 'legal claim' : 'pattern signal'})`}
            rows={[{ key: 'own', value: text, label: 'this lead' }, ...(compareRank !== undefined ? [{ key: 'compare', value: row.compare === null ? 'not in its record' : `+${row.compare}`, label: `#${compareRank}` }] : [])]}
            note={value?.evidence}
          />
        );
        const end = value ? x(value.points) : x(0);
        const tick = row.compare === null ? null : x(row.compare);
        const tickLine = tick !== null && (
          <line aria-hidden="true" data-compare-tick x1={tick} x2={tick} y1={y - TICK_OVERHANG} y2={y + BAND + TICK_OVERHANG} style={{ stroke: VIZ.ink, strokeWidth: 2 }} strokeLinecap="round" />
        );
        return (
          <g key={row.key}>
            {/* An absent factor's words sit at the bars' start like every other absent row, haloed
                over the compared lead's tick, which is drawn beneath them. */}
            {row.value === 'absent' && tickLine}
            <text x={0} y={y + BAND / 2} dy="0.32em" className={row.counted && row.value !== 'absent' ? 'fill-fg text-[12px] font-medium' : ROW_LABEL_CLASS} style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 4, strokeLinejoin: 'round' }}>
              <title>{row.key}</title>
              {fitText(row.label, layout.plotX - 12)}
            </text>
            <MarkGroup
              box={{ x: layout.plotX, y, width: Math.max(end - layout.plotX, 24), height: BAND }}
              interaction={{
                name,
                item: roving.getItemProps(index),
                onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
                onPointerLeave: tooltip.hide,
                onFocusMark: (box) => tooltip.show(tip, box),
                onBlurMark: tooltip.hide,
              }}
            >
              {value && value.points > 0 && <path d={hBarPath(x(0), end, y, BAND, 3)} style={{ fill: row.counted ? counted : lead }} />}
              {/* With no bar of its own, the words start past the compared lead's tick, never under it. */}
              {row.value === 'unassessed' && <AbsentSlot absence={UNASSESSED} box={{ x: Math.max(x(0) + 2, (tick ?? 0) + 6), y: y - 2, width: 96, height: BAND + 4 }} text="not assessed" />}
              {row.value === 'absent' && <text x={x(0) + 6} y={y + BAND / 2} dy="0.32em" className={`${MUTED_LABEL_CLASS} italic`} style={{ paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 4, strokeLinejoin: 'round' }}>{NOT_IN_RECORD}</text>}
              {value && <text x={Math.max(end, tick ?? 0) + 6} y={y + BAND / 2} dy="0.32em" className="viz-num fill-fg text-[11px] font-medium">{text}</text>}
            </MarkGroup>
            {row.value !== 'absent' && tickLine}
          </g>
        );
      })}
      {tooltip.node}
    </g>
  );
}

