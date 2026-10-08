import { useEffect, useId, useRef, useState, type FormEvent } from 'react';
import { open as chooseFiles } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import Button, { buttonClass } from '../ui/Button';
import Dialog, { DialogBody, DialogHeader } from '../ui/Dialog';
import Skeleton from '../ui/Skeleton';
import { Field, Input, Select } from '../ui/Field';
import InlineError from '../ui/InlineError';
import { formatShortDate } from '../objects/labels';
import { cn } from '../../utils/cn';
import { CALIFORNIA_CPRA_JURISDICTION, TRACKER_LIMITS } from '../../types';
import type {
  AddTimelineEventInput,
  ClaimedRequest,
  RequestDeadline,
  StatutoryExtensionReason,
  SuggestedEvent,
  TimelineEventKind,
} from '../../types';
import { DeadlineChip } from './DeadlineChip';
import { localInputToRfc3339, rfc3339ToLocalInput } from './rfc3339';
import { useMyRequests } from './useMyRequests';
import ClockStrip, { trackerClock } from '../charts/figures/ClockStrip';
import DeadlineRunway, { runwayModel } from '../charts/figures/DeadlineRunway';
import { displayError } from '../../utils/errorMessage';

export const KIND_LABELS: Record<TimelineEventKind, string> = {
  SUBMITTED: 'Submitted',
  ACKNOWLEDGED: 'Acknowledged',
  EXTENSION_NOTICE: 'Extension notice',
  DETERMINATION: 'Determination',
  RECORDS_RELEASED: 'Records released',
  CLOSED: 'Closed',
  CLARIFICATION_REQUESTED: 'Clarification requested by agency',
  FEE_NOTICE: 'Fee notice',
  PARTIAL_PRODUCTION: 'Partial production (installment)',
  WITHHOLDING_NOTICE: 'Withholding / exemption notice',
  FOLLOW_UP_SENT: 'Follow-up sent',
  APPEAL_FILED: 'Appeal or escalation filed',
  OTHER: 'Other',
};

const REASON_LABELS: Record<StatutoryExtensionReason, string> = {
  SEPARATE_FIELD_FACILITIES: 'Records at separate field facilities',
  VOLUMINOUS_RECORDS: 'Voluminous records',
  INTERAGENCY_CONSULTATION: 'Consultation with another agency',
  DATA_COMPILATION: 'Data compilation or programming',
  CYBERATTACK_SYSTEMS_UNAVAILABLE: 'Cyberattack made systems unavailable',
  DECLARED_EMERGENCY_OPERATIONAL_IMPACT: 'Declared emergency affected operations',
};

const button = cn(buttonClass({ size: 'sm' }), 'focus-ring');
const primaryButton = cn(buttonClass({ variant: 'primary', size: 'sm' }), 'focus-ring');

function KindSelect({ value, onChange, label }: { value: TimelineEventKind; onChange: (kind: TimelineEventKind) => void; label: string }) {
  return (
    <Select label={label} className="focus-ring" value={value} onChange={(event) => onChange(event.target.value as TimelineEventKind)}>
      {(Object.keys(KIND_LABELS) as TimelineEventKind[]).map((kind) => (
        <option key={kind} value={kind}>{KIND_LABELS[kind]}</option>
      ))}
    </Select>
  );
}

export function chronologyText(claim: ClaimedRequest): string {
  const events = [...claim.events].sort((a, b) => Date.parse(a.occurredAt) - Date.parse(b.occurredAt));
  const lines = events.map((event) => {
    const source = event.basis === 'EMAIL_CONFIRMED' ? 'confirmed from an email' : 'entered by requester';
    const reason = event.extensionReason ? `; ${REASON_LABELS[event.extensionReason]}` : '';
    return `${event.occurredAt.slice(0, 10)}  ${KIND_LABELS[event.kind]} (${source}${reason})`;
  });
  return [`Request ${claim.reference}${claim.title ? `: ${claim.title}` : ''}`, ...lines].join('\n');
}

