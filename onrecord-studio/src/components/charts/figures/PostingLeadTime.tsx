import type { CalendarMeeting, MeetingDocument, PostingAbsence, PostingCounts } from '../../../types/meetings';
import ChartFrame from '../frame/ChartFrame';
import { Axis } from '../frame/Axis';
import { Grid } from '../frame/Grid';
import { ReferenceLine } from '../frame/ReferenceLine';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent } from '../frame/Tooltip';
import { VIZ } from '../core/claim';
import { absent } from '../core/nulls';
import { scaleLinear } from '../core/scale';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { Dot } from '../marks/Dot';
import { MUTED_LABEL_CLASS } from '../marks/shared';
import { RowLabel, rowLayout } from './BulletRows';

/** Short rail wording for each reason a file has no posting time. */
export const POSTING_ABSENCE_SHORT: Record<PostingAbsence, string> = {
  NO_FETCH_RECORD: 'no fetch record',
  NO_HEADER: 'no Last-Modified header',
  UNREADABLE_HEADER: 'header not a readable date',
  REQUEST_TIME_HOST: 'host reports request time',
  HEADER_IS_REQUEST_TIME: 'header is request time',
  REQUEST_TIME_NOT_RULED_OUT: 'request time not ruled out',
};

const NO_START = 'meeting start time not recorded';
const NO_DATE = 'meeting date not recorded';

interface PlottedFile {
  key: string;
  meeting: CalendarMeeting;
  document: MeetingDocument;
  lastModified: string;
  /** Hours relative to the stated start: negative is before it. */
  hours: number;
}

interface Row {
  key: string;
  label: string;
  meeting: CalendarMeeting;
  files: PlottedFile[];
}

const BAND = 14;
const TOP = 26;
const YARDSTICK_HOURS = 72;

function dateLabel(date: string): string {
  const parsed = Date.parse(`${date}T00:00:00Z`);
  return Number.isFinite(parsed) ? new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }).format(parsed) : date;
}

/** '141 h 0 min before stated start'; whole minutes, never rounded up. */
export function leadTimeText(minutes: number): string {
  const size = Math.abs(minutes);
  const text = `${Math.floor(size / 60).toLocaleString('en-US')} h ${size % 60} min`;
  return minutes < 0 ? `${text} after stated start` : `${text} before stated start`;
}

function utc(at: string): string {
  return `${at.slice(0, 16).replace('T', ' ')} UTC`;
}

/** Splits the calendar's agenda files into plotted rows and the 'No posting time' rail. */
export function postingLeadTimeData(meetings: readonly CalendarMeeting[]) {
  const rail = new Map<string, number>();
  const unplotted: { meeting: CalendarMeeting; document: MeetingDocument; reason: string }[] = [];
  const upcoming: Row[] = [];
  const past: Row[] = [];
  for (const meeting of meetings) {
    const files: PlottedFile[] = [];
    for (const document of meeting.documents) {
      if (document.role !== 'AGENDA') continue;
      const reason = meeting.timing === 'DATE_NOT_RECORDED' || !meeting.date
        ? NO_DATE
        : document.posting.state === 'NOT_RECORDED'
          ? POSTING_ABSENCE_SHORT[document.posting.reason]
          : !meeting.startTime || document.minutesBeforeMeeting === null
            ? NO_START
            : null;
      if (reason !== null || document.posting.state !== 'RECORDED' || document.minutesBeforeMeeting === null) {
        const why = reason ?? NO_START;
        rail.set(why, (rail.get(why) ?? 0) + 1);
        unplotted.push({ meeting, document, reason: why });
        continue;
      }
      files.push({ key: `${document.portalKey}:${document.documentId}`, meeting, document, lastModified: document.posting.lastModified, hours: -document.minutesBeforeMeeting / 60 });
    }
    if (files.length === 0) continue;
    files.sort((a, b) => a.lastModified.localeCompare(b.lastModified));
    const row = { key: meeting.id, label: `${meeting.body} · ${dateLabel(meeting.date as string)}`, meeting, files };
    (meeting.timing === 'UPCOMING' ? upcoming : past).push(row);
  }
  upcoming.sort((a, b) => (a.meeting.date as string).localeCompare(b.meeting.date as string));
  past.sort((a, b) => (b.meeting.date as string).localeCompare(a.meeting.date as string));
  return { upcoming, past, rail: [...rail.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])), unplotted };
}

