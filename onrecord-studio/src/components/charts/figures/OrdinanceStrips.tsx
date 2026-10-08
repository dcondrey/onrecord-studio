import { useState } from 'react';
import type { EncampmentIncidentDistance, EncampmentLocationObservation, NightHoursObservation, RecordedActivity } from '../../../types/analysisWorkbench';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { Grid } from '../frame/Grid';
import { ReferenceBand, ReferenceLine } from '../frame/ReferenceLine';
import type { ChartSources } from '../frame/SourceCaption';
import { TooltipContent } from '../frame/Tooltip';
import { absent } from '../core/nulls';
import { beeswarm } from '../core/layout';
import { scaleLinear, type BandScale, type ContinuousScale } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { ExcludedStub, StackedBar, type StackSegment } from '../marks/Bar';
import { Dot } from '../marks/Dot';
import { MUTED_LABEL_CLASS, type MarkInteraction } from '../marks/shared';
import { ROW_LABEL_CLASS } from './BulletRows';

/** Minutes after midnight of a recorded wall-clock time ('2025-03-14 22:15', '22:15:00'), or null. Never Date.parse: the log's time is local and has no zone. */
export function clockMinutes(recordedAt: string): number | null {
  const match = /(?:^|[T ])(\d{1,2}):(\d{2})(?::\d{2})?/.exec(recordedAt.trim());
  if (!match) return null;
  const hours = Number(match[1]);
  const minutes = Number(match[2]);
  return hours < 24 && minutes < 60 ? hours * 60 + minutes : null;
}

/** The axis runs noon to noon so the night window is one contiguous span. */
const fromNoon = (minutes: number) => (minutes - 720 + 1440) % 1440;
const WINDOW_START = fromNoon(21 * 60);
const WINDOW_END = fromNoon(5 * 60 + 30);
const NEAR_FEET = 1320;
const FAR_FEET = 2640;
const MAX_FEET = 5000;

const isNightSection = (section: string) => section.toLowerCase().startsWith('63.0404(a)');

/** Axis labels at clock hours on the noon-to-noon scale, through the shared band axis. */
function clockAxis(x: ContinuousScale): BandScale<string> {
  // Every 3 hours where the labels fit, else every 6, so midnight is never the label dropped.
  const every = x(180) - x(0) >= 60 ? 3 : 6;
  const keys = Array.from({ length: 24 / every + 1 }, (_, i) => `${i * every}`);
  const position = (key: string) => x(Number(key) * 60);
  return Object.assign(position, { bandwidth: 0, step: x(180) - x(0), domain: keys });
}
const clockLabel = (key: string) => {
  const hour = (Number(key) + 12) % 24;
  return hour === 0 ? 'midnight' : hour === 12 ? 'noon' : `${hour % 12} ${hour < 12 ? 'a.m.' : 'p.m.'}`;
};

const SWARM_R = 4;
const SWARM_ROWS = 3;
const DIAMETER = 2 * SWARM_R + 1;
const STRIP = (2 * SWARM_ROWS + 1) * DIAMETER;
const BAR = 12;
/** Below 480px the share bars put their section label above the bar instead of beside it. */
const narrow = (width: number) => width < 480;
/** A lane: its label, a note on what the dots are, then the strip. */
const LANE_HEAD = 40;
const LANE = LANE_HEAD + STRIP + 8;
/** A gutter under the last lane for the window band's label, above the axis. */
const GUTTER = 20;
/** The share bars are a separate small figure under the clock axis, on their own 0–100% scale. */
const SHARE_GAP = 40;
const SHARE_HEAD = 20;
const shareRow = (width: number) => (narrow(width) ? 36 : 24);
const timeHeight = (lanes: number, width: number) => lanes * LANE + GUTTER + SHARE_GAP + SHARE_HEAD + lanes * shareRow(width) + 4;

interface Lane {
  key: string;
  label: string;
  activities: number;
  inside: number;
  boundary: number;
  outside: number;
  noTime: number;
  listed: { activity: RecordedActivity; at: number }[];
}