function ExtensionFields({
  reason,
  expected,
  onReason,
  onExpected,
}: {
  reason: StatutoryExtensionReason | '';
  expected: string;
  onReason: (value: StatutoryExtensionReason | '') => void;
  onExpected: (value: string) => void;
}) {
  return (
    <>
      <Select label="Statutory reason for the extension (required)" className="focus-ring" value={reason} onChange={(event) => onReason(event.target.value as StatutoryExtensionReason | '')}>
        <option value="">Choose the reason the notice gives</option>
        {(Object.keys(REASON_LABELS) as StatutoryExtensionReason[]).map((value) => (
          <option key={value} value={value}>{REASON_LABELS[value]}</option>
        ))}
      </Select>
      <Field label="Agency's promised response date (optional)" help="Without it, the extension is not applied to the deadline.">
        <Input type="datetime-local" className="focus-ring tabular-nums" value={expected} onChange={(event) => onExpected(event.target.value)} />
      </Field>
    </>
  );
}

function ClaimForm({ portals, onSubmit, isBusy }: {
  portals: { key: string; name: string }[];
  onSubmit: (input: { portalKey: string | null; reference: string; title: string | null; sourceUrl: string | null; jurisdiction: string | null }) => void;
  isBusy: boolean;
}) {
  const [reference, setReference] = useState('');
  const [title, setTitle] = useState('');
  const [sourceUrl, setSourceUrl] = useState('');
  const [portalKey, setPortalKey] = useState('');
  const [jurisdiction, setJurisdiction] = useState('');
  const [otherJurisdiction, setOtherJurisdiction] = useState('');
  const headingId = useId();
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onSubmit({
      portalKey: portalKey || null,
      reference,
      title: title || null,
      sourceUrl: sourceUrl || null,
      jurisdiction: jurisdiction === 'OTHER' ? otherJurisdiction || null : jurisdiction || null,
    });
    setReference('');
    setTitle('');
    setSourceUrl('');
  };
  return (
    <form onSubmit={submit} className="flex flex-col gap-3 border-t border-line pt-5" aria-labelledby={headingId}>
      <div>
        <h3 id={headingId} className="text-heading text-fg">Track a request</h3>
        <p className="mt-1 text-caption text-muted">
          Nothing here is sent anywhere, and no portal login is used or stored. Only the dates you enter or confirm from emails count.
        </p>
      </div>
      <Field label="Request number, as the portal shows it">
        <Input required maxLength={TRACKER_LIMITS.referenceCharacters} className="focus-ring" value={reference} onChange={(event) => setReference(event.target.value)} />
      </Field>
      <Field label="Title (optional)">
        <Input maxLength={TRACKER_LIMITS.titleCharacters} className="focus-ring" value={title} onChange={(event) => setTitle(event.target.value)} />
      </Field>
      <Field label="Link to the request (optional, https only)">
        <Input type="url" maxLength={TRACKER_LIMITS.sourceUrlCharacters} className="focus-ring" value={sourceUrl} onChange={(event) => setSourceUrl(event.target.value)} />
      </Field>
      <Select label="Portal (optional)" className="focus-ring" value={portalKey} onChange={(event) => setPortalKey(event.target.value)}>
        <option value="">Not linked to a registered portal</option>
        {portals.map((portal) => (
          <option key={portal.key} value={portal.key}>{portal.name}</option>
        ))}
      </Select>
      <Select label="Jurisdiction" className="focus-ring" value={jurisdiction} onChange={(event) => setJurisdiction(event.target.value)}>
        <option value="">Not set</option>
        <option value={CALIFORNIA_CPRA_JURISDICTION}>California (Public Records Act)</option>
        <option value="OTHER">Other (no deadline model)</option>
      </Select>
      {jurisdiction === 'OTHER' && (
        <Input aria-label="Jurisdiction name" maxLength={TRACKER_LIMITS.jurisdictionCharacters} className="focus-ring" value={otherJurisdiction} onChange={(event) => setOtherJurisdiction(event.target.value)} />
      )}
      <button type="submit" disabled={isBusy || reference.trim() === ''} className={cn(primaryButton, 'self-start')}>Track this request</button>
    </form>
  );
}