function rowName(row: Row): string {
  const [first, ...revised] = row.files;
  const lead = (file: PlottedFile) => `${Math.round(-file.hours).toLocaleString('en-US')} hours ${file.hours <= 0 ? 'before' : 'after'} stated start`;
  return `${row.label}: agenda posted ${lead(first)}${revised.map((file) => `; revised copy ${lead(file)}`).join('')}.`;
}

function layout(rows: readonly Row[], upcomingCount: number, width: number) {
  const base = rowLayout(rows, width, { band: BAND, top: TOP });
  // A gap for the 'today' rule between upcoming and past meetings.
  const split = upcomingCount > 0 && upcomingCount < rows.length ? 14 : 0;
  const placed = base.rows.map((row, i) => (i >= upcomingCount ? { ...row, labelY: row.labelY + split, markY: row.markY + split } : row));
  const todayY = split ? placed[upcomingCount].markY - (placed[upcomingCount].inline ? 10 : 28) : null;
  return { rows: placed, plotX: base.plotX, axisY: base.height + split, todayY };
}

function Plot({ rows, upcomingCount, width, onOpen }: { rows: Row[]; upcomingCount: number; width: number; onOpen(meetingId: string): void }) {
  const tooltip = useTooltip();
  const { rows: placed, plotX, axisY, todayY } = layout(rows, upcomingCount, width);
  const hours = rows.flatMap((row) => row.files.map((file) => file.hours));
  const x = scaleLinear([Math.min(-YARDSTICK_HOURS * 1.1, ...hours), Math.max(0, ...hours)], [plotX + 8, width - 12], { zero: true });
  const files = rows.flatMap((row) => row.files.map((file) => ({ row, file })));
  const roving = useRovingFocus({ shape: { rows: Math.max(1, files.length) }, onActivate: (i) => files[i] && onOpen(files[i].row.meeting.id), onEscape: tooltip.hide });
  const zero = Math.round(x(0)) + 0.5;
  const late = hours.some((h) => h > 0);
  let index = 0;
  return (
    <>
      <Grid scale={x} extent={[TOP, axisY]} vertical />
      <ReferenceLine kind="yardstick" axis="x" at={x(-YARDSTICK_HOURS)} extent={[2, axisY]} label="72 h before start (reference; no notice rule is evaluated)" labelSide="start" />
      <line x1={zero} x2={zero} y1={TOP - 4} y2={axisY} style={{ stroke: VIZ.ink, strokeWidth: 1 }} aria-hidden="true" />
      <text x={zero - 4} y={12} textAnchor="end" className={MUTED_LABEL_CLASS}>stated start</text>
      {late && <text x={zero + 4} y={12} className={MUTED_LABEL_CLASS}>header later than stated start</text>}
      {todayY !== null && <ReferenceLine kind="today" axis="y" at={todayY} extent={[0, width]} label="Today: upcoming above, past below" />}
      {placed.map((place, r) => {
        const row = rows[r];
        const cy = place.markY + BAND / 2;
        const xs = row.files.map((file) => x(file.hours));
        return (
          <g key={place.key} aria-label={rowName(row)}>
            <RowLabel row={place} title={rowName(row)} />
            {xs.length > 1 && <line x1={Math.min(...xs)} x2={Math.max(...xs)} y1={cy} y2={cy} style={{ stroke: VIZ.context, strokeWidth: 1 }} aria-hidden="true" />}
            {row.files.map((file, j) => {
              const i = index++;
              const minutes = file.document.minutesBeforeMeeting as number;
              const sentence = `${row.label}, ${file.document.label}: Last-Modified ${utc(file.lastModified)}, ${leadTimeText(minutes)}${j > 0 ? ' (revised copy)' : ''}.`;
              const content = (
                <TooltipContent
                  title={file.document.label}
                  rows={[
                    { key: 'lead', value: leadTimeText(minutes), label: 'per Last-Modified' },
                    { key: 'lm', value: utc(file.lastModified), label: 'Last-Modified' },
                  ]}
                  note={file.document.differingFetches > 0 ? `${file.document.differingFetches} later ${file.document.differingFetches === 1 ? 'fetch' : 'fetches'} returned different bytes; the earlier copy was not kept.` : undefined}
                />
              );
              return (
                <g key={file.key}>
                  <Dot
                    cx={xs[j]}
                    cy={cy}
                    role="default"
                    interaction={{
                      name: sentence,
                      item: roving.getItemProps(i),
                      onClick: () => onOpen(row.meeting.id),
                      onPointerMove: (event) => tooltip.show(content, event),
                      onPointerLeave: tooltip.hide,
                      onFocusMark: (box) => tooltip.show(content, box),
                      onBlurMark: tooltip.hide,
                    }}
                  />
                  {file.document.differingFetches > 0 && (
                    <text x={xs[j] + 9} y={cy} dy="0.32em" className={MUTED_LABEL_CLASS}>
                      <title>{`${file.document.differingFetches} later ${file.document.differingFetches === 1 ? 'fetch' : 'fetches'} returned different bytes`}</title>
                      {`↻ ${file.document.differingFetches}`}
                    </text>
                  )}
                </g>
              );
            })}
          </g>
        );
      })}
      <Axis scale={x} kind="hours" orient="bottom" offset={axisY} />
      {tooltip.node}
    </>
  );
}

