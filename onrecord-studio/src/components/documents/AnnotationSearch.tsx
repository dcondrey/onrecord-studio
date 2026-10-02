import { useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ANNOTATION_LIMITS } from '../../types';
import type { AnnotationSearchPage, AnnotationSearchQuery, AnnotationSearchRow, AnnotationStatus } from '../../types';
import { KIND_LABELS, STATUS_LABELS } from './annotationLogic';
import { annotationButton as button, annotationField as field } from './annotationStyles';

export interface EvidenceLink {
  portalKey: string;
  requestId: number;
  documentId: number;
  characterOffset: number;
}

const BLANK: AnnotationSearchQuery = {
  code: null,
  author: null,
  status: null,
  text: null,
  orphanedOnly: false,
  cursor: null,
  limit: ANNOTATION_LIMITS.searchPage,
};

// Kept as typed so a space inside a name or phrase survives; trimmed when the search is sent.
const blankToNull = (value: string) => (value === '' ? null : value);
const trimmedOrNull = (value: string | null) => (value === null || value.trim() === '' ? null : value.trim());

/** Annotations across every document on this device, with the same placement checks as the document view. */
export default function AnnotationSearch({ onOpen }: { onOpen: (link: EvidenceLink) => void }) {
  const gate = useLatestRequestGate();
  const [draft, setDraft] = useState<AnnotationSearchQuery>(BLANK);
  const [applied, setApplied] = useState<AnnotationSearchQuery | null>(null);
  const [rows, setRows] = useState<AnnotationSearchRow[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchPage = async (query: AnnotationSearchQuery, append: boolean) => {
    const request = gate.begin();
    setLoading(true);
    setError(null);
    try {
      const page: AnnotationSearchPage = await invokeCommand('search_document_annotations', { query });
      // A page for an earlier search, or a "more" for a filter that has since changed, must not
      // be added to the list the reviewer is now looking at.
      if (!gate.isCurrent(request)) return;
      setApplied({ ...query, cursor: null });
      setRows((current) => (append ? [...current, ...page.rows] : page.rows));
      setNextCursor(page.nextCursor);
    } catch (reason) {
      if (gate.isCurrent(request)) setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(request)) setLoading(false);
    }
  };

  const search = () => void fetchPage({
    ...draft,
    code: trimmedOrNull(draft.code),
    author: trimmedOrNull(draft.author),
    text: trimmedOrNull(draft.text),
    cursor: null,
  }, false);
  const more = () => {
    if (applied && nextCursor) void fetchPage({ ...applied, cursor: nextCursor }, true);
  };

  const open = (row: AnnotationSearchRow) => {
    if (row.requestId === null) return;
    const located = row.anchor && row.anchor.state !== 'ORPHANED' ? row.anchor.sourceOffset : null;
    onOpen({ portalKey: row.portalKey, requestId: row.requestId, documentId: row.documentId, characterOffset: located ?? 0 });
  };

  return (
    <details className="mx-4 my-2 rounded border border-slate-800 p-3 text-label text-slate-300 lg:mx-6">
      <summary className="cursor-pointer text-label font-semibold text-slate-200">Search annotations across documents</summary>
      <div className="mt-2 space-y-2">
        <p className="text-slate-400">Lists the notes, highlights, and issue codes saved on this computer, across all documents. Each is checked against its document's current text. Positions refer to extracted text, not PDF pages.</p>
        <form className="grid gap-1.5 sm:grid-cols-4" onSubmit={(event) => { event.preventDefault(); search(); }}>
          <label>Code<input className={field} value={draft.code ?? ''} onChange={(event) => setDraft({ ...draft, code: blankToNull(event.target.value) })} /></label>
          <label>Author<input className={field} value={draft.author ?? ''} onChange={(event) => setDraft({ ...draft, author: blankToNull(event.target.value) })} /></label>
          <label>Status
            <select className={field} value={draft.status ?? ''} onChange={(event) => setDraft({ ...draft, status: event.target.value === '' ? null : event.target.value as AnnotationStatus })}>
              <option value="">Any status</option>
              {(Object.keys(STATUS_LABELS) as AnnotationStatus[]).map((value) => <option key={value} value={value}>{STATUS_LABELS[value]}</option>)}
            </select>
          </label>
          <label>Text in quote or note<input className={field} value={draft.text ?? ''} onChange={(event) => setDraft({ ...draft, text: blankToNull(event.target.value) })} /></label>
          <label className="flex items-center gap-1 sm:col-span-2">
            <input type="checkbox" checked={draft.orphanedOnly} onChange={(event) => setDraft({ ...draft, orphanedOnly: event.target.checked })} />
            Orphaned only (passage no longer found)
          </label>
          <div className="sm:col-span-2"><button type="submit" className={button}>Search</button>{loading && <span role="status" className="ml-2 text-slate-400">Searching…</span>}</div>
        </form>
        {error && <p role="alert" className="text-rose-300">{error}</p>}
        {applied && rows.length === 0 && !loading && !error && <p className="text-slate-400">No annotations match.</p>}
        {rows.length > 0 && (
          <ul aria-label="Annotation results" className="space-y-1.5">
            {rows.map((row) => {
              const state = row.anchor?.state ?? null;
              return (
                <li key={row.id} className="rounded border border-slate-800 p-2">
                  <div className="flex flex-wrap items-center gap-2 text-slate-400">
                    <span className="font-semibold text-slate-200">{row.fileName ?? `Document ${row.documentId}`}</span>
                    <span>{row.portalKey} / {row.documentId}</span>
                    <span>{KIND_LABELS[row.kind]}</span>
                    {row.tag && <span className="rounded border border-cyan-400/30 px-1.5 text-cyan-200">{row.tag}</span>}
                    <span>{row.author ?? 'author not recorded'} · {STATUS_LABELS[row.status].toLowerCase()}</span>
                    {state === 'ANCHORED' && <span className="text-emerald-200">Anchored</span>}
                    {state === 'RE_ANCHORED' && <span className="text-amber-200">Re-anchored</span>}
                    {state === 'ORPHANED' && <span className="text-rose-200">Orphaned</span>}
                    {state === null && row.quote !== '' && <span>not checked: text unavailable</span>}
                    {row.requestId !== null ? (
                      <button type="button" className="underline" onClick={() => open(row)}>
                        {state === 'ANCHORED' || state === 'RE_ANCHORED' ? 'Open at passage' : 'Open document'}
                      </button>
                    ) : <span>document not available</span>}
                  </div>
                  {row.quote && <blockquote className="mt-1 border-l border-slate-600 pl-2 text-slate-300">{row.quote}</blockquote>}
                  {state === 'ORPHANED' && <p className="mt-1 text-rose-200">{row.anchor?.reason} The quote above is as it was saved.</p>}
                  {row.note && <p className="mt-1 whitespace-pre-wrap">{row.note}</p>}
                </li>
              );
            })}
          </ul>
        )}
        {nextCursor && <button type="button" className={button} disabled={loading} onClick={more}>{loading ? 'Loading…' : 'Show more'}</button>}
        {applied && !nextCursor && rows.length > 0 && !loading && <p className="text-slate-400">That is every match: {rows.length.toLocaleString('en-US')}.</p>}
      </div>
    </details>
  );
}
