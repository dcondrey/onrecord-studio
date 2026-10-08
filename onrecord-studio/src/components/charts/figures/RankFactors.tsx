import type { StoryLead } from '../../../types';
import { COUNTED_FACTOR, FACTOR_ORDER, FACTOR_SCALE_MAX_BP, factorLabel, isKnownFactor, isUnassessed, points, storyHeadline } from '../../dashboard/storyFactors';
import { paintFor, VIZ } from '../core/claim';
import { measureText } from '../core/layout';
import { absent } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import ChartFrame from '../frame/ChartFrame';
import { TooltipContent } from '../frame/Tooltip';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { AbsentSlot, hBarPath, MarkGroup } from '../marks/shared';

/*
 * Why the lead story leads: each factor of the attention score for #1 as a bar on the ranker's
 * shared 0–25 point scale, with #2's points for the same factor as an ink tick. The score ranks
 * reading order; it is neither newsworthiness nor a claim about any record, which the title and the
 * source line say. Only the counted-violation factor may be crimson.
 */

const ROW = 30;
const BAR = 12;
const AXIS = 22;
const LABEL_FONT = '12px -apple-system, BlinkMacSystemFont, sans-serif';
const UNASSESSED = absent('not-recorded', 'the finding’s confidence was not assessed');

interface FactorRow {
  name: string;
  label: string;
  first: { points: number; evidence: string } | 'absent' | 'unassessed';
  second: number | null;
}

function rowsOf(first: StoryLead, second: StoryLead | undefined): FactorRow[] {
  const emitted = new Set([first, second].flatMap((lead) => lead?.publicInterest.factors.map((factor) => factor.name) ?? []));
  const names = [...FACTOR_ORDER.filter((name) => emitted.has(name)), ...[...emitted].filter((name) => !isKnownFactor(name))];
  return names.map((name) => {
    const own = first.publicInterest.factors.find((factor) => factor.name === name);
    const other = second?.publicInterest.factors.find((factor) => factor.name === name);
    return {
      name,
      label: factorLabel(name),
      first: !own ? 'absent' : isUnassessed(first, name) ? 'unassessed' : { points: points(own.basisPoints), evidence: own.evidence },
      second: other && second && !isUnassessed(second, name) ? points(other.basisPoints) : null,
    };
  });
}

function Plot({ rows, width, hasSecond, onOpen }: { rows: readonly FactorRow[]; width: number; hasSecond: boolean; onOpen?(): void }) {
  const tooltip = useTooltip();
  const roving = useRovingFocus({ shape: { rows: rows.length, cols: 1 }, onActivate: () => onOpen?.(), onEscape: tooltip.hide });
  const labelWidth = Math.max(...rows.map((row) => measureText(row.label, LABEL_FONT))) + 16;
  const max = Math.max(points(FACTOR_SCALE_MAX_BP), ...rows.map((row) => (typeof row.first === 'object' ? row.first.points : 0)), ...rows.map((row) => row.second ?? 0));
  const scale = scaleLinear([0, max], [labelWidth, width - 40]);
  const ticks = scale.ticks(5);
  const plotBottom = rows.length * ROW;
  return (
    <g>
      {ticks.map((tick) => (
        <g key={tick}>
          <line x1={scale(tick)} x2={scale(tick)} y1={0} y2={plotBottom} style={{ stroke: tick === 0 ? VIZ.baseline : VIZ.grid, strokeWidth: 1 }} shapeRendering="crispEdges" />
          <text x={scale(tick)} y={plotBottom + 12} dy="0.32em" textAnchor="middle" className="viz-num fill-muted text-[11px]">{tick}</text>
        </g>
      ))}
      <text x={0} y={plotBottom + 12} dy="0.32em" className="fill-muted text-[11px]">Points</text>
      {rows.map((row, index) => {
        const y = index * ROW + (ROW - BAR) / 2;
        const counted = row.name === COUNTED_FACTOR;
        const fill = paintFor(counted ? 'counted' : 'descriptive', 'fill', 'lead').fill;
        const value = typeof row.first === 'object' ? row.first : null;
        const firstText = value ? `+${value.points}` : row.first === 'unassessed' ? 'not assessed' : 'not in its record';
        const secondText = row.second === null ? (hasSecond ? 'not in its record' : null) : `+${row.second}`;
        const name = `${row.label}: lead story ${firstText}${secondText ? `; #2 ${secondText}` : ''}${value ? `. ${value.evidence}` : ''}`;
        const tip = (
          <TooltipContent
            title={counted ? `${row.label} (legal claim)` : `${row.label} (pattern signal)`}
            rows={[{ key: 'first', value: firstText, label: 'lead story' }, ...(secondText ? [{ key: 'second', value: secondText, label: '#2' }] : [])]}
            note={value?.evidence}
          />
        );
        const right = value ? scale(value.points) : scale(0);
        return (
          <g key={row.name}>
            <text x={0} y={y + BAR / 2} dy="0.32em" className={counted ? 'fill-fg text-[12px] font-medium' : 'fill-fg-secondary text-[12px]'}>
              <title>{row.name}</title>
              {row.label}
            </text>
            <MarkGroup
              box={{ x: labelWidth, y, width: Math.max(right - labelWidth, 24), height: BAR }}
              interaction={{
                name,
                item: roving.getItemProps(index, 0),
                onPointerMove: (event) => tooltip.show(tip, { clientX: event.clientX, clientY: event.clientY }),
                onPointerLeave: tooltip.hide,
                onFocusMark: (box) => tooltip.show(tip, box),
                onBlurMark: tooltip.hide,
                onClick: onOpen,
              }}
            >
              {value && value.points > 0 && <path d={hBarPath(scale(0), right, y, BAR, 3)} style={{ fill }} />}
              {row.first === 'unassessed' && <AbsentSlot absence={UNASSESSED} box={{ x: scale(0) + 2, y: y - 2, width: 96, height: BAR + 4 }} text="not assessed" />}
              {row.first === 'absent' && <text x={scale(0) + 6} y={y + BAR / 2} dy="0.32em" className="fill-muted text-[11px]">·</text>}
              {value && <text x={Math.max(right, row.second === null ? 0 : scale(row.second) + 1) + 6} y={y + BAR / 2} dy="0.32em" className="viz-num fill-fg text-[11px] font-medium">{`+${value.points}`}</text>}
            </MarkGroup>
            {row.second !== null && (
              <line x1={scale(row.second)} x2={scale(row.second)} y1={y - 4} y2={y + BAR + 4} style={{ stroke: VIZ.ink, strokeWidth: 2 }} strokeLinecap="round" aria-hidden="true" />
            )}
          </g>
        );
      })}
      {tooltip.node}
    </g>
  );
}

