import { useCallback, useEffect, useState, type KeyboardEvent } from 'react';
import { open as chooseFolder } from '@tauri-apps/plugin-dialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import {
  RESOLUTION_REASON_MAX,
  type DecisionSummary,
  type ResolutionCandidate,
  type ResolutionCandidatePage,
  type ResolutionEntitySnapshot,
  type ResolutionImportPreview,
  type ResolutionLedgerPage,
  type ResolutionVerdict,
  type ResolutionSuggestionBasis,
} from '../../types';
import { SignerTrustNotice } from './CaseSharing';
import { useContributors } from './useContributors';

const AUTHOR_KEY = 'onrecord.resolutionAuthor';
const MAXIMUM_PACKAGE_BYTES = 8 * 1024 * 1024;
const PAGE = 10;

const BASIS_LABEL: Record<ResolutionSuggestionBasis, string> = {
  STRONG_IDENTIFIER: 'shared identifier',
  NAME_ONLY: 'name-only',
  MANUAL: 'manual',
};

const VERDICT_LABEL: Record<ResolutionVerdict, string> = { SAME: 'Same', DIFFERENT: 'Different', UNSURE: 'Unsure' };

function readAuthor(): string {
  try {
    return window.localStorage.getItem(AUTHOR_KEY) ?? '';
  } catch {
    return '';
  }
}

function saveAuthor(author: string) {
  try {
    window.localStorage.setItem(AUTHOR_KEY, author);
  } catch {
    // A remembered name is a convenience.
  }
}

/** Counts of the reviewer's own current judgments. Never a statement about accuracy. */
export function decisionLines(summary: DecisionSummary): string[] {
  return summary.byBasis.map((row) => {
    const decided = row.same + row.different + row.unsure;
    return `You marked ${row.same} of ${decided} ${BASIS_LABEL[row.basis]} suggestion${decided === 1 ? '' : 's'} as the same.`;
  });
}

function Side({ title, side }: { title: string; side: ResolutionEntitySnapshot }) {
  return (
    <section aria-label={title} className="rounded border border-slate-700 p-3 text-xs text-slate-300">
      <p className="text-[10px] uppercase tracking-wide text-slate-400">{title} · {side.key.kind.toLowerCase()}</p>
      <p className="mt-1 text-sm font-semibold text-slate-100">{side.displayValue}</p>
      <p className="mt-1">Appears in {side.documentCount} document{side.documentCount === 1 ? '' : 's'}.</p>
      <p className="mt-2 text-slate-400">Showing {side.examples.length} of {side.documentCount}:</p>
      <ul className="mt-1 space-y-0.5">
        {side.examples.map((example) => (
          <li key={`${example.portalKey}:${example.documentId}:${example.characterStart}`}>
            {example.portalKey} · document {example.documentId} · characters {example.characterStart}-{example.characterEnd}
          </li>
        ))}
      </ul>
    </section>
  );
}

function CandidateCard({
  candidate,
  author,
  busy,
  onDecide,
}: {
  candidate: ResolutionCandidate;
  author: string;
  busy: boolean;
  onDecide: (candidate: ResolutionCandidate, verdict: ResolutionVerdict, reason: string) => void;
}) {
  const [reason, setReason] = useState('');
  const { evidence } = candidate;
  const disabled = busy || !author.trim();
  const decide = (verdict: ResolutionVerdict) => {
    if (!disabled) onDecide(candidate, verdict, reason);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const target = event.target as HTMLElement;
    if (target.tagName === 'TEXTAREA' || target.tagName === 'INPUT' || event.ctrlKey || event.metaKey || event.altKey) return;
    const verdict = ({ s: 'SAME', d: 'DIFFERENT', u: 'UNSURE' } as const)[event.key.toLowerCase() as 's' | 'd' | 'u'];
    if (verdict) {
      event.preventDefault();
      decide(verdict);
    }
  };
  return (
    <article aria-label="Candidate pair" tabIndex={0} onKeyDown={onKeyDown} className="space-y-3 rounded border border-cyan-500/30 p-3 outline-none focus:ring-1 focus:ring-cyan-400">
      <p className="text-xs text-slate-300">
        {evidence.basis === 'STRONG_IDENTIFIER' && evidence.sharedIdentifier
          ? `Both appear in the same text segment as ${evidence.sharedIdentifier.displayValue} (${evidence.sharedIdentifier.leftDocuments} and ${evidence.sharedIdentifier.rightDocuments} documents). Co-occurrence is not ownership; read the documents.`
          : 'The names match once punctuation, word order and company suffixes are ignored. A similar name is a lead, not proof.'}
      </p>
      <div className="grid gap-3 sm:grid-cols-2">
        <Side title="Entity A" side={evidence.left} />
        <Side title="Entity B" side={evidence.right} />
      </div>
      <label className="block text-xs text-slate-300">
        Reason (optional, kept in the ledger)
        <textarea className="analysis-input mt-1 w-full" rows={2} maxLength={RESOLUTION_REASON_MAX} value={reason} onChange={(event) => setReason(event.target.value)} />
      </label>
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className="analysis-button" disabled={disabled} aria-keyshortcuts="s" onClick={() => decide('SAME')}>Same (S)</button>
        <button type="button" className="analysis-button" disabled={disabled} aria-keyshortcuts="d" onClick={() => decide('DIFFERENT')}>Different (D)</button>
        <button type="button" className="analysis-button" disabled={disabled} aria-keyshortcuts="u" onClick={() => decide('UNSURE')}>Unsure (U)</button>
        {!author.trim() && <span className="text-xs text-amber-200">Enter your name above to record a judgment.</span>}
      </div>
    </article>
  );
}

