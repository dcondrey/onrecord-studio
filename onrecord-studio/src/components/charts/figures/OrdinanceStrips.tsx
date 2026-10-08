import type { EncampmentIncidentDistance, EncampmentLocationObservation, NightHoursObservation, RecordedActivity } from '../../../types/analysisWorkbench';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { Grid } from '../frame/Grid';
import { ReferenceBand, ReferenceLine } from '../frame/ReferenceLine';
import type { ChartSources } from '../frame/SourceCaption';
import { TooltipContent } from '../frame/Tooltip';
import { absent } from '../core/nulls';
import { scaleLinear, type BandScale, type ContinuousScale } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { ExcludedStub, StackedBar, type StackSegment } from '../marks/Bar';
import { Beeswarm } from '../marks/Dot';
import { LABEL_CLASS, MUTED_LABEL_CLASS, type MarkInteraction } from '../marks/shared';
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
  const keys = [0, 3, 6, 9, 12, 15, 18, 21, 24].map((h) => `${h}`);
  const position = (key: string) => x(Number(key) * 60);
  return Object.assign(position, { bandwidth: 0, step: x(180) - x(0), domain: keys });
}
const clockLabel = (key: string) => {
  const hour = (Number(key) + 12) % 24;
  return hour === 0 ? 'midnight' : hour === 12 ? 'noon' : `${hour % 12} ${hour < 12 ? 'a.m.' : 'p.m.'}`;
};

const SWARM_R = 4;
const SWARM_ROWS = 3;
const STRIP = (2 * SWARM_ROWS + 1) * (2 * SWARM_R + 1);
const BAR = 12;
const LANE = 18 + STRIP + 8 + BAR + 18;
/** A gutter under the last lane for the window band's label, above the axis. */
const GUTTER = 20;

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

function interactionFor(name: string, content: JSX.Element, tooltip: ReturnType<typeof useTooltip>, item: MarkInteraction['item'], onClick?: () => void): MarkInteraction {
  return {
    name,
    item,
    onClick,
    onPointerMove: (event) => tooltip.show(content, event),
    onPointerLeave: tooltip.hide,
    onFocusMark: (box) => tooltip.show(content, box),
    onBlurMark: tooltip.hide,
  };
}