export default function RankFactors({ leads, onOpen }: { leads: readonly StoryLead[]; onOpen?(): void }) {
  const [first, second] = leads;
  if (!first) return null;
  const rows = rowsOf(first, second);
  const headline = storyHeadline(leads);
  const hasCounted = rows.some((row) => row.name === COUNTED_FACTOR && typeof row.first === 'object');
  return (
    <ChartFrame
      title="Why it leads (attention score, not a finding)"
      headline={headline}
      claim="descriptive"
      sources={{ tables: ['findings'], note: 'the attention score ranks reading order; it is not newsworthiness and not a claim about any record' }}
      summary={`${headline} ${rows.map((row) => `${row.label}: ${typeof row.first === 'object' ? `+${row.first.points}` : row.first === 'unassessed' ? 'not assessed' : 'not in the record'}${row.second === null ? '' : `, #2 +${row.second}`}`).join('; ')}.`}
      status="ready"
      table={{
        caption: 'Attention-score factors for the lead story and #2',
        columns: [
          { key: 'factor', label: 'Factor' },
          { key: 'first', label: 'Lead story', align: 'end' },
          ...(second ? [{ key: 'second', label: '#2', align: 'end' as const }] : []),
          { key: 'evidence', label: 'Basis' },
        ],
        rows: rows.map((row) => ({
          factor: row.label,
          first: typeof row.first === 'object' ? row.first.points : row.first === 'unassessed' ? UNASSESSED : 'not in this finding’s record',
          ...(second ? { second: row.second ?? 'not in its record' } : {}),
          evidence: typeof row.first === 'object' ? row.first.evidence : '',
        })),
      }}
      legend={[
        { key: 'first', label: 'Lead story', swatch: 'rect', tone: 'lead' },
        ...(hasCounted ? [{ key: 'counted', label: 'Counted violation (legal claim)', swatch: 'rect' as const, tone: 'counted' as const }] : []),
        ...(second ? [{ key: 'second', label: '#2 for the same factor', swatch: 'line' as const, tone: 'ink' as const }] : []),
      ]}
      minHeight={rows.length * ROW + AXIS}
      interactive={{ roledescription: 'factor bars' }}
      storageKey="lead-story-factors"
    >
      {({ width }) => <Plot rows={rows} width={width} hasSecond={Boolean(second)} onOpen={onOpen} />}
    </ChartFrame>
  );
}