/**
 * F11 PostingLeadTime: each agenda file's Last-Modified header against its meeting's stated start,
 * on hours relative to the start (earlier reads left). The 72 h line is a yardstick, not a notice
 * rule, and dots left of it are not recoloured. Files with no posting time are counted in the rail
 * by reason and never plotted.
 */
export default function PostingLeadTime({ meetings, counts, onOpen }: { meetings: CalendarMeeting[]; counts: PostingCounts; onOpen(meetingId: string): void }) {
  const { upcoming, past, rail, unplotted } = postingLeadTimeData(meetings);
  const rows = [...upcoming, ...past];
  // Only a file whose header is not recorded has no posting time; one on a meeting with no stated start or date has one but is not plotted.
  const noTime = unplotted.filter(({ document }) => document.posting.state === 'NOT_RECORDED').length;
  const notPlotted = unplotted.length;
  const headline = counts.agendaFilesWithBothTimes > 0
    ? `${counts.under72HoursBeforeStart} of ${counts.agendaFilesWithBothTimes} agenda files with both times recorded were posted less than 72 hours before the stated start, per Last-Modified${noTime ? `; ${noTime} ${noTime === 1 ? 'file has' : 'files have'} no posting time` : ''}.`
    : null;
  const table: TwinTable = {
    caption: 'Agenda posting times',
    columns: [{ key: 'meeting', label: 'Meeting' }, { key: 'file', label: 'Agenda file' }, { key: 'lm', label: 'Last-Modified (UTC)' }, { key: 'lead', label: 'Relative to stated start' }],
    rows: [
      ...rows.flatMap((row) => row.files.map((file) => ({ meeting: row.label, file: file.document.label, lm: utc(file.lastModified), lead: leadTimeText(file.document.minutesBeforeMeeting as number) }))),
      ...unplotted.map(({ meeting, document, reason }) => ({ meeting: `${meeting.body} · ${meeting.date ? dateLabel(meeting.date) : 'date not recorded'}`, file: document.label, lm: document.posting.state === 'RECORDED' ? utc(document.posting.lastModified) : absent('excluded', reason), lead: absent('excluded', reason) })),
    ],
  };
  const minHeight = (width: number) => (rows.length ? layout(rows, upcoming.length, width).axisY + 28 : 80);
  return (
    <div className="grid gap-2">
      <ChartFrame
        title="Agenda posting lead time"
        headline={headline}
        sources={{ tables: ['documents', 'document_acquisitions', 'portals'], note: 'Last-Modified is the server’s statement, not proof of public availability' }}
        summary={`${rows.length} meetings with ${rows.reduce((sum, row) => sum + row.files.length, 0)} plotted agenda files; ${notPlotted} agenda files are not plotted, ${noTime} of them for want of a posting time.`}
        status={rows.length ? 'ready' : 'empty'}
        emptyText="No agenda file has both a posting time and a stated start time, so nothing is plotted."
        table={table}
        legend={[
          { key: 'dot', label: 'Agenda file (Last-Modified)', swatch: 'dot', tone: 'mark' },
          { key: 'rev', label: 'Copies of one agenda', swatch: 'line', tone: 'context' },
          { key: '72', label: '72 h reference (not a rule)', swatch: 'dashed-line', tone: 'context' },
        ]}
        minHeight={minHeight(640)}
        plotHeight={minHeight}
        interactive={{ roledescription: 'dot plot' }}
        storageKey="meetings-posting-lead-time"
      >
        {({ width }) => <Plot rows={rows} upcomingCount={upcoming.length} width={width} onOpen={onOpen} />}
      </ChartFrame>
      {rail.length > 0 && (
        <p aria-label="Not plotted" className="text-caption text-muted">
          <span className="font-medium text-fg-secondary">Not plotted: </span>
          {rail.map(([reason, count]) => `${reason} ${count}`).join(' · ')}
        </p>
      )}
    </div>
  );
}
