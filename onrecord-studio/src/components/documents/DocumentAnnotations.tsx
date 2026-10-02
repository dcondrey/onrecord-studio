import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ANNOTATION_LIMITS } from '../../types';
import type { AnnotationKind, AnnotationSettings, AnnotationStatus, DocumentAnnotationList, Finding, ResolvedAnnotation } from '../../types';
import AnnotationAgreement from './AnnotationAgreement';
import AnnotationCodebook from './AnnotationCodebook';
import AnnotationExchange from './AnnotationExchange';
import { annotationCitation, isCoded, KIND_LABELS, OFFSET_STATEMENT, REGION_STATEMENT, STATUS_LABELS } from './annotationLogic';
import { pageViewBridge, pageViewKey, usePageViewState } from './pageViewBridge';
import { percent } from './pageRegionLogic';
import { annotationButton as button, annotationField as field } from './annotationStyles';

export interface AnnotationAnchor {
  quote: string;
  prefix: string;
  suffix: string;
}

export interface PassageTarget extends AnnotationAnchor {
  sourceOffset: number | null;
}

const FINDING_PICKER_LIMIT = 200;
const OTHER_CODE = '\u0000other';

export function annotationsToMarkdown(fileName: string, portalKey: string, documentId: number, annotations: ResolvedAnnotation[]): string {
  const lines = [`## ${fileName} (${portalKey} / document ${documentId})`, ''];
  for (const item of annotations) {
    const label = item.tag ? `${KIND_LABELS[item.kind]}: ${item.tag}` : KIND_LABELS[item.kind];
    const where = item.pageRegion
      ? `page ${item.pageRegion.pageIndex + 1}, rectangle left ${percent(item.pageRegion.x)} top ${percent(item.pageRegion.y)} width ${percent(item.pageRegion.width)} height ${percent(item.pageRegion.height)} of the original file's page`
      : item.anchor?.state === 'ORPHANED'
      ? 'passage not found in the current text'
      : item.anchor?.sourceOffset != null
        ? `character ${item.anchor.sourceOffset.toLocaleString('en-US')}`
        : item.quote ? 'position not checked' : 'whole document';
    lines.push(`- **${label}** (${where}${item.author ? `, ${item.author}` : ''}, ${STATUS_LABELS[item.status].toLowerCase()})`);
    if (item.quote) lines.push(`  > ${item.quote.replace(/\s*\n\s*/g, ' ')}`);
    if (item.note) lines.push(`  ${item.note.replace(/\n/g, '\n  ')}`);
  }
  lines.push('', `${OFFSET_STATEMENT} ${REGION_STATEMENT} Notes are the reviewer's own and are not part of the record.`);
  return lines.join('\n');
}

/**
 * Reviewer notes, highlights, and issue codes on one document's extracted text. Mount it keyed by
 * document: every async result is also checked against the gate, so a list or save that settles
 * after the viewer has moved on is dropped instead of being shown against the wrong document.
 */