function TimePlot({ lanes, width, onRecord }: { lanes: Lane[]; width: number; onRecord(key: string): void }) {
  const tooltip = useTooltip();
  const plotX = 0;
  const right = width - 150;
  const x = scaleLinear([0, 1440], [plotX + 8, right], { nice: false, zero: true });
  const dots = lanes.flatMap((lane) => lane.listed.map((entry) => ({ lane, ...entry })));
  const roving = useRovingFocus({ shape: { rows: Math.max(1, dots.length) }, onActivate: (i) => dots[i] && onRecord(recordKey(dots[i].activity)), onEscape: tooltip.hide });
  const height = lanes.length * LANE;
  let dotIndex = 0;
  return (
    <>
      <ReferenceBand axis="x" from={x(WINDOW_START)} to={x(WINDOW_END)} extent={[0, height + GUTTER - 2]} label="63.0405(b) window 9:00 p.m.–5:30 a.m." />
      {lanes.map((lane, l) => {
        const y = l * LANE;
        const cy = y + 18 + STRIP / 2;
        const timed = lane.inside + lane.boundary + lane.outside;
        const bar = scaleLinear([0, Math.max(1, timed)], [plotX + 8, right], { nice: false });
        const barY = y + 18 + STRIP + 8;
        return (
          <g key={lane.key} aria-label={`${lane.label}: ${lane.activities} activities, ${lane.inside} inside the window, ${lane.boundary} at an endpoint, ${lane.outside} outside, ${lane.noTime} time not recorded.`}>
            <text x={0} y={y + 10} className={ROW_LABEL_CLASS}>{lane.label}</text>
            <Beeswarm
              x={x}
              cy={cy}
              r={SWARM_R}
              maxRows={SWARM_ROWS}
              items={lane.listed.map(({ activity, at }) => {
                const i = dotIndex++;
                const sentence = `${activity.activityType} ${activity.section}, activity ${activity.activityNumber}: recorded ${activity.recordedAt}${activity.atWindowBoundary ? ', at an endpoint' : ''}, ${activity.location}.`;
                return {
                  key: recordKey(activity),
                  value: at,
                  role: 'default' as const,
                  hollow: activity.atWindowBoundary,
                  interaction: interactionFor(sentence, <TooltipContent title={`Activity ${activity.activityNumber}`} rows={[{ key: 't', value: activity.recordedAt, label: 'recorded' }]} note={`${activity.activityType} ${activity.section} · ${activity.location}${activity.atWindowBoundary ? ' · at an endpoint' : ''}`} />, tooltip, roving.getItemProps(i), () => onRecord(recordKey(activity))),
                };
              })}
            />
            {timed > 0 ? (
              <StackedBar
                band={{ pos: barY, thickness: BAR }}
                scale={bar}
                segments={([
                  { key: 'inside', value: lane.inside, tone: 'ord-3', label: `${lane.inside} inside` },
                  { key: 'boundary', value: lane.boundary, tone: 'ord-2', label: `${lane.boundary} at endpoint` },
                  { key: 'outside', value: lane.outside, tone: 'ord-1', label: `${lane.outside} outside` },
                ] satisfies StackSegment[]).filter((segment) => segment.value > 0)}
              />
            ) : (
              <text x={plotX + 8} y={barY + BAR / 2} dy="0.32em" className={MUTED_LABEL_CLASS}>no activity has a recorded time</text>
            )}
            {lane.noTime > 0 && <ExcludedStub band={{ pos: barY, thickness: BAR }} at={right + 8} count={lane.noTime.toLocaleString('en-US')} label="time not recorded" />}
            <text x={plotX + 8} y={barY + BAR + 12} className={LABEL_CLASS}>
              {`${lane.inside.toLocaleString('en-US')} inside · ${lane.boundary.toLocaleString('en-US')} at an endpoint · ${lane.outside.toLocaleString('en-US')} outside`}
            </text>
          </g>
        );
      })}
      <Axis band={clockAxis(x)} format={clockLabel} orient="bottom" offset={height + GUTTER} />
      <line x1={x(0)} x2={x(1440)} y1={height + GUTTER + 0.5} y2={height + GUTTER + 0.5} style={{ stroke: 'var(--viz-baseline)', strokeWidth: 1 }} aria-hidden="true" />
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

const DIST_LANE = 18 + STRIP + 6;

function DistancePlot({ locations, width, onRecord }: { locations: EncampmentLocationObservation; width: number; onRecord(key: string): void }) {
  const tooltip = useTooltip();
  const plotX = 0;
  const right = width - 120;
  const x = scaleLinear([0, MAX_FEET], [plotX + 8, right], { nice: false });
  const lanes = locations.sections.map((section) => ({
    section,
    points: locations.incidents.filter((incident) => incident.section === section.section && incident.latitude !== null && incident.longitude !== null)
      .flatMap((incident) => {
        const feet = nearestFeet(incident);
        return feet === null ? [] : [{ incident, feet }];
      }),
  }));
  const all = lanes.flatMap((lane) => lane.points);
  const roving = useRovingFocus({ shape: { rows: Math.max(1, all.length) }, onActivate: (i) => all[i] && onRecord(incidentKey(all[i].incident)), onEscape: tooltip.hide });
  const height = lanes.length * DIST_LANE;
  let index = 0;
  return (
    <>
      <Grid scale={x} extent={[0, height]} vertical />
      <ReferenceLine kind="yardstick" axis="x" at={x(NEAR_FEET)} extent={[0, height]} labelSide="start" label="1,320 ft: quarter-mile stand-in for “two blocks” (reference; not stated in the ordinance)" />
      <ReferenceLine kind="yardstick" axis="x" at={x(FAR_FEET)} extent={[0, height]} label="2,640 ft (reference)" />
      {lanes.map(({ section, points }, l) => {
        const y = l * DIST_LANE;
        const beyond = points.filter((point) => point.feet > MAX_FEET).length;
        return (
          <g key={section.section} aria-label={`${section.section}: ${points.length} geocoded offenses plotted; ${section.notGeocoded} not geocoded.`}>
            <text x={0} y={y + 10} className={ROW_LABEL_CLASS}>{section.section}</text>
            <Beeswarm
              x={x}
              cy={y + 18 + STRIP / 2}
              r={SWARM_R}
              maxRows={SWARM_ROWS}
              items={points.map(({ incident, feet }) => {
                const i = index++;
                const nearest = [incident.nearestSchool && `school ${incident.nearestSchool.name}, ${Math.round(incident.nearestSchool.distanceFeet).toLocaleString('en-US')} ft`, incident.nearestStation && `station ${incident.nearestStation.name}, ${Math.round(incident.nearestStation.distanceFeet).toLocaleString('en-US')} ft`].filter(Boolean).join('; ');
                const sentence = `Case ${incident.caseNumber}, ${incident.occurredOn ?? 'date not recorded'}, ${incident.blockAddress}: nearest ${nearest}.`;
                return {
                  key: incidentKey(incident),
                  value: Math.min(feet, MAX_FEET),
                  role: 'default' as const,
                  interaction: interactionFor(sentence, <TooltipContent title={`Case ${incident.caseNumber}`} rows={[{ key: 'd', value: `${Math.round(feet).toLocaleString('en-US')} ft`, label: 'to the nearer school or station' }]} note={`${incident.occurredOn ?? 'date not recorded'} · ${incident.blockAddress} · ${nearest}`} />, tooltip, roving.getItemProps(i), () => onRecord(incidentKey(incident))),
                };
              })}
            />
            <text x={right + 10} y={y + 18 + STRIP / 2} dy="0.32em" className={MUTED_LABEL_CLASS}>
              <tspan className="viz-num fill-fg-secondary">{section.notGeocoded.toLocaleString('en-US')}</tspan>
              {' not geocoded'}
              {beyond > 0 && <tspan x={right + 10} dy="1.3em">{`${beyond} beyond 5,000 ft`}</tspan>}
            </text>
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
  return (
    <ChartFrame
      title="Time of day (63.0405(b): 9:00 p.m. to 5:30 a.m.)"
      headline={nightHeadline(night)}
      sources={sources}
      summary={`${lanes.length} section lanes. Dots are the ${night.listedActivities.length} activities listed inside or at the window's endpoints${unreadable ? `; ${unreadable} listed times could not be read and are not placed` : ''}. Bars split each section's timed activities; activities with no recorded time are counted beside the bar.`}
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
      minHeight={lanes.length * LANE + GUTTER + 28}
      interactive={{ roledescription: 'strip plot' }}
      storageKey="encampment-night-hours"
    >
      {({ width }) => <TimePlot lanes={lanes} width={width} onRecord={onRecord} />}
    </ChartFrame>
  );
}

export function DistanceStrip({ locations, sources, onRecord }: { locations: EncampmentLocationObservation; sources: ChartSources; onRecord(key: string): void }) {
  const notGeocoded = locations.sections.reduce((total, row) => total + row.notGeocoded, 0);
  const band = (value: number | null) => (value === null ? absent('not-recorded', 'needs both the incident and the school or station layer') : value);
  return (
    <ChartFrame
      title="Distance to the nearer school or station (63.0404(c))"
      headline={distanceHeadline(locations)}
      sources={sources}
      summary={`${locations.sections.length} section lanes of geocoded offenses by distance in feet; ${notGeocoded} offenses not geocoded and not plotted. 1,320 and 2,640 ft are stand-ins; the ordinance states no distance.`}
      status={locations.sections.length ? 'ready' : 'empty'}
      emptyText="No section has offenses to place."
      table={{
        caption: 'Distance bands by section',
        columns: [{ key: 'section', label: 'Section' }, { key: 'offenses', label: 'Offenses', kind: 'count' }, { key: 'cases', label: 'Cases', kind: 'count' }, { key: 'notGeocoded', label: 'Not geocoded', kind: 'count' }, { key: 'near', label: '≤ 1,320 ft', kind: 'count' }, { key: 'far', label: '1,320–2,640 ft', kind: 'count' }, { key: 'beyond', label: '> 2,640 ft', kind: 'count' }],
        rows: locations.sections.map((row) => ({ section: row.section, offenses: row.offenses, cases: row.distinctCases, notGeocoded: row.notGeocoded, near: band(row.withinNearBand), far: band(row.withinFarBand), beyond: band(row.beyondFarBand) })),
      }}
      legend={[
        { key: 'offense', label: 'Geocoded offense', swatch: 'dot', tone: 'mark' },
        { key: 'stand-in', label: 'Distance stand-in (reference, not in the ordinance)', swatch: 'dashed-line', tone: 'context' },
      ]}
      minHeight={locations.sections.length * DIST_LANE + 36}
      interactive={{ roledescription: 'strip plot' }}
      storageKey="encampment-distance"
    >
      {({ width }) => <DistancePlot locations={locations} width={width} onRecord={onRecord} />}
    </ChartFrame>
  );
}
