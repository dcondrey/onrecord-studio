import { useCallback, useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  CalendarMeeting,
  FindingCitation,
  MeetingCalendar,
  MeetingDocumentRole,
  MeetingSuggestion,
  MeetingTiming,
  SuggestionBasis,
  SuggestionReasonKind,
} from '../../types';

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

function Locator({ citation }: { citation: FindingCitation }) {
  const range =
    citation.characterStart !== null && citation.characterEnd !== null
      ? `characters ${citation.characterStart}–${citation.characterEnd}`
      : 'character range not recorded';
  return (
    <span className="text-label text-muted">
      {citation.label ?? `document ${citation.documentId}`} · {range}
    </span>
  );
}

function Quote({ citation }: { citation: FindingCitation }) {
  return (
    <blockquote className="mt-1 border-l-2 border-line-strong pl-2 text-label leading-relaxed text-fg-secondary">
      “{citation.quote ?? 'quote not recorded'}”
      <div><Locator citation={citation} /></div>
    </blockquote>
  );
}

function SuggestionDetail({ suggestion, draft, onDraft }: { suggestion: MeetingSuggestion; draft: string; onDraft: (text: string) => void }) {
  const [copied, setCopied] = useState<string | null>(null);
  const copy = () => {
    const id = suggestion.id;
    void navigator.clipboard?.writeText(draft).then(() => setCopied(id)).catch(() => setCopied(null));
  };
  return (
    <div className="mt-3 flex flex-col gap-3 rounded-lg border border-line p-3" aria-label={`Suggestion ${suggestion.agendaItem}`}>
      <ul className="flex flex-col gap-2">
        {suggestion.reasons.map((reason, index) => (
          <li key={`${reason.kind}:${index}`} className="text-label text-fg-secondary">
            <div className="flex flex-wrap gap-2 text-label font-semibold">
              <span className="text-accent">{REASON_TEXT[reason.kind]}</span>
              <span className="text-warning">Basis: {BASIS_TEXT[reason.basis]}</span>
            </div>
            <p className="mt-1">{reason.reason}</p>
            <Quote citation={reason.citation} />
          </li>
        ))}
      </ul>
      <label className="flex flex-col gap-1 text-label text-fg-secondary">
        Public comment draft (edit before using; nothing is sent)
        <textarea
          className="min-h-[220px] rounded border border-control bg-transparent p-2 font-mono text-label text-fg"
          value={draft}
          onChange={(event) => onDraft(event.target.value)}
        />
      </label>
      <div className="flex items-center gap-2">
        <button type="button" className="analysis-button" onClick={copy}>Copy draft</button>
        {copied === suggestion.id && <span className="text-label text-success">Copied</span>}
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
    <article className="rounded-xl border border-line-strong p-4" aria-label={`${meeting.body} ${meeting.date ?? 'date not recorded'}`}>
      <div className="text-label leading-4 text-accent">{meeting.date ?? 'Date not recorded'}</div>
      <h4 className="mt-1 text-sm font-semibold text-fg">{meeting.body}</h4>
      <div className="text-label text-muted">{meeting.jurisdiction}</div>
      {meeting.dateNote && <p className="mt-1 text-label text-warning">{meeting.dateNote}</p>}
      <ul className="mt-2 flex flex-wrap gap-2 text-label">
        {meeting.documents.map((document) => (
          <li key={`${document.portalKey}:${document.documentId}`} className="rounded border border-line px-2 py-0.5 text-fg-secondary">
            {ROLE_TEXT[document.role]}:{' '}
            {document.sourceUrl ? <a className="underline" href={document.sourceUrl} target="_blank" rel="noreferrer">{document.label}</a> : document.label}
          </li>
        ))}
      </ul>
      <div className="mt-2 text-label text-fg-secondary">
        Comment deadline:{' '}
        {meeting.commentDeadlines.length === 0 ? <span className="text-muted">not recorded</span> : <Quote citation={meeting.commentDeadlines[0]} />}
      </div>
      {meeting.suggestions.length > 0 && (
        <div className="mt-3">
          <div className="app-eyebrow">Items to watch</div>
          <ul className="mt-1 flex flex-col gap-1">
            {meeting.suggestions.map((suggestion) => (
              <li key={suggestion.id}>
                <button
                  type="button"
                  aria-expanded={selected === suggestion.id}
                  className="text-left text-label text-fg underline-offset-2 hover:underline"
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
        setError(String(reason));
        setLoading(false);
      });
  }, [gate]);

  useEffect(() => {
    load();
    return gate.invalidate;
  }, [load, gate]);

  return (
    <main className="flex min-h-0 flex-1 flex-col overflow-y-auto p-5 lg:p-6">
      <div className="app-eyebrow text-highlight">Meetings</div>
      <h2 className="mt-1.5 text-[26px] font-semibold tracking-[-0.035em] text-fg">Public meetings in your records</h2>
      <p className="mt-2 max-w-3xl text-label leading-relaxed text-muted">
        Meetings, bodies and dates come only from agendas, minutes and videos already collected. Suggestions quote the agenda and
        name what they rest on. They are leads to read, not conclusions about an item. Today counts as upcoming.
      </p>
      <div className="mt-3">
        <button type="button" className="analysis-button" onClick={load}>
          {loading ? 'Reading documents…' : 'Refresh'}
        </button>
      </div>
      {error && <p className="mt-3 text-label leading-4 text-warning">Meetings unavailable: {error}</p>}
      {calendar && calendar.meetings.length === 0 && (
        <p className="mt-4 text-label leading-4 text-muted">No agenda, minutes or meeting video has been collected yet.</p>
      )}
      {calendar && SECTIONS.map(({ timing, title }) => {
        const meetings = calendar.meetings.filter((meeting) => meeting.timing === timing);
        if (meetings.length === 0) return null;
        return (
          <section key={timing} aria-label={`${title} meetings`} className="mt-5">
            <h3 className="text-sm font-semibold text-fg">{title}</h3>
            <div className="mt-2 grid gap-3 lg:grid-cols-2">
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
        <details className="mt-5 text-label text-muted">
          <summary className="cursor-pointer">Limitations</summary>
          <ul className="mt-1 list-disc pl-4">
            {calendar.limitations.map((limitation) => <li key={limitation}>{limitation}</li>)}
          </ul>
        </details>
      )}
    </main>
  );
}