function SuggestionCard({ claim, suggestion, isBusy, onConfirm, onReject }: {
  claim: ClaimedRequest;
  suggestion: SuggestedEvent;
  isBusy: boolean;
  onConfirm: (input: { kind: TimelineEventKind; occurredAt: string; reason: StatutoryExtensionReason | null; expected: string | null }) => void;
  onReject: () => void;
}) {
  const [kind, setKind] = useState(suggestion.kind);
  const [when, setWhen] = useState(rfc3339ToLocalInput(suggestion.candidateOccurredAt));
  const [reason, setReason] = useState<StatutoryExtensionReason | ''>('');
  const [expected, setExpected] = useState('');
  const occurredAt = localInputToRfc3339(when);
  const needsReason = kind === 'EXTENSION_NOTICE';
  const ready = occurredAt !== null && (!needsReason || reason !== '');
  return (
    <li className="rounded-xl border border-dashed border-line-strong p-4" aria-label={`Suggested ${KIND_LABELS[suggestion.kind]} for ${claim.reference}`}>
      <p className="text-control text-fg">Suggested, not counted until you confirm</p>
      <blockquote className="mt-2 border-l-2 border-line-strong pl-2 italic">{suggestion.evidence.matchedSentence}</blockquote>
      <p className="mt-1 text-caption text-muted">
        From an email{suggestion.evidence.subjectExcerpt ? ` titled "${suggestion.evidence.subjectExcerpt}"` : ''}.
      </p>
      <div className="mt-3 grid gap-3">
        <KindSelect label="What this describes" value={kind} onChange={setKind} />
        <Field
          label="Date"
          help={suggestion.candidateOccurredAt
            ? 'Filled from the email date, which is not necessarily the date of the event.'
            : 'The email had no readable date; enter the date of the event.'}
        >
          <Input type="datetime-local" className="focus-ring tabular-nums" value={when} onChange={(event) => setWhen(event.target.value)} />
        </Field>
        {needsReason && <ExtensionFields reason={reason} expected={expected} onReason={setReason} onExpected={setExpected} />}
      </div>
      <div className="mt-3 flex gap-2">
        <button
          type="button"
          disabled={isBusy || !ready}
          className={button}
          onClick={() =>
            occurredAt &&
            onConfirm({
              kind,
              occurredAt,
              reason: needsReason && reason !== '' ? reason : null,
              expected: needsReason ? localInputToRfc3339(expected) : null,
            })
          }
        >
          Confirm this event
        </button>
        <button type="button" disabled={isBusy} className={button} onClick={onReject}>Reject</button>
      </div>
    </li>
  );
}