function SharePanel({ onChanged }: { onChanged: () => void }) {
  const contributors = useContributors();
  const gate = useLatestRequestGate();
  const [payload, setPayload] = useState('');
  const [preview, setPreview] = useState<ResolutionImportPreview | null>(null);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  const run = async <T,>(task: () => Promise<T>, accept: (value: T) => void) => {
    const request = gate.begin();
    setBusy(true);
    setError('');
    try {
      const value = await task();
      if (gate.isCurrent(request)) accept(value);
    } catch (reason) {
      if (gate.isCurrent(request)) setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(request)) setBusy(false);
    }
  };

  const exportLedger = () =>
    run(
      async () => {
        const folder = await chooseFolder({ directory: true, multiple: false });
        if (typeof folder !== 'string') return null;
        const envelope = await invokeCommand('export_resolution_ledger', {});
        const fileName = `resolution-ledger-${new Date().toISOString().slice(0, 10)}-${Date.now()}.json`;
        await invokeCommand('publish_packet_to_folder', { folder, fileName, payload: envelope });
        return fileName;
      },
      (fileName) => setMessage(fileName ? `Signed ledger written as ${fileName}.` : ''),
    );

  const chooseFile = (file: File | undefined) => {
    gate.invalidate();
    setBusy(false);
    setPreview(null);
    setMessage('');
    if (!file) return;
    if (file.size > MAXIMUM_PACKAGE_BYTES) {
      setError('Ledger files must be at most 8 MB.');
      return;
    }
    void run(
      async () => {
        const text = await file.text();
        return { text, preview: await invokeCommand('preview_resolution_import', { payload: text }) };
      },
      ({ text, preview: next }) => {
        setPayload(text);
        setPreview(next);
      },
    );
  };

  return (
    <div className="space-y-3 text-xs text-slate-300">
      <div className="flex flex-wrap items-center gap-2">
        <button type="button" className="analysis-button" disabled={busy} onClick={() => void exportLedger()}>Export signed ledger</button>
        <label className="flex items-center gap-2">
          Import a ledger file
          <input type="file" accept=".json,application/json" className="text-xs" onChange={(event) => chooseFile(event.target.files?.[0])} />
        </label>
      </div>
      {preview && (
        <div className="space-y-2 rounded border border-slate-700 p-3">
          <p role="status" className="font-semibold text-cyan-200">
            {preview.additions} new entr{preview.additions === 1 ? 'y' : 'ies'}, {preview.alreadyPresent} already here, {preview.conflicts.length} conflict{preview.conflicts.length === 1 ? '' : 's'}.
          </p>
          <SignerTrustNotice signerKeyId={preview.signerKeyId} contributors={contributors} />
          {preview.contradictionsAfter > preview.contradictionsBefore && (
            <p className="text-amber-200">Importing would raise contradictions from {preview.contradictionsBefore} to {preview.contradictionsAfter}. They stay unmerged until you resolve them.</p>
          )}
          {preview.conflicts.map((conflict) => (
            <p key={conflict.entryId} className="text-amber-200">Entry {conflict.entryId}: {conflict.reason}</p>
          ))}
          <button
            type="button"
            className="analysis-button"
            disabled={busy || preview.additions === 0}
            onClick={() =>
              void run(
                () => invokeCommand('import_resolution_ledger', { payload, packetSha256: preview.packetSha256 }),
                (result) => {
                  setMessage(`Added ${result.added} entries. Nothing already here was replaced.`);
                  setPreview(null);
                  setPayload('');
                  onChanged();
                },
              )
            }
          >
            Add these entries
          </button>
        </div>
      )}
      {message && <p role="status" className="text-emerald-300">{message}</p>}
      {error && <p role="alert" className="text-rose-300">{error}</p>}
    </div>
  );
}

