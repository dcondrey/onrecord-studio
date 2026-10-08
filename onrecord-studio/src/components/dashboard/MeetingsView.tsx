import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  AgendaItemChangeKind,
  AgendaRevision,
  CalendarMeeting,
  FindingCitation,
  MeetingCalendar,
  MeetingDocument,
  MeetingDocumentRole,
  MeetingSuggestion,
  MeetingTiming,
  PostingAbsence,
  SuggestionBasis,
  SuggestionReasonKind,
} from '../../types';
import PageHeader from '../ui/PageHeader';
import EmptyState from '../ui/EmptyState';
import InlineError from '../ui/InlineError';
import Button from '../ui/Button';
import { Field, Textarea } from '../ui/Field';
import { openSourceConnector } from './sourceEvents';

const SECTIONS: Array<{ timing: MeetingTiming; title: string }> = [
  { timing: 'UPCOMING', title: 'Upcoming' },
  { timing: 'PAST', title: 'Past' },
  { timing: 'DATE_NOT_RECORDED', title: 'Date not recorded' },
];

const ROLE_TEXT: Record<MeetingDocumentRole, string> = { AGENDA: 'Agenda', MINUTES: 'Minutes', VIDEO: 'Video' };

const BASIS_TEXT: Record<SuggestionBasis, string> = { COUNTED: 'Counted', STATED: 'Stated', PATTERN: 'Pattern' };

const REASON_TEXT: Record<SuggestionReasonKind, string> = {
  INVESTIGATION_TOPIC: 'Investigation topic',
  WATCHED_AGENCY: 'Watched agency',
  WATCHED_ENTITY: 'Watched party',
  FINDING_SUBJECT: 'Stored finding subject',
  EVIDENCE_GAP: 'Evidence gap (untested)',
  CHECKABLE_FIGURE: 'Stated figure',
};

const ABSENCE_TEXT: Record<PostingAbsence, string> = {
  NO_FETCH_RECORD: 'no fetch of this file is recorded',
  NO_HEADER: 'the server sent no Last-Modified header',
  UNREADABLE_HEADER: 'the Last-Modified header is not a readable date',
  REQUEST_TIME_HOST: 'this host reports the time of the request as Last-Modified',
  HEADER_IS_REQUEST_TIME: 'the Last-Modified header equals the time of the request',
  REQUEST_TIME_NOT_RULED_OUT: 'the header is too close to the fetch to tell it from the time of the request',
};

const CHANGE_TEXT: Record<AgendaItemChangeKind, string> = {
  ADDED: 'Added',
  REMOVED: 'Removed',
  REWORDED: 'Title reworded',
  TEXT_CHANGED: 'Text changed below the title',
};

const plural = (count: number, unit: string) => `${count} ${unit}${count === 1 ? '' : 's'}`;

/** The span between the header time and the stated start, in whole units; never rounded up. */
function describeLeadTime(minutes: number): string {
  const size = Math.abs(minutes);
  const days = Math.floor(size / 1440);
  const hours = Math.floor((size % 1440) / 60);
  const rest = size % 60;
  const span = days > 0
    ? [plural(days, 'day'), hours > 0 ? plural(hours, 'hour') : null].filter(Boolean).join(' ')
    : [hours > 0 ? plural(hours, 'hour') : null, rest > 0 || hours === 0 ? plural(rest, 'minute') : null].filter(Boolean).join(' ');
  return minutes < 0 ? `${span} after the start` : `${span} before the start`;
}

// A meeting date is a calendar date as the agenda states it, so it is formatted without a zone shift.
const MEETING_DATE = new Intl.DateTimeFormat(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' });
const POSTED_AT = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });

function formatMeetingDate(date: string): string {
  const parsed = new Date(`${date}T00:00:00Z`);
  return Number.isNaN(parsed.valueOf()) ? date : MEETING_DATE.format(parsed);
}

/** "14:00" as stated on the agenda, read as a wall-clock time. */
function formatStatedTime(time: string): string {
  const match = /^(\d{1,2}):(\d{2})/.exec(time);
  if (!match) return time;
  const hour = Number(match[1]);
  return `${hour % 12 === 0 ? 12 : hour % 12}:${match[2]} ${hour < 12 ? 'AM' : 'PM'}`;
}