function Detail({ claim, deadline, tracker }: { claim: ClaimedRequest; deadline: RequestDeadline | undefined; tracker: ReturnType<typeof useMyRequests> }) {
  const [kind, setKind] = useState<TimelineEventKind>('SUBMITTED');
  const [when, setWhen] = useState('');
  const [reason, setReason] = useState<StatutoryExtensionReason | ''>('');
  const [expected, setExpected] = useState('');
  const [pickError, setPickError] = useState<string | null>(null);
  const [confirmingRemoval, setConfirmingRemoval] = useState(false);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  // Shared between the clock's ticks and the chronology rows: pointing at one marks the other.
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const eventRows = useRef(new Map<string, HTMLLIElement>());
  // A removed date stays recoverable for five seconds: it is the user's own evidence.
  const [removed, setRemoved] = useState<{ input: AddTimelineEventInput; label: string } | null>(null);
  const undoTimer = useRef<number | null>(null);
  useEffect(() => () => {
    if (undoTimer.current !== null) window.clearTimeout(undoTimer.current);
  }, []);
  const removeWithUndo = async (event: ClaimedRequest['events'][number]) => {
    await tracker.removeEvent(claim.id, event.id);
    if (undoTimer.current !== null) window.clearTimeout(undoTimer.current);
    setRemoved({
      input: {
        claimId: claim.id,
        kind: event.kind,
        occurredAt: event.occurredAt,
        extensionReason: event.extensionReason,
        expectedDeterminationAt: event.expectedDeterminationAt,
      },
      label: `${KIND_LABELS[event.kind]}, ${formatShortDate(event.occurredAt)}`,
    });
    undoTimer.current = window.setTimeout(() => setRemoved(null), 5000);
  };
  const undoRemove = async () => {
    if (!removed) return;
    if (undoTimer.current !== null) window.clearTimeout(undoTimer.current);
    setRemoved(null);
    await tracker.addEvent(removed.input);
  };
  const copyChronology = async () => {
    try {
      await navigator.clipboard.writeText(chronologyText(claim));
      setCopyStatus('Chronology copied');
    } catch {
      setCopyStatus('The clipboard is unavailable');
    }
  };
  const occurredAt = localInputToRfc3339(when);
  const needsReason = kind === 'EXTENSION_NOTICE';
  const events = [...claim.events].sort((left, right) => left.occurredAt.localeCompare(right.occurredAt));
  const outcome = tracker.importOutcome?.claimId === claim.id ? tracker.importOutcome.report : null;

  const choose = async () => {
    setPickError(null);
    try {
      const chosen = await chooseFiles({ multiple: true, directory: false, filters: [{ name: 'Email', extensions: ['eml', 'msg'] }] });
      const paths = chosen === null ? [] : Array.isArray(chosen) ? chosen : [chosen];
      if (paths.length > 0) await tracker.importEmails(claim.id, paths.slice(0, TRACKER_LIMITS.importPaths));
    } catch (reason) {
      setPickError(String(reason instanceof Error ? reason.message : reason));
    }
  };

  const focusEventRow = (eventId: string) => {
    const row = eventRows.current.get(eventId);
    row?.scrollIntoView?.({ block: 'nearest' });
    row?.focus();
    setHighlighted(eventId);
  };

  return (
    <div className="flex min-w-0 flex-col gap-6" aria-label={`Request ${claim.reference}`}>
      <section aria-label="Deadline">
        <h3 className="text-heading text-fg">{claim.title || `Request ${claim.reference}`}</h3>
        <p className="mt-0.5 text-caption tabular-nums text-muted">
          {claim.title ? `Request ${claim.reference}` : 'No title recorded'}
          {claim.sourceUrl && (
            <>
              {' · '}
              <a href={claim.sourceUrl} target="_blank" rel="noreferrer" title={claim.sourceUrl} className="focus-ring inline-block max-w-[16rem] truncate align-bottom text-accent underline-offset-2 hover:underline">
                {linkLabel(claim.sourceUrl)}
              </a>
            </>
          )}
        </p>
        {deadline ? (
          <div className="mt-3 grid min-w-0 gap-2">
            <ClockStrip
              data={trackerClock(claim, deadline, KIND_LABELS)}
              storageKey="my-requests-detail-clock"
              highlightedEvent={highlighted}
              onEventHover={setHighlighted}
              onEventActivate={focusEventRow}
            />
            <p className="text-caption text-muted" data-testid="basis-statement">{deadline.basisStatement}</p>
            {deadline.notes.map((note) => (
              <p key={note} className="text-caption text-muted">{note}</p>
            ))}
          </div>
        ) : (
          <Skeleton variant="lines" lines={2} label="Loading the deadline" className="mt-3" />
        )}
      </section>

      <section aria-label="Timeline" className="border-t border-line pt-5">
        <h3 className="text-heading text-fg">Timeline</h3>
        {events.length === 0 ? (
          <p className="mt-1 text-muted">No dates recorded yet.</p>
        ) : (
          <ol className="mt-2 flex flex-col gap-1.5">
            {events.map((event) => (
              <li
                key={event.id}
                tabIndex={-1}
                ref={(element) => {
                  if (element) eventRows.current.set(event.id, element);
                  else eventRows.current.delete(event.id);
                }}
                onPointerEnter={() => setHighlighted(event.id)}
                onPointerLeave={() => setHighlighted(null)}
                data-highlighted={highlighted === event.id ? '' : undefined}
                className={cn('flex items-start justify-between gap-2 rounded-lg border p-2 outline-none transition-colors', highlighted === event.id ? 'border-line-strong bg-raised' : 'border-line')}
              >
                <span className="text-fg-secondary">
                  <strong className="font-medium text-fg">{KIND_LABELS[event.kind]}</strong> on{' '}
                  <time dateTime={event.occurredAt} title={event.occurredAt} className="whitespace-nowrap tabular-nums">{formatShortDate(event.occurredAt)}</time> ·{' '}
                  {event.basis === 'EMAIL_CONFIRMED' ? 'confirmed from an email' : 'entered by you'}
                  {event.extensionReason && ` · ${REASON_LABELS[event.extensionReason]}`}
                  {event.evidence && <blockquote className="border-l-2 border-line-strong pl-1.5 italic text-fg-secondary">{event.evidence.matchedSentence}</blockquote>}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  iconOnly
                  icon="trash"
                  disabled={tracker.isBusy}
                  aria-label={`Remove ${KIND_LABELS[event.kind]} on ${formatShortDate(event.occurredAt)}`}
                  onClick={() => void removeWithUndo(event)}
                  className="focus-ring"
                />
              </li>
            ))}
          </ol>
        )}
        {claim.events.length > 0 && (
          <div className="mt-1.5 flex items-center gap-2">
            <button type="button" className={button} onClick={() => void copyChronology()}>Copy chronology</button>
          </div>
        )}
        {removed && (
          <div role="status" className="mt-1.5 flex items-center gap-2 rounded-lg border border-line bg-raised px-3 py-1.5">
            <span>Removed · {removed.label}</span>
            <button type="button" className={cn(button, 'ml-auto')} disabled={tracker.isBusy} onClick={() => void undoRemove()}>Undo</button>
            {copyStatus && <span role="status" className="text-success">{copyStatus}</span>}
          </div>
        )}
        <form
          className="mt-4 grid gap-3"
          aria-label="Add a date"
          onSubmit={(submitted) => {
            submitted.preventDefault();
            if (!occurredAt || (needsReason && reason === '')) return;
            void tracker
              .addEvent({
                claimId: claim.id,
                kind,
                occurredAt,
                extensionReason: needsReason && reason !== '' ? reason : null,
                expectedDeterminationAt: needsReason ? localInputToRfc3339(expected) : null,
              })
              .then(() => setWhen(''));
          }}
        >
          <KindSelect label="Kind of date" value={kind} onChange={setKind} />
          <Field label="Date and time">
            <Input type="datetime-local" className="focus-ring tabular-nums" value={when} onChange={(event) => setWhen(event.target.value)} />
          </Field>
          {needsReason && <ExtensionFields reason={reason} expected={expected} onReason={setReason} onExpected={setExpected} />}
          <button type="submit" className={cn(primaryButton, 'justify-self-start')} disabled={tracker.isBusy || !occurredAt || (needsReason && reason === '')}>Add this date</button>
        </form>
      </section>

      <section aria-label="Suggested events" className="border-t border-line pt-5">
        <h3 className="mb-2 text-heading text-fg">Suggested from emails</h3>
        {claim.suggestions.length === 0 ? (
          <p className="text-muted">Nothing waiting for confirmation.</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {claim.suggestions.map((suggestion) => (
              <SuggestionCard
                key={suggestion.id}
                claim={claim}
                suggestion={suggestion}
                isBusy={tracker.isBusy}
                onConfirm={({ kind: confirmed, occurredAt: date, reason: why, expected: by }) =>
                  void tracker.confirm({
                    claimId: claim.id,
                    suggestionId: suggestion.id,
                    kind: confirmed,
                    occurredAt: date,
                    extensionReason: why,
                    expectedDeterminationAt: by,
                  })
                }
                onReject={() => void tracker.reject(claim.id, suggestion.id)}
              />
            ))}
          </ul>
        )}
      </section>

      <section aria-label="Import emails" className="border-t border-line pt-5">
        <h3 className="text-heading text-fg">Notification emails</h3>
        <p className="mb-3 mt-1 text-caption text-muted">
          Choose .eml or .msg files the portal emailed you. They are read on this device only; attachments and remote content are never opened.
        </p>
        <button type="button" className={button} disabled={tracker.isBusy} onClick={() => void choose()}>Choose email files</button>
        {pickError && <p role="alert" className="text-danger">{pickError}</p>}
        {outcome && (
          <ul className="mt-1 flex flex-col gap-1" aria-label="Import results">
            {outcome.files.map((file, index) => (
              <li key={`${file.fileName}-${index}`}>
                {file.fileName}: {file.outcome === 'IMPORTED' ? `${file.suggestionsAdded} suggested` : file.outcome.split('_').join(' ').toLowerCase()}
                {file.detail && <span className="text-muted"> · {file.detail}</span>}
              </li>
            ))}
          </ul>
        )}
      </section>

      {confirmingRemoval ? (
        <div role="group" aria-label="Confirm stop tracking" className="rounded-lg border border-danger/30 bg-danger/10 p-3 text-danger">
          <p>Stop tracking {claim.reference}? Its recorded dates and imported suggestions are deleted from this computer. The agency's own records are not affected.</p>
          <div className="mt-2 flex gap-2">
            <Button variant="destructive" size="sm" disabled={tracker.isBusy} onClick={() => void tracker.remove(claim.id)} className="focus-ring">
              Yes, delete tracking data
            </Button>
            <button type="button" className={button} onClick={() => setConfirmingRemoval(false)}>Keep tracking</button>
          </div>
        </div>
      ) : (
        <Button variant="danger" size="sm" disabled={tracker.isBusy} onClick={() => setConfirmingRemoval(true)} className="focus-ring self-start">
          Stop tracking this request
        </Button>
      )}
    </div>
  );
}