function interactionFor(name: string, content: JSX.Element, tooltip: ReturnType<typeof useTooltip>, item: MarkInteraction['item'], onReadout: (text: string) => void, onClick?: () => void): MarkInteraction {
  return {
    name,
    item,
    onClick,
    onPointerMove: (event) => tooltip.show(content, event),
    onPointerLeave: tooltip.hide,
    onFocusMark: (box) => {
      tooltip.show(content, box);
      onReadout(name);
    },
    onBlurMark: tooltip.hide,
  };
}

/**
 * The deterministic beeswarm, split into the dots that have a free place and the overflow that has
 * none. layout.beeswarm parks overflow on its outermost row, on top of a dot already there, so an
 * overflow point is one that collides with a point kept before it.
 */
function swarm<T extends { key: string; value: number }>(items: readonly T[], x: ContinuousScale) {
  const byKey = new Map(items.map((item) => [item.key, item] as const));
  const kept: { item: T; cx: number; dy: number }[] = [];
  const over: { item: T; cx: number }[] = [];
  for (const point of beeswarm(items, x, SWARM_R, SWARM_ROWS)) {
    const item = byKey.get(point.key);
    if (!item) continue;
    if (kept.some((other) => Math.hypot(other.cx - point.cx, other.dy - point.dy) < DIAMETER - 0.01)) over.push({ item, cx: point.cx });
    else kept.push({ item, cx: point.cx, dy: point.dy });
  }
  // Overflow within one dot's width of its neighbour shares one '+n' marker.
  const piles: { cx: number; items: T[] }[] = [];
  for (const point of over.sort((a, b) => a.cx - b.cx)) {
    const last = piles[piles.length - 1];
    if (last && point.cx - last.cx < DIAMETER) last.items.push(point.item);
    else piles.push({ cx: point.cx, items: [point.item] });
  }
  return { kept, piles };
}

function Pile({ cx, cy, count, what }: { cx: number; cy: number; count: number; what: string }) {
  return (
    <text x={cx} y={cy - STRIP / 2 - 2} textAnchor="middle" className="viz-num fill-fg-secondary text-[10px] font-medium">
      <title>{`${count} more ${what} here have no free place in the strip; each is in the Records table`}</title>
      {`+${count}`}
    </text>
  );
}