function Posting({ document }: { document: MeetingDocument }) {
  const { posting } = document;
  const postedAt = posting.state === 'RECORDED' ? new Date(posting.lastModified) : null;
  return (
    <div className="mt-0.5 text-caption text-muted">
      {posting.state === 'RECORDED' ? (
        <span title={`Last-Modified header: ${posting.lastModified.slice(0, 16).replace('T', ' ')} UTC`}>
          Posted <time dateTime={posting.lastModified} className="tabular-nums">{postedAt && !Number.isNaN(postedAt.valueOf()) ? POSTED_AT.format(postedAt) : posting.lastModified}</time>, per the file’s Last-Modified header
          {document.minutesBeforeMeeting !== null && ` · ${describeLeadTime(document.minutesBeforeMeeting)}`}
        </span>
      ) : posting.reason === 'NO_FETCH_RECORD' ? (
        <>Posting time not recorded: this file was never fetched</>
      ) : (
        <>Posting time not recorded: {ABSENCE_TEXT[posting.reason]}</>
      )}
      {document.differingFetches > 0 && (
        <div>
          {document.differingFetches === 1 ? '1 fetch' : `${document.differingFetches} fetches`} of this address returned different bytes. The earlier
          text was not kept, so those copies are not compared.
        </div>
      )}
    </div>
  );
}

function Revision({ revision }: { revision: AgendaRevision }) {
  return (
    <li className="py-2 text-body text-fg-secondary">
      <div>
        {revision.earlierLabel} → {revision.laterLabel}{' '}
        <span className="text-muted">
          ({revision.orderBasis === 'LAST_MODIFIED' ? 'ordered by Last-Modified header' : 'ordered by when the app collected them; no posting time on one copy'})
        </span>
      </div>
      {revision.changes.length === 0 ? (
        <p className="mt-1 text-muted">No item was added, removed or changed between these copies.</p>
      ) : (
        <ul className="mt-1 flex flex-col gap-1">
          {revision.changes.map((change) => (
            <li key={`${change.kind}:${change.item}`}>
              <span className="font-medium text-fg">{CHANGE_TEXT[change.kind]}: {change.item}</span>
              {change.kind === 'REWORDED' ? (
                <div>
                  <div>Earlier: “{change.before}”</div>
                  <div>Later: “{change.after}”</div>
                </div>
              ) : (
                (change.before ?? change.after) && <span> “{change.before ?? change.after}”</span>
              )}
            </li>
          ))}
        </ul>
      )}
      {revision.titlesNotCompared.length > 0 && (
        <p className="mt-1 text-muted">
          Wording not compared (the title has no proven end, or the item number repeats): {revision.titlesNotCompared.join(', ')}.
        </p>
      )}
    </li>
  );
}

function Locator({ citation }: { citation: FindingCitation }) {
  const range =
    citation.characterStart !== null && citation.characterEnd !== null
      ? `characters ${citation.characterStart}–${citation.characterEnd}`
      : 'character range not recorded';
  return (
    <span className="text-caption text-muted">
      {citation.label ?? `document ${citation.documentId}`} · {range}
    </span>
  );
}

function Quote({ citation }: { citation: FindingCitation }) {
  return (
    <blockquote className="mt-1 border-l-2 border-line-strong pl-3 text-body text-fg-secondary">
      “{citation.quote ?? 'quote not recorded'}”
      <div><Locator citation={citation} /></div>
    </blockquote>
  );
}