export default function MyRequestsPanel({ onClose }: { onClose: () => void }) {
  const tracker = useMyRequests(true);
  const portalGate = useLatestRequestGate();
  const [portals, setPortals] = useState<{ key: string; name: string }[]>([]);
  const [portalError, setPortalError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [runwayHighlight, setRunwayHighlight] = useState<string | null>(null);

  useEffect(() => {
    const token = portalGate.begin();
    invokeCommand('list_saved_portals', {})
      .then((saved) => {
        if (!portalGate.isCurrent(token)) return;
        setPortals(saved.map((source) => ({ key: source.registration.portalKey, name: source.registration.displayName })));
      })
      .catch((reason) => {
        if (!portalGate.isCurrent(token)) return;
        setPortalError(displayError(reason).message);
      });
  }, [portalGate]);

  const selected = tracker.claims.find((claim) => claim.id === selectedId) ?? null;

  return (
    <Dialog onClose={onClose} placement="sheet-right">
      <DialogHeader title="My requests" />
      <DialogBody className="text-body text-fg-secondary">
        {tracker.error && tracker.status !== 'failed' && <InlineError className="mt-2" summary={tracker.error} />}
        {portalError && <InlineError className="mt-2" summary={`Saved portals could not be loaded, so none can be linked. ${portalError}`} />}

        {tracker.status === 'loading' ? (
          <Skeleton variant="lines" lines={2} label="Loading your tracked requests" className="mt-3" />
        ) : tracker.status === 'failed' ? (
          <InlineError className="mt-3" summary="Your tracked requests could not be loaded." detail={tracker.error ?? undefined} onRetry={() => void tracker.refresh()} />
        ) : selected ? (
          <div className="mt-3">
            <Button variant="ghost" size="sm" icon="back" onClick={() => setSelectedId(null)} className="focus-ring -ml-2">All requests</Button>
            <div className="mt-3">
              <Detail claim={selected} deadline={tracker.deadlines[selected.id]} tracker={tracker} />
            </div>
          </div>
        ) : (
          <div className="mt-1 flex flex-col gap-6">
            {tracker.claims.length === 0 ? (
              <p className="text-muted">You are not tracking any request yet.</p>
            ) : (
              <>
              <DeadlineRunway
                model={runwayModel(tracker.claims, tracker.deadlines)}
                highlighted={runwayHighlight}
                onHover={setRunwayHighlight}
                onOpen={setSelectedId}
              />
              <ul className="flex flex-col gap-2" aria-label="Tracked requests">
                {tracker.claims.map((claim) => (
                  <li key={claim.id} onPointerEnter={() => setRunwayHighlight(claim.id)} onPointerLeave={() => setRunwayHighlight(null)}>
                    <button type="button" data-highlighted={runwayHighlight === claim.id ? '' : undefined} className={cn('focus-ring flex w-full flex-col items-start gap-1.5 rounded-xl border bg-panel/60 p-4 text-left transition-colors hover:border-line-strong hover:bg-raised', runwayHighlight === claim.id ? 'border-line-strong bg-raised' : 'border-line')} onClick={() => setSelectedId(claim.id)}>
                      <span className="block text-body font-medium text-fg">{claim.title || `Request ${claim.reference}`}</span>
                      {claim.title && <span className="block text-caption tabular-nums text-muted">Request {claim.reference}</span>}
                      {tracker.deadlines[claim.id] && <DeadlineChip deadline={tracker.deadlines[claim.id]} />}
                      {claim.suggestions.length > 0 && (
                        <span className="text-caption text-fg-secondary">{claim.suggestions.length} suggested event{claim.suggestions.length === 1 ? '' : 's'} to review</span>
                      )}
                    </button>
                  </li>
                ))}
              </ul>
              </>
            )}
            <ClaimForm
              portals={portals}
              isBusy={tracker.isBusy}
              onSubmit={(input) => void tracker.claim(input).then((created) => created && setSelectedId(created.id))}
            />
          </div>
        )}
      </DialogBody>
    </Dialog>
  );
}

/** A saved link as its host and path, so it reads as a link rather than a wrapped URL; the full URL is the tooltip. */
function linkLabel(url: string): string {
  try {
    const parsed = new URL(url);
    return `${parsed.host}${parsed.pathname === '/' ? '' : parsed.pathname}`;
  } catch {
    return url;
  }
}