function TimePlot({ lanes, width, onRecord, onReadout }: { lanes: Lane[]; width: number; onRecord(key: string): void; onReadout(text: string): void }) {
  const tooltip = useTooltip();
  const plotX = 0;
  const right = width - 12;
  // 24px in from the left so the 'noon' label centred on the first tick is not clipped.
  const x = scaleLinear([0, 1440], [plotX + 24, right], { nice: false, zero: true });
  const placed = lanes.map((lane) => swarm(lane.listed.map((entry) => ({ ...entry, key: recordKey(entry.activity), value: entry.at })), x));
  // Arrow keys follow the clock: lane by lane, and within a lane in plotted order.
  const dots = placed.flatMap((lane) => lane.kept.map((point) => point.item));
  const roving = useRovingFocus({ shape: { rows: Math.max(1, dots.length) }, onActivate: (i) => dots[i] && onRecord(dots[i].key), onEscape: tooltip.hide });
  const height = lanes.length * LANE;
  const shareTop = height + GUTTER + SHARE_GAP;
  const labelColumn = narrow(width) ? 0 : Math.min(220, Math.round(width * 0.3));
  const barX0 = labelColumn;
  const barX1 = Math.max(barX0 + 80, Math.min(barX0 + 360, width - 190));
  const share = scaleLinear([0, 1], [barX0, barX1], { nice: false });
  let dotIndex = 0;
  return (
    <>
        {/* The window shades only the clock strips and the gutter above the axis, never the share bars, which are not on the clock. */}
        <ReferenceBand axis="x" from={x(WINDOW_START)} to={x(WINDOW_END)} extent={[height, height + GUTTER - 2]} label="63.0405(b) window 9:00 p.m.–5:30 a.m." />
        {lanes.map((lane, l) => {
          const y = l * LANE;
          const cy = y + LANE_HEAD + STRIP / 2;
          const { kept, piles } = placed[l];
          const note = narrow(width)
            ? `${lane.outside.toLocaleString('en-US')} outside the window: counted, not placed`
            : `dots: activities listed inside or at the window endpoints; ${lane.outside.toLocaleString('en-US')} outside the window are counted, not placed`;
          return (
            <g key={lane.key} aria-label={`${lane.label}: ${lane.activities} activities, ${lane.inside} inside the window, ${lane.boundary} at an endpoint, ${lane.outside} outside, ${lane.noTime} time not recorded.`}>
              <rect x={x(WINDOW_START)} y={y + LANE_HEAD - 2} width={x(WINDOW_END) - x(WINDOW_START)} height={STRIP + 4} style={{ fill: 'var(--viz-track)', opacity: 0.6 }} aria-hidden="true" />
              <text x={0} y={y + 12} className={ROW_LABEL_CLASS}>{lane.label}</text>
              <text x={0} y={y + 27} className={MUTED_LABEL_CLASS}>{note}</text>
              {kept.map(({ item, cx, dy }) => {
                const { activity } = item;
                const i = dotIndex++;
                const sentence = `${activity.activityType} ${activity.section}, activity ${activity.activityNumber}: recorded ${activity.recordedAt}${activity.atWindowBoundary ? ', at an endpoint' : ''}, ${activity.location}.`;
                const content = <TooltipContent title={`Activity ${activity.activityNumber}`} rows={[{ key: 't', value: activity.recordedAt, label: 'recorded' }]} note={`${activity.activityType} ${activity.section} · ${activity.location}${activity.atWindowBoundary ? ' · at an endpoint' : ''}`} />;
                return <Dot key={item.key} cx={cx} cy={cy + dy} r={SWARM_R} role="default" hollow={activity.atWindowBoundary} interaction={interactionFor(sentence, content, tooltip, roving.getItemProps(i), onReadout, () => onRecord(item.key))} />;
              })}
              {piles.map((pile) => <Pile key={pile.items[0].key} cx={pile.cx} cy={cy} count={pile.items.length} what="activities" />)}
            </g>
          );
        })}
        <Axis band={clockAxis(x)} format={clockLabel} orient="bottom" offset={height + GUTTER} />
        <line x1={x(0)} x2={x(1440)} y1={height + GUTTER + 0.5} y2={height + GUTTER + 0.5} style={{ stroke: 'var(--viz-baseline)', strokeWidth: 1 }} aria-hidden="true" />
        <g aria-label="Share of each section's timed activities">
          <text x={0} y={shareTop} className="fill-fg text-[12px] font-medium">Share of timed activities, 0–100% (not clock time)</text>
          {!narrow(width) && (
            <>
              <text x={barX0} y={shareTop + 14} className={MUTED_LABEL_CLASS}>0%</text>
              <text x={barX1} y={shareTop + 14} textAnchor="end" className={MUTED_LABEL_CLASS}>100%</text>
            </>
          )}
          {lanes.map((lane, l) => {
            const rowTop = shareTop + SHARE_HEAD + l * shareRow(width);
            const barY = rowTop + (narrow(width) ? 16 : 4);
            const timed = lane.inside + lane.boundary + lane.outside;
            const words = `${lane.inside.toLocaleString('en-US')} inside · ${lane.boundary.toLocaleString('en-US')} at an endpoint · ${lane.outside.toLocaleString('en-US')} outside`;
            return (
              <g key={lane.key}>
                <text x={0} y={narrow(width) ? rowTop + 10 : barY + BAR / 2} dy={narrow(width) ? undefined : '0.32em'} className={ROW_LABEL_CLASS}>
                  <title>{`${lane.label}: ${words}`}</title>
                  {lane.label.length > 28 ? `${lane.label.slice(0, 27)}…` : lane.label}
                </text>
                {timed > 0 ? (
                  <StackedBar
                    band={{ pos: barY, thickness: BAR }}
                    scale={share}
                    segments={([
                      { key: 'inside', value: lane.inside / timed, tone: 'ord-3', label: `${lane.inside} inside` },
                      { key: 'boundary', value: lane.boundary / timed, tone: 'ord-2', label: `${lane.boundary} at endpoint` },
                      { key: 'outside', value: lane.outside / timed, tone: 'ord-1', label: `${lane.outside} outside` },
                    ] satisfies StackSegment[]).filter((segment) => segment.value > 0)}
                  />
                ) : (
                  <text x={barX0} y={barY + BAR / 2} dy="0.32em" className={MUTED_LABEL_CLASS}>no activity has a recorded time</text>
                )}
                {lane.noTime > 0 && <ExcludedStub band={{ pos: barY, thickness: BAR }} at={barX1 + 8} count={lane.noTime.toLocaleString('en-US')} label="time not recorded" />}
              </g>
            );
          })}
        </g>
        {tooltip.node}
    </>
  );
}

