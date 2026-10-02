import { useEffect, useState, type FormEvent } from 'react';
import { open as chooseFiles } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useDialogFocus } from '../licensing/useDialogFocus';
import { CALIFORNIA_CPRA_JURISDICTION, TRACKER_LIMITS } from '../../types';
import type {
  ClaimedRequest,
  RequestDeadline,
  StatutoryExtensionReason,
  SuggestedEvent,
  TimelineEventKind,
} from '../../types';
import { DeadlineChip } from './DeadlineChip';
import { localInputToRfc3339, rfc3339ToLocalInput } from './rfc3339';
import { useMyRequests } from './useMyRequests';

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

const field = 'focus-ring w-full rounded border border-line-strong bg-canvas px-2 py-1 text-fg';
const button = 'focus-ring rounded border border-line-strong px-2 py-1 font-semibold text-fg hover:border-accent/60 disabled:opacity-50';

function KindSelect({ value, onChange, label }: { value: TimelineEventKind; onChange: (kind: TimelineEventKind) => void; label: string }) {
  return (
    <select aria-label={label} className={field} value={value} onChange={(event) => onChange(event.target.value as TimelineEventKind)}>
      {(Object.keys(KIND_LABELS) as TimelineEventKind[]).map((kind) => (
        <option key={kind} value={kind}>{KIND_LABELS[kind]}</option>
      ))}
    </select>
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
      <label className="block">
        Statutory reason for the extension (required)
        <select className={field} value={reason} onChange={(event) => onReason(event.target.value as StatutoryExtensionReason | '')}>
          <option value="">Choose the reason the notice gives</option>
          {(Object.keys(REASON_LABELS) as StatutoryExtensionReason[]).map((value) => (
            <option key={value} value={value}>{REASON_LABELS[value]}</option>
          ))}
        </select>
      </label>
      <label className="block">
        Date the notice says the agency will respond by (optional; without it the extension is not applied)
        <input type="datetime-local" className={field} value={expected} onChange={(event) => onExpected(event.target.value)} />
      </label>
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
    <form onSubmit={submit} className="flex flex-col gap-1.5" aria-label="Claim a request">
      <label>
        Request number, as the portal shows it
        <input required maxLength={TRACKER_LIMITS.referenceCharacters} className={field} value={reference} onChange={(event) => setReference(event.target.value)} />
      </label>
      <label>
        Title (optional)
        <input maxLength={TRACKER_LIMITS.titleCharacters} className={field} value={title} onChange={(event) => setTitle(event.target.value)} />
      </label>
      <label>
        Link to the request (optional, https only)
        <input type="url" maxLength={TRACKER_LIMITS.sourceUrlCharacters} className={field} value={sourceUrl} onChange={(event) => setSourceUrl(event.target.value)} />
      </label>
      <label>
        Portal (optional)
        <select className={field} value={portalKey} onChange={(event) => setPortalKey(event.target.value)}>
          <option value="">Not linked to a registered portal</option>
          {portals.map((portal) => (
            <option key={portal.key} value={portal.key}>{portal.name}</option>
          ))}
        </select>
      </label>
      <label>
        Jurisdiction
        <select className={field} value={jurisdiction} onChange={(event) => setJurisdiction(event.target.value)}>
          <option value="">Not set</option>
          <option value={CALIFORNIA_CPRA_JURISDICTION}>California (Public Records Act)</option>
          <option value="OTHER">Other (no deadline model)</option>
        </select>
      </label>
      {jurisdiction === 'OTHER' && (
        <input aria-label="Jurisdiction name" maxLength={TRACKER_LIMITS.jurisdictionCharacters} className={field} value={otherJurisdiction} onChange={(event) => setOtherJurisdiction(event.target.value)} />
      )}
      <p className="text-label text-muted">
        Nothing here is sent anywhere, and no portal login is used or stored. Only the dates you enter or confirm from emails count.
      </p>
      <button type="submit" disabled={isBusy || reference.trim() === ''} className={button}>Track this request</button>
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
    <li className="rounded border border-dashed border-line-strong p-2" aria-label={`Suggested ${KIND_LABELS[suggestion.kind]} for ${claim.reference}`}>
      <p className="text-label font-semibold text-fg">Suggested, not counted until you confirm</p>
      <blockquote className="mt-1 border-l-2 border-line-strong pl-1.5 italic">{suggestion.evidence.matchedSentence}</blockquote>
      <p className="text-label text-muted">
        From an email{suggestion.evidence.subjectExcerpt ? ` titled "${suggestion.evidence.subjectExcerpt}"` : ''}.
      </p>
      <div className="mt-1 grid gap-1.5">
        <KindSelect label="What this describes" value={kind} onChange={setKind} />
        <label>
          {suggestion.candidateOccurredAt
            ? 'Date (filled from the email date, which is not necessarily the date of the event)'
            : 'Date (the email had no readable date; enter the date of the event)'}
          <input type="datetime-local" className={field} value={when} onChange={(event) => setWhen(event.target.value)} />
        </label>
        {needsReason && <ExtensionFields reason={reason} expected={expected} onReason={setReason} onExpected={setExpected} />}
      </div>
      <div className="mt-1.5 flex gap-2">
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

  return (
    <div className="flex flex-col gap-3" aria-label={`Request ${claim.reference}`}>
      <section aria-label="Deadline" className="rounded border border-line-strong p-2">
        <div className="flex flex-wrap items-center gap-2">
          <h3 className="text-label font-semibold text-fg">{claim.reference}</h3>
          {deadline && <DeadlineChip deadline={deadline} />}
        </div>
        {claim.title && <p>{claim.title}</p>}
        {deadline ? (
          <>
            <p className="mt-1" data-testid="basis-statement">{deadline.basisStatement}</p>
            {deadline.notes.map((note) => (
              <p key={note} className="text-label text-muted">{note}</p>
            ))}
          </>
        ) : (
          <p className="mt-1 text-muted">Loading the deadline.</p>
        )}
        {claim.sourceUrl && (
          <p className="mt-1 break-all text-label text-muted">Link you saved: {claim.sourceUrl}</p>
        )}
      </section>

      <section aria-label="Timeline">
        <h3 className="text-label font-semibold text-fg">Timeline</h3>
        {events.length === 0 ? (
          <p className="text-muted">No dates recorded yet.</p>
        ) : (
          <ol className="flex flex-col gap-1">
            {events.map((event) => (
              <li key={event.id} className="flex items-start justify-between gap-2 rounded border border-line p-1.5">
                <span>
                  <strong>{KIND_LABELS[event.kind]}</strong> on {event.occurredAt.slice(0, 10)} ·{' '}
                  {event.basis === 'EMAIL_CONFIRMED' ? 'confirmed from an email' : 'entered by you'}
                  {event.extensionReason && ` · ${REASON_LABELS[event.extensionReason]}`}
                  {event.evidence && <blockquote className="border-l-2 border-line-strong pl-1.5 italic text-fg-secondary">{event.evidence.matchedSentence}</blockquote>}
                </span>
                <button type="button" disabled={tracker.isBusy} className={button} aria-label={`Remove ${KIND_LABELS[event.kind]} on ${event.occurredAt.slice(0, 10)}`} onClick={() => void tracker.removeEvent(claim.id, event.id)}>
                  Remove
                </button>
              </li>
            ))}
          </ol>
        )}
        {claim.events.length > 0 && (
          <div className="mt-1.5 flex items-center gap-2">
            <button type="button" className={button} onClick={() => void copyChronology()}>Copy chronology</button>
            {copyStatus && <span role="status" className="text-success">{copyStatus}</span>}
          </div>
        )}
        <form
          className="mt-2 grid gap-1.5"
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
          <label>
            Date and time
            <input type="datetime-local" className={field} value={when} onChange={(event) => setWhen(event.target.value)} />
          </label>
          {needsReason && <ExtensionFields reason={reason} expected={expected} onReason={setReason} onExpected={setExpected} />}
          <button type="submit" className={button} disabled={tracker.isBusy || !occurredAt || (needsReason && reason === '')}>Add this date</button>
        </form>
      </section>

      <section aria-label="Suggested events">
        <h3 className="text-label font-semibold text-fg">Suggested from emails</h3>
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

      <section aria-label="Import emails">
        <h3 className="text-label font-semibold text-fg">Notification emails</h3>
        <p className="text-label text-muted">
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
        <div role="alert" className="rounded border border-danger/30 p-2">
          <p>Stop tracking {claim.reference}? Its recorded dates and imported suggestions are deleted from this computer. The agency's own records are not affected.</p>
          <div className="mt-1.5 flex gap-2">
            <button type="button" className={button} disabled={tracker.isBusy} onClick={() => void tracker.remove(claim.id)}>
              Yes, delete tracking data
            </button>
            <button type="button" className={button} onClick={() => setConfirmingRemoval(false)}>Keep tracking</button>
          </div>
        </div>
      ) : (
        <button type="button" className={button} disabled={tracker.isBusy} onClick={() => setConfirmingRemoval(true)}>
          Stop tracking this request
        </button>
      )}
    </div>
  );
}