function SuggestionDetail({ suggestion, draft, onDraft }: { suggestion: MeetingSuggestion; draft: string; onDraft: (text: string) => void }) {
  const [copy, setCopy] = useState<'idle' | 'copied' | 'blocked'>('idle');
  const copyDraft = async () => {
    try {
      await navigator.clipboard.writeText(draft);
      setCopy('copied');
    } catch {
      setCopy('blocked');
    }
  };
  return (
    <div className="mt-3 flex flex-col gap-3 rounded-lg bg-raised/60 p-3" aria-label={`Suggestion ${suggestion.agendaItem}`}>
      <ul className="flex flex-col gap-2">
        {suggestion.reasons.map((reason, index) => (
          <li key={`${reason.kind}:${index}`} className="text-body text-fg-secondary">
            <div className="flex flex-wrap gap-2 text-caption font-medium">
              <span className="text-fg">{REASON_TEXT[reason.kind]}</span>
              <span className="text-muted">Basis: {BASIS_TEXT[reason.basis]}</span>
            </div>
            <p className="mt-1">{reason.reason}</p>
            <Quote citation={reason.citation} />
          </li>
        ))}
      </ul>
      <Field label="Public comment draft (edit before using; nothing is sent)">
        <Textarea className="min-h-[220px]" value={draft} onChange={(event) => onDraft(event.target.value)} />
      </Field>
      <div className="flex items-center gap-2">
        <Button size="sm" onClick={() => void copyDraft()}>Copy draft</Button>
        {copy === 'copied' && <span role="status" className="text-caption text-success">Copied</span>}
        {copy === 'blocked' && <span role="status" className="text-caption text-warning">Copying was blocked by the system. Select the text above to copy it.</span>}
      </div>
    </div>
  );
}

function MeetingCard({ meeting, selected, onSelect, drafts, onDraft }: {
  meeting: CalendarMeeting;
  selected: string | null;
  onSelect: (id: string | null) => void;
  drafts: Record<string, string>;
  onDraft: (id: string, text: string) => void;
}) {
  return (
    <article className="app-panel divide-y divide-line" aria-label={`${meeting.body} ${meeting.date ?? 'date not recorded'}`}>
      <div className="p-4">
        <p className="text-caption tabular-nums text-fg-secondary" title={meeting.startTime ? `${meeting.startTime} as stated on the agenda` : undefined}>
          {meeting.date ? formatMeetingDate(meeting.date) : 'Date not recorded'}
          {meeting.date && (meeting.startTime ? ` · ${formatStatedTime(meeting.startTime)}` : ' · start time not recorded')}
        </p>
        <h4 className="mt-1 text-heading text-fg">{meeting.body}</h4>
        <p className="text-caption text-muted">{meeting.jurisdiction}</p>
        {meeting.dateNote && <p className="mt-1 text-caption text-warning">{meeting.dateNote}</p>}
      </div>
      <ul className="flex flex-col gap-2 p-4 text-body">
        {meeting.documents.map((document) => (
          <li key={`${document.portalKey}:${document.documentId}`} className="text-fg-secondary">
            {ROLE_TEXT[document.role]}:{' '}
            {document.sourceUrl ? <a className="focus-ring rounded text-accent underline-offset-2 hover:underline" href={document.sourceUrl} target="_blank" rel="noreferrer">{document.label}<span className="sr-only"> (opens in browser)</span></a> : document.label}
            <Posting document={document} />
          </li>
        ))}
      </ul>
      {meeting.revisions.length > 0 && (
        <div className="p-4" aria-label="Agenda copies compared">
          <h5 className="text-body font-medium text-fg">Agenda copies compared</h5>
          <ul className="mt-1 divide-y divide-line">
            {meeting.revisions.map((revision) => (
              <Revision key={`${revision.earlierDocumentId}:${revision.laterDocumentId}`} revision={revision} />
            ))}
          </ul>
        </div>
      )}
      <div className="p-4 text-body text-fg-secondary">
        Comment deadline:{' '}
        {meeting.commentDeadlines.length === 0 ? <span className="text-muted">not recorded</span> : <Quote citation={meeting.commentDeadlines[0]} />}
      </div>
      {meeting.suggestions.length > 0 && (
        <div className="p-4">
          <h5 className="text-body font-medium text-fg">Items to watch</h5>
          <ul className="mt-1 flex flex-col gap-1">
            {meeting.suggestions.map((suggestion) => (
              <li key={suggestion.id}>
                <button
                  type="button"
                  aria-expanded={selected === suggestion.id}
                  className="focus-ring rounded text-left text-body text-accent underline-offset-2 hover:underline"
                  onClick={() => onSelect(selected === suggestion.id ? null : suggestion.id)}
                >
                  {suggestion.agendaItem}: {suggestion.title}
                </button>
                {selected === suggestion.id && (
                  <SuggestionDetail
                    suggestion={suggestion}
                    draft={drafts[suggestion.id] ?? suggestion.comment.text}
                    onDraft={(text) => onDraft(suggestion.id, text)}
                  />
                )}
              </li>
            ))}
          </ul>
        </div>
      )}
    </article>
  );
}