const recordKey = (activity: RecordedActivity) => `activity:${activity.activityNumber}:${activity.section}:${activity.recordedAt}`;
export const incidentKey = (incident: EncampmentIncidentDistance) => `incident:${incident.caseNumber}:${incident.occurredOn ?? ''}:${incident.blockAddress}`;
export { recordKey as activityKey };

function nearestFeet(incident: EncampmentIncidentDistance): number | null {
  const distances = [incident.nearestSchool?.distanceFeet, incident.nearestStation?.distanceFeet].filter((d): d is number => typeof d === 'number' && Number.isFinite(d));
  return distances.length ? Math.min(...distances) : null;
}

const DIST_HEAD = 28;
const DIST_LANE = DIST_HEAD + STRIP + 6;
/** Room above the first lane for the yardstick labels. */
const DIST_TOP = 32;

function DistancePlot({ locations, width, onRecord, onReadout }: { locations: EncampmentLocationObservation; width: number; onRecord(key: string): void; onReadout(text: string): void }) {
  const tooltip = useTooltip();
  const plotX = 0;
  const right = width - 12;
  // The last 64px hold the stub for offenses beyond the axis: they are counted there, never drawn at 5,000 ft.
  const x = scaleLinear([0, MAX_FEET], [plotX + 8, right - 64], { nice: false });
  const lanes = locations.sections.map((section) => {
    const geocoded = locations.incidents.filter((incident) => incident.section === section.section && incident.latitude !== null && incident.longitude !== null);
    const measuredPoints = geocoded.flatMap((incident) => {
      const feet = nearestFeet(incident);
      return feet === null ? [] : [{ incident, feet, key: incidentKey(incident), value: feet }];
    });
    const points = measuredPoints.filter((point) => point.feet <= MAX_FEET);
    return { section, notMeasured: geocoded.length - measuredPoints.length, beyond: measuredPoints.length - points.length, ...swarm(points, x) };
  });
  // Arrow keys follow distance: lane by lane, nearest first.
  const all = lanes.flatMap((lane) => lane.kept.map((point) => point.item));
  const roving = useRovingFocus({ shape: { rows: Math.max(1, all.length) }, onActivate: (i) => all[i] && onRecord(all[i].key), onEscape: tooltip.hide });
  const height = DIST_TOP + lanes.length * DIST_LANE;
  let index = 0;
  return (
    <>
        <Grid scale={x} extent={[0, height]} vertical />
        {/* Labels on two levels, both right of their line, so neither runs into the other or off the plot. */}
        <ReferenceLine kind="yardstick" axis="x" at={x(NEAR_FEET)} extent={[0, height]} label="1,320 ft stand-in (reference)" />
        <ReferenceLine kind="yardstick" axis="x" at={x(FAR_FEET)} extent={[14, height]} label="2,640 ft stand-in (reference)" />
        {lanes.map(({ section, kept, piles, notMeasured, beyond }, l) => {
          const y = DIST_TOP + l * DIST_LANE;
          const cy = y + DIST_HEAD + STRIP / 2;
          const counts = [`${section.notGeocoded.toLocaleString('en-US')} not geocoded`, ...(notMeasured > 0 ? [`${notMeasured.toLocaleString('en-US')} distance not measured`] : [])];
          return (
            <g key={section.section} aria-label={`${section.section}: ${kept.length + piles.reduce((sum, pile) => sum + pile.items.length, 0)} geocoded offenses plotted; ${beyond} beyond 5,000 ft; ${counts.join('; ')}.`}>
              <text x={0} y={y + 10} className={ROW_LABEL_CLASS}>
                {section.section}
                <tspan className={MUTED_LABEL_CLASS}>{` · ${counts.join(' · ')}`}</tspan>
              </text>
              {kept.length === 0 && beyond === 0 && <text x={plotX + 8} y={cy} dy="0.32em" className={MUTED_LABEL_CLASS}>distance not measured for any offense in this section</text>}
              {kept.map(({ item, cx, dy }) => {
                const { incident, feet } = item;
                const i = index++;
                const nearest = [incident.nearestSchool && `school ${incident.nearestSchool.name}, ${Math.round(incident.nearestSchool.distanceFeet).toLocaleString('en-US')} ft`, incident.nearestStation && `station ${incident.nearestStation.name}, ${Math.round(incident.nearestStation.distanceFeet).toLocaleString('en-US')} ft`].filter(Boolean).join('; ');
                const sentence = `Case ${incident.caseNumber}, ${incident.occurredOn ?? 'date not recorded'}, ${incident.blockAddress}: nearest ${nearest}.`;
                const content = <TooltipContent title={`Case ${incident.caseNumber}`} rows={[{ key: 'd', value: `${Math.round(feet).toLocaleString('en-US')} ft`, label: 'to the nearer school or station' }]} note={`${incident.occurredOn ?? 'date not recorded'} · ${incident.blockAddress} · ${nearest}`} />;
                return <Dot key={item.key} cx={cx} cy={cy + dy} r={SWARM_R} role="default" interaction={interactionFor(sentence, content, tooltip, roving.getItemProps(i), onReadout, () => onRecord(item.key))} />;
              })}
              {piles.map((pile) => <Pile key={pile.items[0].key} cx={pile.cx} cy={cy} count={pile.items.length} what="offenses" />)}
              {beyond > 0 && (
                <g>
                  <title>{`${beyond} geocoded ${beyond === 1 ? 'offense is' : 'offenses are'} more than 5,000 ft from a school or station; counted here, not placed on the axis`}</title>
                  <path d={`M${right - 60},${cy - 5}l5,5l-5,5`} style={{ fill: 'none', stroke: 'var(--viz-mark)', strokeWidth: 1.5 }} />
                  <text x={right - 50} y={cy} dy="0.32em" className={MUTED_LABEL_CLASS}>
                    <tspan className="viz-num fill-fg-secondary">{beyond.toLocaleString('en-US')}</tspan>
                    {' >5,000 ft'}
                  </text>
                </g>
              )}
            </g>
          );
        })}
        <Axis scale={x} kind="count" orient="bottom" offset={height} />
        <text x={right} y={height + 30} textAnchor="end" className={MUTED_LABEL_CLASS}>feet to the nearer school or station</text>
        {tooltip.node}
    </>
  );
}