function LedgerView({ page, onHistory, historyOf }: { page: ResolutionLedgerPage; onHistory: (pair: string | null) => void; historyOf: string | null }) {
  return (
    <div className="space-y-2 text-xs text-slate-300">
      {page.contradictions.map((contradiction) => (
        <p key={contradiction.sameEntryIds.join(',')} role="alert" className="rounded border border-amber-500/40 p-2 text-amber-200">
          Contradiction: {contradiction.keys.map((key) => key.value).join(', ')} are linked by SAME judgments and also separated by a DIFFERENT one. They are left unmerged. Record a new judgment to resolve it.
        </p>
      ))}
      {historyOf && (
        <p>
          Showing the history of one pair.{' '}
          <button type="button" className="underline" onClick={() => onHistory(null)}>Show the whole ledger</button>
        </p>
      )}
      <ol className="space-y-2">
        {page.items.map((item) => {
          const pair = `${item.entry.left.kind}|${item.entry.left.value}||${item.entry.right.kind}|${item.entry.right.value}`;
          return (
            <li key={item.entry.id} className={`rounded border p-2 ${item.isLatest ? 'border-slate-600' : 'border-slate-800 text-slate-400'}`}>
              <p>
                <strong>{VERDICT_LABEL[item.entry.verdict]}</strong>: {item.entry.evidence.left.displayValue} and {item.entry.evidence.right.displayValue}
                {!item.isLatest && ' (superseded)'}
              </p>
              <p>{item.entry.author} · {item.entry.decidedAt}{item.importedFrom && ` · imported, signer ${item.importedFrom.slice(0, 12)}`} · {BASIS_LABEL[item.entry.evidence.basis]}</p>
              {item.entry.reason && <p>Reason: {item.entry.reason}</p>}
              {historyOf !== pair && <button type="button" className="underline" onClick={() => onHistory(pair)}>History of this pair</button>}
            </li>
          );
        })}
      </ol>
      <p>{page.items.length} of {page.total} shown. The ledger holds {page.entryCount} entries and is never edited.</p>
    </div>
  );
}

