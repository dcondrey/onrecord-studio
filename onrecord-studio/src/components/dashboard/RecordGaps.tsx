import { useCallback, useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { guidanceForSubject } from '../../utils/sourceGuidance';
import { gapRequestInput, recordGapState, recordGapStateLabel, type RecordGap } from '../../utils/recordGaps';
import type { RecordGapRequestStatus, RequestSearchResult } from '../../types';

const SEARCHED_PER_TERM = 5;

/** The records that would answer what the leads' own tables cannot: whether a held request already
 * released them, the request to file when none did, and where a filed request stands. */
export default function RecordGaps({ gaps }: { gaps: RecordGap[] }) {
  const portals = useAppStore((state) => state.portals);
  const [tracked, setTracked] = useState<RecordGapRequestStatus[]>([]);
  const [error, setError] = useState('');
  const gate = useLatestRequestGate();
  const keys = useMemo(() => gaps.map((gap) => gap.key), [gaps]);
  const signature = JSON.stringify(keys);
  const load = useCallback(() => {
    const request = gate.begin();
    invokeCommand('get_record_gap_requests', { gapKeys: JSON.parse(signature) as string[] }).then((statuses) => {
      if (!gate.isCurrent(request)) return;
      setTracked(statuses); setError('');
    }, (reason) => { if (gate.isCurrent(request)) setError(String(reason)); });
  }, [gate, signature]);
  useEffect(() => { load(); return gate.invalidate; }, [gate, load]);
  // The source's own name when it is a saved source; otherwise its host. Never a guess at an agency.
  const custodian = (gap: RecordGap) => {
    if (!gap.sourceOrigin) return '';
    const host = new URL(gap.sourceOrigin).host;
    const saved = portals.find((portal) => [portal.registration.originUrl, portal.registration.canonicalUrl].some((url) => { try { return new URL(url).host === host; } catch { return false; } }));
    return saved?.registration.displayName ?? host;
  };
  if (!gaps.length) return null;
  const open = gaps.filter((gap) => !gap.answered);
  return <section aria-label="Record gaps" className="space-y-2">
    <h3 className="text-sm font-semibold text-fg">Records that would answer these leads</h3>
    <p className="text-label leading-4 text-muted">Each lead leaves a question its tables cannot answer. Search what you already hold first; a request is drafted only for you to review and file yourself.{gaps.length > open.length ? ` ${gaps.length - open.length} answered by an imported record.` : ''}</p>
    {error && <p role="alert" className="text-label leading-4 text-warning">Tracked requests could not be read: {error}</p>}
    {open.map((gap) => <RecordGapRow key={gap.key} gap={gap} custodian={custodian(gap)} tracked={tracked.find((status) => status.gapKey === gap.key)} onTracked={load} />)}
  </section>;
}

function RecordGapRow({ gap, custodian, tracked, onTracked }: { gap: RecordGap; custodian: string; tracked: RecordGapRequestStatus | undefined; onTracked: () => void }) {
  const [matches, setMatches] = useState<RequestSearchResult[] | undefined>(undefined);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const gate = useLatestRequestGate();
  const input = gapRequestInput(gap, custodian);
  const run = (work: () => Promise<string>) => {
    const request = gate.begin();
    setBusy(true); setError(''); setMessage('');
    work().then((done) => { if (gate.isCurrent(request)) setMessage(done); }, (reason) => { if (gate.isCurrent(request)) setError(String(reason)); })
      .finally(() => { if (gate.isCurrent(request)) setBusy(false); });
    return request;
  };
  const searchHeld = () => {
    const request = run(async () => {
      const found = new Map<string, RequestSearchResult>();
      for (const term of gap.definition.searchTerms) {
        const page = await invokeCommand('query_audit_records_page', { searchTerm: term, pageSize: SEARCHED_PER_TERM });
        for (const record of page.records) found.set(`${record.portalKey}:${record.requestId}`, record);
      }
      if (gate.isCurrent(request)) setMatches([...found.values()]);
      return '';
    });
  };
  const searchPortal = () => run(async () => {
    await invokeCommand('start_data_source', { url: gap.sourceOrigin!, guidance: guidanceForSubject(gap.definition.searchTerms.join(', ')) });
    return 'Collection started; the source is listed in the sidebar. Search held requests again when it finishes.';
  });
  const track = () => run(async () => {
    await invokeCommand('request_record_gap', { input });
    onTracked();
    return 'Tracked. Record the submission date in casework once you have filed it.';
  });
  const state = recordGapState(gap, tracked, matches, new Date().toISOString());
  return <article className="rounded-xl border border-line p-3 text-label leading-4 text-fg-secondary">
    <h4 className="font-semibold text-fg">{gap.definition.record}</h4>
    <p className="mt-1">{gap.definition.missing} For: {gap.leadTitle}</p>
    <p role="status" className="mt-1 text-fg">{recordGapStateLabel(state)}</p>
    {matches && (matches.length === 0 ? <p className="mt-1 text-muted">No held request mentions {gap.definition.searchTerms.join(', ')}.</p> : <ul className="mt-1 space-y-1">{matches.map((match) => <li key={`${match.portalKey}:${match.requestId}`}>
      {match.title || `Request ${match.requestId}`} · {match.agencyName} · {match.status} · {match.documentCount === 0 ? 'no released files held' : `${match.documentCount} released ${match.documentCount === 1 ? 'file' : 'files'} held`}</li>)}</ul>)}
    {error && <p role="alert" className="mt-1 text-warning">{error}</p>}
    {message && <p className="mt-1 text-muted">{message}</p>}
    <div className="mt-2 flex flex-wrap gap-2">
      <button type="button" className="analysis-button" disabled={busy} onClick={searchHeld}>Search held requests</button>
      {gap.sourceOrigin && <button type="button" className="analysis-button" disabled={busy} onClick={searchPortal}>Collect from {new URL(gap.sourceOrigin).host}</button>}
      {tracked ? <button type="button" className="analysis-button" onClick={() => useAppStore.getState().openCasework(tracked.reportId)}>Open casework</button>
        : <button type="button" className="analysis-button" disabled={busy} onClick={track}>Track this request</button>}
    </div>
    {!tracked && <details className="mt-2"><summary>Drafted request</summary><pre className="mt-2 whitespace-pre-wrap break-words text-label leading-4 text-muted">{input.draft}</pre></details>}
  </article>;
}