export default function DocumentAnnotations({
  portalKey,
  documentId,
  fileName,
  readSelection,
  onJump,
}: {
  portalKey: string;
  documentId: number;
  fileName: string;
  readSelection: () => AnnotationAnchor | null;
  onJump: (target: PassageTarget) => boolean;
}) {
  const listGate = useLatestRequestGate();
  const settingsGate = useLatestRequestGate();
  const findingGate = useLatestRequestGate();
  const gate = useLatestRequestGate();
  const saveGate = useLatestRequestGate();
  const viewKey = pageViewKey(portalKey, documentId);
  const pageView = usePageViewState(viewKey);
  const pending = pageView.pending;
  const pendingRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (pending) pendingRef.current?.scrollIntoView?.({ block: 'nearest' });
  }, [pending]);
  const [list, setList] = useState<DocumentAnnotationList | null>(null);
  const [settings, setSettings] = useState<AnnotationSettings | null>(null);
  const [findings, setFindings] = useState<Finding[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [kind, setKind] = useState<AnnotationKind>('NOTE');
  const [anchor, setAnchor] = useState<AnnotationAnchor | null>(null);
  const [codeChoice, setCodeChoice] = useState('');
  const [freeCode, setFreeCode] = useState('');
  const [findingId, setFindingId] = useState('');
  const [note, setNote] = useState('');
  const [reviewerDraft, setReviewerDraft] = useState('');
  const [editingReviewer, setEditingReviewer] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [confirmingRemoval, setConfirmingRemoval] = useState<string | null>(null);

  const load = useCallback(() => {
    const request = listGate.begin();
    setLoadError(null);
    pageViewBridge.update(viewKey, (state) => ({ revision: state.revision + 1 }));
    invokeCommand('list_document_annotations', { portalKey, documentId })
      .then((result) => {
        if (listGate.isCurrent(request)) setList(result);
      })
      .catch((reason) => {
        if (listGate.isCurrent(request)) setLoadError(formatNativeError(reason));
      });
  }, [documentId, listGate, portalKey, viewKey]);

  const loadSettings = useCallback(() => {
    const request = settingsGate.begin();
    invokeCommand('get_annotation_settings', {})
      .then((result) => {
        if (settingsGate.isCurrent(request)) setSettings(result);
      })
      .catch((reason) => {
        if (settingsGate.isCurrent(request)) setLoadError(formatNativeError(reason));
      });
  }, [settingsGate]);

  useEffect(() => {
    load();
    loadSettings();
  }, [load, loadSettings]);

  useEffect(() => {
    const request = findingGate.begin();
    invokeCommand('list_findings', { filter: { portalKey, documentId }, limit: FINDING_PICKER_LIMIT })
      .then((rows) => {
        if (findingGate.isCurrent(request)) setFindings(rows);
      })
      .catch(() => {
        if (findingGate.isCurrent(request)) setFindings([]);
      });
  }, [documentId, findingGate, portalKey]);

  const codes = settings?.codebook ?? [];
  const tag = codeChoice === OTHER_CODE ? freeCode.trim() : codeChoice;

  const markSelection = () => {
    const selected = readSelection();
    if (!selected) {
      setActionError('Select text in the document first, then choose "Use selected text".');
      return;
    }
    if (selected.quote.length > ANNOTATION_LIMITS.quoteCharacters) {
      setActionError(`Select at most ${ANNOTATION_LIMITS.quoteCharacters.toLocaleString('en-US')} characters.`);
      return;
    }
    setActionError(null);
    setAnchor(selected);
  };

  const needsPassage = kind !== 'NOTE';
  const canSave = !isSaving
    && settings?.reviewer != null
    && (needsPassage ? anchor !== null || pending !== null : note.trim() !== '')
    && (kind !== 'ISSUE' || tag !== '');

  const save = async () => {
    const request = saveGate.begin();
    setIsSaving(true);
    setActionError(null);
    try {
      await invokeCommand('add_document_annotation', {
        input: {
          portalKey,
          documentId,
          kind,
          quote: pending ? pending.quote : anchor?.quote ?? '',
          prefix: pending ? '' : anchor?.prefix ?? '',
          suffix: pending ? '' : anchor?.suffix ?? '',
          note,
          tag: tag === '' ? null : tag,
          findingId: findingId === '' ? null : findingId,
          pageRegion: pending ? pending.rectangle : null,
        },
      });
      if (!saveGate.isCurrent(request)) return;
      setAnchor(null);
      pageViewBridge.update(viewKey, { pending: null });
      setNote('');
      setCodeChoice('');
      setFreeCode('');
      setFindingId('');
      setStatus('Saved on this computer');
      load();
    } catch (reason) {
      if (saveGate.isCurrent(request)) setActionError(formatNativeError(reason));
    } finally {
      if (saveGate.isCurrent(request)) setIsSaving(false);
    }
  };

  const saveReviewer = async () => {
    const request = gate.begin();
    setActionError(null);
    try {
      const next = await invokeCommand('set_annotation_reviewer', { reviewer: reviewerDraft });
      if (!gate.isCurrent(request)) return;
      setSettings(next);
      setEditingReviewer(false);
      setReviewerDraft('');
    } catch (reason) {
      if (gate.isCurrent(request)) setActionError(formatNativeError(reason));
    }
  };

  const remove = async (id: string) => {
    const request = gate.begin();
    setActionError(null);
    try {
      await invokeCommand('remove_document_annotation', { id });
      if (!gate.isCurrent(request)) return;
      setConfirmingRemoval(null);
      load();
    } catch (reason) {
      if (gate.isCurrent(request)) setActionError(formatNativeError(reason));
    }
  };

  const changeStatus = async (id: string, next: AnnotationStatus) => {
    const request = gate.begin();
    setActionError(null);
    try {
      await invokeCommand('set_annotation_status', { id, status: next });
      if (gate.isCurrent(request)) load();
    } catch (reason) {
      if (gate.isCurrent(request)) setActionError(formatNativeError(reason));
    }
  };

  const items = useMemo(() => list?.annotations ?? [], [list]);

  const copy = async (text: string, done: string) => {
    const request = gate.begin();
    try {
      await navigator.clipboard.writeText(text);
      if (gate.isCurrent(request)) setStatus(done);
    } catch {
      if (gate.isCurrent(request)) setActionError('The clipboard is unavailable. Copy the text by hand.');
    }
  };

  const jump = (item: ResolvedAnnotation) => {
    const reached = onJump({
      quote: item.quote,
      prefix: item.prefix,
      suffix: item.suffix,
      sourceOffset: item.anchor?.sourceOffset ?? null,
    });
    setStatus(reached ? null : 'That passage is not in the text chunk on screen. Use Previous or Next chunk to reach it.');
  };

  const regionState = (item: ResolvedAnnotation) => (item.pageRegion ? pageView.states[item.id] ?? null : null);
  const isOrphan = (item: ResolvedAnnotation) => item.anchor?.state === 'ORPHANED' || regionState(item) === 'ORPHANED';
  const placed = items.filter((item) => !isOrphan(item));
  const orphaned = items.filter(isOrphan);

  const showRegion = (item: ResolvedAnnotation) => {
    pageViewBridge.update(viewKey, (state) => ({ selectedId: item.id, jump: state.jump + 1 }));
    setStatus(pageView.open ? null : 'Turn on "Page view" in the viewer toolbar to see this region on its page.');
  };

  const row = (item: ResolvedAnnotation) => {
    const state = item.anchor?.state ?? null;
    const region = item.pageRegion;
    const checked = regionState(item);
    const own = item.importedFromKey === null;
    const linked = item.findingId === null ? null : findings.find((finding) => finding.id === item.findingId);
    const changed = list?.currentTextSha256 != null && item.textSha256 !== null && item.textSha256 !== list.currentTextSha256;
    return (
      <li key={item.id} className={`rounded border p-2 ${pageView.selectedId === item.id ? 'border-indigo-400' : 'border-slate-800'}`}>
        <div className="flex flex-wrap items-center gap-2 text-slate-400">
          <span className="font-semibold text-slate-200">{KIND_LABELS[item.kind]}</span>
          {item.tag && (
            <span className="rounded border border-cyan-400/30 px-1.5 text-cyan-200">
              {item.tag}{!isCoded(item.tag, codes) && ' (uncoded)'}
            </span>
          )}
          {state === 'ANCHORED' && <span className="rounded border border-emerald-400/30 px-1.5 text-emerald-200">Anchored</span>}
          {state === 'RE_ANCHORED' && <span className="rounded border border-amber-400/30 px-1.5 text-amber-200">Re-anchored</span>}
          {state === 'ORPHANED' && <span className="rounded border border-rose-400/40 px-1.5 text-rose-200">Orphaned</span>}
          {item.anchor?.sourceOffset != null && <span>character {item.anchor.sourceOffset.toLocaleString('en-US')}</span>}
          {region && <span className="rounded border border-slate-500 px-1.5 text-slate-200">Page {region.pageIndex + 1}</span>}
          {checked === 'ANCHORED' && <span className="rounded border border-emerald-400/30 px-1.5 text-emerald-200">Original matches</span>}
          {checked === 'ORIGINAL_CHANGED' && <span className="rounded border border-amber-400/50 px-1.5 text-amber-200">Original changed</span>}
          {checked === 'ORPHANED' && <span className="rounded border border-rose-400/40 px-1.5 text-rose-200">Orphaned: page missing</span>}
          {region && checked === null && <span>original not checked</span>}
          {!region && state === null && item.quote === '' && <span>whole document</span>}
          {!region && state === null && item.quote !== '' && <span>not checked: document text unavailable</span>}
          {region && checked !== 'ORPHANED' && <button type="button" className="underline" onClick={() => showRegion(item)}>Show on page {region.pageIndex + 1}</button>}
          {(state === 'ANCHORED' || state === 'RE_ANCHORED') && (
            <button type="button" className="underline" onClick={() => jump(item)}>Go to passage</button>
          )}
          <button type="button" className="underline" onClick={() => void copy(annotationCitation(item, { fileName, regionState: checked, currentTextSha256: list?.currentTextSha256 ?? null, appVersion: __APP_VERSION__ }), 'Copied citation')}>Copy citation</button>
          {confirmingRemoval === item.id ? (
            <>
              <button type="button" className="text-rose-300 underline" onClick={() => void remove(item.id)}>Confirm delete</button>
              <button type="button" className="underline" onClick={() => setConfirmingRemoval(null)}>Keep</button>
            </>
          ) : (
            <button type="button" className="underline" aria-label={`Delete ${KIND_LABELS[item.kind].toLowerCase()}${item.quote ? ` "${item.quote.slice(0, 40)}"` : ''}`} onClick={() => setConfirmingRemoval(item.id)}>Delete</button>
          )}
        </div>
        <p className="mt-1 text-slate-400">
          {item.author ?? 'Author not recorded'}
          {item.importedFromKey && ` · from a colleague, signed by key ${item.importedFromKey.slice(0, 12)}`}
          {' · '}
          {own ? (
            <label>
              <span className="sr-only">Status</span>
              <select aria-label={`Status of ${KIND_LABELS[item.kind].toLowerCase()} ${item.id.slice(0, 8)}`} className="focus-ring rounded border border-slate-700 bg-slate-950 px-1 text-slate-200" value={item.status} onChange={(event) => void changeStatus(item.id, event.target.value as AnnotationStatus)}>
                {(Object.keys(STATUS_LABELS) as AnnotationStatus[]).map((value) => <option key={value} value={value}>{STATUS_LABELS[value]}</option>)}
              </select>
            </label>
          ) : STATUS_LABELS[item.status]}
        </p>
        {!region && item.contextUnanchored && item.quote !== '' && <p className="mt-1 text-amber-200">Unanchored context: made before surrounding text was kept, so it is located by its quote and position alone.</p>}
        {region && item.quote === '' && <p className="mt-1 text-slate-400">No text layer text in this region (an image, a scan, or no text layer), so nothing is quoted.</p>}
        {region && checked === 'ORIGINAL_CHANGED' && <p className="mt-1 text-amber-200">The original file has changed since this region was drawn. The rectangle is shown where it was drawn and may no longer cover what it covered.</p>}
        {region && <p className="mt-1 text-slate-400">Original file SHA-256 when drawn: {region.originalSha256}</p>}
        {!region && item.quote !== '' && item.textSha256 === null && <p className="mt-1 text-slate-400">The document text digest was not recorded for this mark.</p>}
        {changed && <p className="mt-1 text-amber-200">The document text has changed since this was marked.</p>}
        {checked === 'ORPHANED' && <p className="mt-1 text-rose-200">The original file no longer has page {(region?.pageIndex ?? 0) + 1}. The region is kept as saved and is not moved.</p>}
        {state === 'ORPHANED' && <p className="mt-1 text-rose-200">{item.anchor?.reason} Stored quote shown below; it is not located in the current text.</p>}
        {item.quote && <blockquote className="mt-1 border-l border-slate-600 pl-2 text-slate-300">{item.quote}</blockquote>}
        {item.note && <p className="mt-1 whitespace-pre-wrap">{item.note}</p>}
        {item.findingId && <p className="mt-1 text-slate-400">Linked finding: {linked ? linked.headline : `${item.findingId} (not in this document's current findings)`}</p>}
      </li>
    );
  };

  return (
    <section aria-label="Annotations" className={`space-y-2 border-b border-slate-800 p-3 text-label text-slate-300${pageView.open ? ' w-[24rem] max-w-[45%] shrink-0 overflow-y-auto border-r' : ''}`}>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-label font-semibold text-slate-200">Annotations</h3>
        <button type="button" className={button} disabled={items.length === 0} onClick={() => void copy(annotationsToMarkdown(fileName, portalKey, documentId, items), 'Copied annotations as Markdown')}>
          Copy as Markdown
        </button>
      </div>
      <p className="text-slate-400">Your notes stay on this computer. They record your reading, not what the record shows. A text annotation's position refers to the extracted text. A region drawn in the page view refers to a rectangle on a page of the original PDF.</p>

      {settings && (settings.reviewer === null || editingReviewer) ? (
        <div className="flex flex-wrap items-end gap-2">
          <label>
            Your name on annotations
            <input className={field} maxLength={ANNOTATION_LIMITS.authorCharacters} value={reviewerDraft} onChange={(event) => setReviewerDraft(event.target.value)} />
          </label>
          <button type="button" className={button} disabled={reviewerDraft.trim() === ''} onClick={() => void saveReviewer()}>Save name</button>
        </div>
      ) : settings && (
        <p className="text-slate-400">Annotating as <span className="text-slate-200">{settings.reviewer}</span>. <button type="button" className="underline" onClick={() => setEditingReviewer(true)}>Change</button></p>
      )}

      <div className="grid gap-1.5 sm:grid-cols-[9rem_minmax(0,1fr)]">
        <label>
          Type
          <select className={field} value={kind} onChange={(event) => setKind(event.target.value as AnnotationKind)}>
            {(Object.keys(KIND_LABELS) as AnnotationKind[]).map((value) => (
              <option key={value} value={value}>{KIND_LABELS[value]}</option>
            ))}
          </select>
        </label>
        <label>
          {kind === 'ISSUE' ? 'Issue code (required)' : 'Issue code (optional)'}
          <select className={field} value={codeChoice} onChange={(event) => setCodeChoice(event.target.value)}>
            <option value="">{kind === 'ISSUE' ? 'Choose a code' : 'No code'}</option>
            {codes.map((entry) => <option key={entry.code} value={entry.code}>{entry.code}: {entry.label}</option>)}
            <option value={OTHER_CODE}>Other (free text, uncoded)</option>
          </select>
        </label>
      </div>
      {codeChoice === OTHER_CODE && (
        <label className="block">
          Free-text label
          <input className={field} maxLength={ANNOTATION_LIMITS.tagCharacters} value={freeCode} onChange={(event) => setFreeCode(event.target.value)} />
          <span className="text-amber-200">Not in the codebook, so it is flagged as uncoded and will not agree with a coded mark.</span>
        </label>
      )}
      {pending && (
        <div ref={pendingRef} className="rounded border border-cyan-400/40 p-2">
          <p className="font-semibold text-slate-200">
            Region on page {pending.rectangle.pageIndex + 1}: left {percent(pending.rectangle.x)}, top {percent(pending.rectangle.y)}, width {percent(pending.rectangle.width)}, height {percent(pending.rectangle.height)}
          </p>
          {pending.quote ? <blockquote className="mt-1 border-l border-cyan-400 pl-2 text-slate-300">{pending.quote}</blockquote> : <p className="text-slate-400">No text layer text in this region (an image, a scan, or no text layer): nothing will be quoted.</p>}
          <button type="button" className={`${button} mt-1`} onClick={() => pageViewBridge.update(viewKey, { pending: null })}>Discard region</button>
        </div>
      )}
      {needsPassage && !pending && (
        <div>
          <button type="button" className={button} onClick={markSelection}>Use selected text</button>
          {anchor && <blockquote className="mt-1 border-l border-cyan-400 pl-2 text-slate-300">{anchor.quote}</blockquote>}
        </div>
      )}
      <label className="block">
        Note{kind === 'NOTE' ? '' : ' (optional)'}
        <textarea rows={2} maxLength={ANNOTATION_LIMITS.noteCharacters} className={field} value={note} onChange={(event) => setNote(event.target.value)} />
      </label>
      <label className="block">
        Linked finding (optional)
        <select className={field} value={findingId} onChange={(event) => setFindingId(event.target.value)}>
          <option value="">No linked finding</option>
          {findings.map((finding) => <option key={finding.id} value={finding.id}>{finding.headline}</option>)}
        </select>
        {findings.length >= FINDING_PICKER_LIMIT && <span className="text-amber-200">Showing the first {FINDING_PICKER_LIMIT} findings for this document.</span>}
      </label>
      <button type="button" className={button} disabled={!canSave} onClick={() => void save()}>
        {isSaving ? 'Saving…' : 'Save annotation'}
      </button>
      {actionError && <p role="alert" className="text-rose-300">{actionError}</p>}
      {status && <p role="status" className="text-emerald-300">{status}</p>}
      {loadError && (
        <p role="alert" className="text-rose-300">
          Annotations could not be read: {loadError} <button type="button" className="underline" onClick={() => { load(); loadSettings(); }}>Retry</button>
        </p>
      )}
      {list && list.annotations.length === 0 && !loadError && <p className="text-slate-400">No annotations on this document yet.</p>}
      {list && !list.textAvailable && items.some((item) => item.quote !== '') && (
        <p className="text-amber-200">This document's text is not available, so where each passage now sits could not be checked.</p>
      )}
      {placed.length > 0 && <ul aria-label="Saved annotations" className="flex flex-col gap-1.5">{placed.map(row)}</ul>}
      {orphaned.length > 0 && (
        <div>
          <h4 className="font-semibold text-rose-200">Orphaned annotations ({orphaned.length})</h4>
          <p className="text-slate-400">The quoted text is missing from the current document text, or now appears more than once. These are kept with the quote as it was saved and are not moved on a guess.</p>
          <ul aria-label="Orphaned annotations" className="mt-1 flex flex-col gap-1.5">{orphaned.map(row)}</ul>
        </div>
      )}
      <AnnotationCodebook settings={settings} onChanged={setSettings} />
      <AnnotationAgreement annotations={items} />
      <AnnotationExchange portalKey={portalKey} documentId={documentId} hasOwnMarks={items.some((item) => item.importedFromKey === null)} onImported={load} />
    </section>
  );
}