export default function MyRequestsPanel({ onClose }: { onClose: () => void }) {
  const dialogFocus = useDialogFocus(onClose);
  const tracker = useMyRequests(true);
  const portalGate = useLatestRequestGate();
  const [portals, setPortals] = useState<{ key: string; name: string }[]>([]);
  const [portalError, setPortalError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    const token = portalGate.begin();
    invokeCommand('list_saved_portals', {})
      .then((saved) => {
        if (!portalGate.isCurrent(token)) return;
        setPortals(saved.map((source) => ({ key: source.registration.portalKey, name: source.registration.displayName })));
      })
      .catch((reason) => {
        if (!portalGate.isCurrent(token)) return;
        setPortalError(String(reason instanceof Error ? reason.message : reason));
      });
  }, [portalGate]);

  const selected = tracker.claims.find((claim) => claim.id === selectedId) ?? null;

  return (
    <div role="dialog" aria-modal="true" aria-label="My requests" className="fixed inset-0 z-50 flex justify-end bg-black/60" onKeyDown={dialogFocus.onKeyDown} onMouseDown={dialogFocus.onOverlayMouseDown}>
      <div ref={dialogFocus.ref} tabIndex={-1} className="app-panel h-full w-full max-w-xl overflow-y-auto p-4 text-label text-fg-secondary outline-none">
        <div className="flex items-center justify-between">
          <h2 className="text-label font-semibold text-fg">My requests</h2>
          <button type="button" onClick={onClose} className={button}>Close</button>
        </div>
        {tracker.error && <p role="alert" className="mt-2 text-danger">{tracker.error}</p>}
        {portalError && <p role="alert" className="mt-2 text-danger">Couldn't load saved portals: {portalError}</p>}

        {selected ? (
          <div className="mt-3">
            <button type="button" className={button} onClick={() => setSelectedId(null)}>Back to the list</button>
            <div className="mt-2">
              <Detail claim={selected} deadline={tracker.deadlines[selected.id]} tracker={tracker} />
            </div>
          </div>
        ) : (
          <div className="mt-3 flex flex-col gap-4">
            {tracker.claims.length === 0 ? (
              <p className="text-muted">You are not tracking any request yet.</p>
            ) : (
              <ul className="flex flex-col gap-1.5" aria-label="Tracked requests">
                {tracker.claims.map((claim) => (
                  <li key={claim.id}>
                    <button type="button" className="focus-ring w-full rounded border border-control p-2 text-left hover:border-accent/60" onClick={() => setSelectedId(claim.id)}>
                      <span className="block font-semibold text-fg">{claim.reference}{claim.title ? `: ${claim.title}` : ''}</span>
                      {tracker.deadlines[claim.id] && <DeadlineChip deadline={tracker.deadlines[claim.id]} />}
                      {claim.suggestions.length > 0 && (
                        <span className="ml-2 text-label text-fg-secondary">{claim.suggestions.length} suggested event{claim.suggestions.length === 1 ? '' : 's'} to review</span>
                      )}
                    </button>
                  </li>
                ))}
              </ul>
            )}
            <ClaimForm
              portals={portals}
              isBusy={tracker.isBusy}
              onSubmit={(input) => void tracker.claim(input).then((created) => created && setSelectedId(created.id))}
            />
          </div>
        )}
      </div>
    </div>
  );
}