function timeLanes(night: NightHoursObservation): { lanes: Lane[]; unreadable: number } {
  let unreadable = 0;
  const lanes = night.sections.map((row) => {
    const listed = night.listedActivities
      .filter((activity) => activity.section === row.section && activity.activityType === row.activityType)
      .flatMap((activity) => {
        const minutes = clockMinutes(activity.recordedAt);
        if (minutes === null) {
          unreadable += 1;
          return [];
        }
        return [{ activity, at: fromNoon(minutes) }];
      });
    const type = row.activityType || 'type not recorded';
    return {
      key: `${row.section}:${row.activityType}`,
      label: `${row.section} ${type}${isNightSection(row.section) ? '' : ' (for comparison)'}`,
      activities: row.activities,
      inside: row.insideWindow,
      boundary: row.atWindowBoundary,
      outside: row.outsideWindow,
      noTime: row.timeNotRecorded,
      listed,
    };
  });
  return { lanes, unreadable };
}

export function nightHeadline(night: NightHoursObservation): string {
  const night404a = night.sections.filter((row) => isNightSection(row.section));
  const rows = night404a.length ? night404a : night.sections;
  const sum = (pick: (row: (typeof rows)[number]) => number) => rows.reduce((total, row) => total + pick(row), 0);
  const scope = night404a.length ? 'under 63.0404(a)' : 'in the listed sections';
  const noTime = sum((row) => row.timeNotRecorded);
  return `${sum((row) => row.insideWindow).toLocaleString('en-US')} of ${sum((row) => row.activities).toLocaleString('en-US')} activities ${scope} were recorded between 9 p.m. and 5:30 a.m.${noTime ? `; ${noTime.toLocaleString('en-US')} have no recorded time` : ''}.`;
}