export default function MeetingsView() {
  const [calendar, setCalendar] = useState<MeetingCalendar | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const gate = useLatestRequestGate();

  const load = useCallback(() => {
    const request = gate.begin();
    setLoading(true);
    invokeCommand('get_meeting_calendar', {})
      .then((result) => {
        if (!gate.isCurrent(request)) return;
        setCalendar(result);
        setError(null);
        setLoading(false);
      })
      .catch((reason: unknown) => {
        if (!gate.isCurrent(request)) return;
        setError(formatNativeError(reason));
        setLoading(false);
      });
  }, [gate]);

  useEffect(() => {
    load();
    return gate.invalidate;
  }, [load, gate]);

  return (
    <main className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-content space-y-6 px-4 pb-10 pt-6 md:px-6">
        <PageHeader
          title="Public meetings in your records"
          summary="Meetings, bodies and dates come only from agendas, minutes and videos already collected."
          method={<>
            Suggestions quote the agenda and name what they rest on. They are leads to read, not conclusions about an item. Today
            counts as upcoming. A posting time is the Last-Modified header the server sent with the file: the server’s statement about
            the file, not proof of when the public could first read it, and no notice requirement is evaluated here.
          </>}
          actions={<Button onClick={load}>{loading ? 'Refreshing…' : 'Refresh'}</Button>}
        />
        {calendar && calendar.postingCounts.agendaFilesWithBothTimes > 0 && (
          <p className="max-w-prose text-body text-fg-secondary" aria-label="Posting time counts">
            <span className="tabular-nums">{calendar.postingCounts.under72HoursBeforeStart} of {calendar.postingCounts.agendaFilesWithBothTimes}</span> agenda files with a recorded posting
            time and a stated start time were posted less than 72 hours before the meeting, per the file’s Last-Modified header.
            {calendar.postingCounts.afterMeetingStart > 0 && ` ${calendar.postingCounts.afterMeetingStart} carry a header later than the stated start.`}
          </p>
        )}
        {error && <InlineError summary="Meetings couldn’t be read from your records." detail={error} onRetry={load} />}
        {!calendar && !error && (
          <div role="status" aria-busy="true" className="grid items-start gap-4 lg:grid-cols-2">
            <span className="sr-only">Reading meeting documents</span>
            {[0, 1].map((card) => (
              <div key={card} aria-hidden="true" className="flex h-48 flex-col gap-3 rounded-xl border border-line bg-panel p-4">
                <div className="skeleton h-3 w-40 rounded" /><div className="skeleton h-5 w-2/3 rounded" /><div className="skeleton h-3 w-1/3 rounded" />
                <div className="mt-auto skeleton h-3 w-3/4 rounded" />
              </div>
            ))}
          </div>
        )}
        {calendar && calendar.meetings.length === 0 && (
          <EmptyState
            icon="calendar"
            title="No meetings yet"
            body="Meetings appear when agendas, minutes or videos are collected."
            actions={<Button variant="primary" onClick={openSourceConnector}>Add an agenda source</Button>}
          />
        )}
        {calendar && SECTIONS.map(({ timing, title }) => {
          const meetings = calendar.meetings.filter((meeting) => meeting.timing === timing);
          if (meetings.length === 0) return null;
          return (
            <section key={timing} aria-label={`${title} meetings`}>
              <h3 className="text-heading text-fg">{title}</h3>
              <div className="mt-3 grid items-start gap-4 lg:grid-cols-2">
                {meetings.map((meeting) => (
                  <MeetingCard
                    key={meeting.id}
                    meeting={meeting}
                    selected={selected}
                    onSelect={setSelected}
                    drafts={drafts}
                    onDraft={(id, text) => setDrafts((current) => ({ ...current, [id]: text }))}
                  />
                ))}
              </div>
            </section>
          );
        })}
        {calendar && calendar.limitations.length > 0 && (
          <details className="group text-body text-muted">
            <summary className="focus-ring cursor-pointer rounded">Limitations</summary>
            <ul className="mt-1 list-disc pl-4">
              {calendar.limitations.map((limitation) => <li key={limitation}>{limitation}</li>)}
            </ul>
          </details>
        )}
      </div>
    </main>
  );
}