export default function ResolutionReview() {
  const queueGate = useLatestRequestGate();
  const ledgerGate = useLatestRequestGate();
  const [author, setAuthor] = useState(readAuthor);
  const [includeUnsure, setIncludeUnsure] = useState(false);
  const [queue, setQueue] = useState<ResolutionCandidatePage | null>(null);
  const [ledger, setLedger] = useState<ResolutionLedgerPage | null>(null);
  const [historyOf, setHistoryOf] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [lastDecision, setLastDecision] = useState<{ candidate: ResolutionCandidate; entryId: string } | null>(null);
  const [view, setView] = useState<'queue' | 'ledger' | 'share'>('queue');

  const loadQueue = useCallback(
    (cursor: string | null) => {
      const request = queueGate.begin();
      setBusy(true);
      invokeCommand('list_resolution_candidates', { cursor, limit: PAGE, includeUnsure })
        .then((page) => {
          if (!queueGate.isCurrent(request)) return;
          setQueue((held) => (cursor && held ? { ...page, candidates: [...held.candidates, ...page.candidates], remaining: held.candidates.length + page.remaining } : page));
          setError('');
        })
        .catch((reason) => {
          if (queueGate.isCurrent(request)) setError(formatNativeError(reason));
        })
        .finally(() => {
          if (queueGate.isCurrent(request)) setBusy(false);
        });
    },
    [queueGate, includeUnsure],
  );

  const loadLedger = useCallback(
    (pair: string | null) => {
      const request = ledgerGate.begin();
      invokeCommand('list_resolution_ledger', { cursor: null, limit: 50, pair })
        .then((page) => {
          if (ledgerGate.isCurrent(request)) setLedger(page);
        })
        .catch((reason) => {
          if (ledgerGate.isCurrent(request)) setError(formatNativeError(reason));
        });
    },
    [ledgerGate],
  );

  useEffect(() => {
    setQueue(null);
    loadQueue(null);
  }, [loadQueue]);

  useEffect(() => {
    loadLedger(historyOf);
  }, [loadLedger, historyOf]);

  const record = useCallback(
    async (candidate: ResolutionCandidate, verdict: ResolutionVerdict, reason: string, supersedes: string | null) => {
      setBusy(true);
      setError('');
      try {
        const item = await invokeCommand('record_resolution_judgment', {
          input: { leftEntityId: candidate.leftEntityId, rightEntityId: candidate.rightEntityId, verdict, author: author.trim(), reason, supersedes },
        });
        // Removed by pair, not by position: the queue may have been reloaded while this was in flight.
        setQueue((held) => (held ? { ...held, candidates: held.candidates.filter((entry) => entry.pairId !== candidate.pairId), remaining: Math.max(0, held.remaining - 1) } : held));
        setLastDecision({ candidate, entryId: item.entry.id });
        loadLedger(historyOf);
      } catch (reason_) {
        setError(formatNativeError(reason_));
      } finally {
        setBusy(false);
      }
    },
    [author, historyOf, loadLedger],
  );

  const undo = async () => {
    if (!lastDecision) return;
    const { candidate, entryId } = lastDecision;
    await record(candidate, 'UNSURE', 'Undone', entryId);
    setQueue((held) => (held ? { ...held, candidates: [candidate, ...held.candidates], remaining: held.remaining + 1 } : held));
    setLastDecision(null);
  };

  const current = queue?.candidates[0] ?? null;

  return (
    <section aria-label="Entity resolution" className="app-panel space-y-3 p-4">
      <header className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-slate-100">Resolve entities</h3>
          <p className="text-xs text-slate-400">Your judgments are kept in a ledger on this device, survive an index rebuild, and are never edited or deleted. Nothing is merged without you.</p>
        </div>
        <label className="text-xs text-slate-300">
          Your name
          <input className="analysis-input ml-2" value={author} maxLength={120} onChange={(event) => { setAuthor(event.target.value); saveAuthor(event.target.value); }} />
        </label>
      </header>
      <div className="flex flex-wrap gap-2" role="tablist" aria-label="Resolution views">
        {(['queue', 'ledger', 'share'] as const).map((name) => (
          <button key={name} type="button" role="tab" aria-selected={view === name} className="analysis-button" onClick={() => setView(name)}>
            {{ queue: 'Review queue', ledger: 'Ledger history', share: 'Import and export' }[name]}
          </button>
        ))}
      </div>
      {error && <p role="alert" className="text-xs text-rose-300">{error}</p>}
      {ledger && (
        <ul className="text-xs text-slate-400" aria-label="Your decisions so far">
          {decisionLines(ledger.summary).map((line) => <li key={line}>{line}</li>)}
          {ledger.summary.byBasis.length > 0 && <li>These are counts of your own decisions, not a measure of how accurate anyone was.</li>}
        </ul>
      )}
      {view === 'queue' && (
        <div className="space-y-3">
          <label className="flex items-center gap-2 text-xs text-slate-300">
            <input type="checkbox" checked={includeUnsure} onChange={(event) => setIncludeUnsure(event.target.checked)} />
            Include pairs I marked unsure
          </label>
          {queue === null && !error && <p role="status" className="text-xs text-slate-400">Looking for candidate pairs…</p>}
          {queue?.scanTruncated && <p className="text-xs text-amber-200">The index has more names than one scan reads, so some pairs were not considered.</p>}
          {queue && (queue.guards.skippedNameGroups > 0 || queue.guards.skippedIdentifiers > 0) && (
            <p className="text-xs text-amber-200">Skipped {queue.guards.skippedNameGroups} very common names and {queue.guards.skippedIdentifiers} widely shared identifiers; pairing them would bury real leads.</p>
          )}
          {current ? (
            <CandidateCard key={current.pairId} candidate={current} author={author} busy={busy} onDecide={(candidate, verdict, reason) => void record(candidate, verdict, reason, null)} />
          ) : (
            queue && <p role="status" className="text-xs text-slate-300">No candidate pairs are waiting.</p>
          )}
          <div className="flex flex-wrap items-center gap-2 text-xs text-slate-400">
            {queue && <span>{queue.remaining} waiting</span>}
            {lastDecision && <button type="button" className="analysis-button" disabled={busy} onClick={() => void undo()}>Undo last judgment</button>}
            {queue?.nextCursor && queue.candidates.length <= 1 && <button type="button" className="analysis-button" disabled={busy} onClick={() => loadQueue(queue.nextCursor)}>Load more</button>}
          </div>
        </div>
      )}
      {view === 'ledger' && (ledger ? <LedgerView page={ledger} onHistory={setHistoryOf} historyOf={historyOf} /> : <p role="status" className="text-xs text-slate-400">Reading the ledger…</p>)}
      {view === 'share' && <SharePanel onChanged={() => { loadLedger(historyOf); loadQueue(null); }} />}
    </section>
  );
}