export function distanceHeadline(locations: EncampmentLocationObservation): string {
  const notGeocoded = locations.sections.reduce((total, row) => total + row.notGeocoded, 0);
  if (locations.sections.some((row) => row.withinNearBand === null || row.withinFarBand === null || row.beyondFarBand === null)) {
    return `Distance to schools and stations is not measured for every section; ${notGeocoded.toLocaleString('en-US')} offenses not geocoded.`;
  }
  const near = locations.sections.reduce((total, row) => total + (row.withinNearBand ?? 0), 0);
  const measuredCount = locations.sections.reduce((total, row) => total + (row.withinNearBand ?? 0) + (row.withinFarBand ?? 0) + (row.beyondFarBand ?? 0), 0);
  return `${near.toLocaleString('en-US')} of ${measuredCount.toLocaleString('en-US')} geocoded offenses were within 1,320 ft of a school or station; ${notGeocoded.toLocaleString('en-US')} not geocoded.`;
}

/**
 * F12 OrdinanceStrips. (A) Time of day on a noon-to-noon axis so the night window is one band; a
 * static, deterministic beeswarm of listed activities per section (endpoints hollow), a 100% bar
 * of inside / at endpoint / outside, and the activities with no recorded time as an excluded stub
 * beside it, never on the clock. (B) Distance to the nearer school or station with the 1,320 and
 * 2,640 ft stand-ins as yardsticks; offenses not geocoded are counted beside each lane, never
 * plotted. These are observations: no accent colour anywhere.
 */
export function NightHoursStrip({ night, sources, onRecord }: { night: NightHoursObservation; sources: ChartSources; onRecord(key: string): void }) {
  const { lanes, unreadable } = timeLanes(night);
  const [readout, setReadout] = useState('');
  return (
    <>
      <ChartFrame
        title="Time of day (63.0405(b): 9:00 p.m. to 5:30 a.m.)"
        headline={nightHeadline(night)}
        sources={sources}
        summary={`${lanes.length} section lanes. Dots are the ${night.listedActivities.length} activities listed inside or at the window's endpoints; activities outside the window are counted, not placed${unreadable ? `; ${unreadable} listed times could not be read and are not placed` : ''}. Below the clock, bars split each section's timed activities from 0 to 100%; activities with no recorded time are counted beside the bar.`}
        status={lanes.length ? 'ready' : 'empty'}
        emptyText="No section has listed activities."
        table={{
          caption: 'Night-hours counts by section',
          columns: [{ key: 'section', label: 'Section' }, { key: 'activities', label: 'Activities', kind: 'count' }, { key: 'inside', label: '9 p.m.–5:30 a.m.', kind: 'count' }, { key: 'boundary', label: 'At an endpoint', kind: 'count' }, { key: 'outside', label: 'Outside', kind: 'count' }, { key: 'noTime', label: 'Time not recorded' }],
          rows: lanes.map((lane) => ({ section: lane.label, activities: lane.activities, inside: lane.inside, boundary: lane.boundary, outside: lane.outside, noTime: lane.noTime ? absent('excluded', `${lane.noTime} activities have no recorded time`) : 0 })),
        }}
        legend={[
          { key: 'listed', label: 'Listed activity', swatch: 'dot', tone: 'mark' },
          { key: 'endpoint', label: 'At an endpoint', swatch: 'hollow', tone: 'mark' },
          { key: 'window', label: 'Ordinance window', swatch: 'rect', tone: 'track' },
        ]}
        minHeight={timeHeight(lanes.length, 640)}
        plotHeight={(width) => timeHeight(lanes.length, width)}
        interactive={{ roledescription: 'strip plot' }}
        storageKey="encampment-night-hours"
      >
        {({ width }) => <TimePlot lanes={lanes} width={width} onRecord={onRecord} onReadout={setReadout} />}
      </ChartFrame>
      <p aria-live="polite" className="sr-only">{readout}</p>
    </>
  );
}

export function DistanceStrip({ locations, sources, onRecord }: { locations: EncampmentLocationObservation; sources: ChartSources; onRecord(key: string): void }) {
  const notGeocoded = locations.sections.reduce((total, row) => total + row.notGeocoded, 0);
  const band = (value: number | null) => (value === null ? absent('not-recorded', 'needs both the incident and the school or station layer') : value);
  const [readout, setReadout] = useState('');
  return (
    <>
      <ChartFrame
        title="Distance to the nearer school or station (63.0404(c))"
        headline={distanceHeadline(locations)}
        sources={sources}
        summary={`${locations.sections.length} section lanes of geocoded offenses by distance in feet, up to 5,000 ft; farther ones are counted at the right edge; ${notGeocoded} offenses not geocoded and not plotted. 1,320 and 2,640 ft are stand-ins; the ordinance states no distance.`}
        status={locations.sections.length ? 'ready' : 'empty'}
        emptyText="No section has offenses to place."
        table={{
          caption: 'Distance bands by section',
          columns: [{ key: 'section', label: 'Section' }, { key: 'offenses', label: 'Offenses', kind: 'count' }, { key: 'cases', label: 'Cases', kind: 'count' }, { key: 'notGeocoded', label: 'Not geocoded', kind: 'count' }, { key: 'near', label: '≤ 1,320 ft', kind: 'count' }, { key: 'far', label: '1,320–2,640 ft', kind: 'count' }, { key: 'beyond', label: '> 2,640 ft', kind: 'count' }],
          rows: locations.sections.map((row) => ({ section: row.section, offenses: row.offenses, cases: row.distinctCases, notGeocoded: row.notGeocoded, near: band(row.withinNearBand), far: band(row.withinFarBand), beyond: band(row.beyondFarBand) })),
        }}
        legend={[
          { key: 'offense', label: 'Geocoded offense', swatch: 'dot', tone: 'mark' },
          { key: 'stand-in', label: '1,320 ft: quarter-mile stand-in for “two blocks”; 2,640 ft: half mile (references, not stated in the ordinance)', swatch: 'dashed-line', tone: 'context' },
        ]}
        minHeight={DIST_TOP + locations.sections.length * DIST_LANE + 36}
        plotHeight={() => DIST_TOP + locations.sections.length * DIST_LANE + 36}
        interactive={{ roledescription: 'strip plot' }}
        storageKey="encampment-distance"
      >
        {({ width }) => <DistancePlot locations={locations} width={width} onRecord={onRecord} onReadout={setReadout} />}
      </ChartFrame>
      <p aria-live="polite" className="sr-only">{readout}</p>
    </>
  );
}
