import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ANNOTATION_LIMITS } from '../../types';
import type { AnnotationKind, DocumentAnnotation } from '../../types';

export interface AnnotationAnchor {
  quote: string;
  characterOffset: number;
}

const KIND_LABELS: Record<AnnotationKind, string> = {
  NOTE: 'Note',
  HIGHLIGHT: 'Highlight',
  ISSUE: 'Issue code',
};

const field = 'focus-ring w-full rounded border border-slate-700 bg-slate-950 px-2 py-1 text-slate-100';
const button = 'focus-ring rounded border border-slate-600 px-2 py-1 font-semibold text-slate-200 hover:border-cyan-400/60 disabled:opacity-50';

export function annotationsToMarkdown(fileName: string, portalKey: string, documentId: number, annotations: DocumentAnnotation[]): string {
  const lines = [`## ${fileName} (${portalKey} / document ${documentId})`, ''];
  for (const item of annotations) {
    const label = item.tag ? `${KIND_LABELS[item.kind]}: ${item.tag}` : KIND_LABELS[item.kind];
    lines.push(`- **${label}** at character ${item.characterOffset.toLocaleString('en-US')}`);
    if (item.quote) lines.push(`  > ${item.quote.replace(/\s*\n\s*/g, ' ')}`);
    if (item.note) lines.push(`  ${item.note.replace(/\n/g, '\n  ')}`);
  }
  lines.push('', 'Offsets locate text in the extracted copy of the record. Notes are the reviewer\'s own and are not part of the record.');
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
  currentOffset,
  onJump,
}: {
  portalKey: string;
  documentId: number;
  fileName: string;
  readSelection: () => AnnotationAnchor | null;
  currentOffset: number;
  onJump: (characterOffset: number) => boolean;
}) {
  const listGate = useLatestRequestGate();
  const gate = useLatestRequestGate();
  const [items, setItems] = useState<DocumentAnnotation[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [kind, setKind] = useState<AnnotationKind>('NOTE');
  const [anchor, setAnchor] = useState<AnnotationAnchor | null>(null);
  const [tag, setTag] = useState('');
  const [note, setNote] = useState('');
  const [isSaving, setIsSaving] = useState(false);
  const [confirmingRemoval, setConfirmingRemoval] = useState<string | null>(null);

  const load = useCallback(() => {
    const request = listGate.begin();
    setLoadError(null);
    invokeCommand('list_document_annotations', { portalKey, documentId })
      .then((rows) => {
        if (listGate.isCurrent(request)) setItems(rows);
      })
      .catch((reason) => {
        if (listGate.isCurrent(request)) setLoadError(formatNativeError(reason));
      });
  }, [documentId, listGate, portalKey]);

  useEffect(() => {
    load();
  }, [load]);

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
    && (needsPassage ? anchor !== null : note.trim() !== '')
    && (kind !== 'ISSUE' || tag.trim() !== '');

  const save = async () => {
    const request = gate.begin();
    setIsSaving(true);
    setActionError(null);
    try {
      await invokeCommand('add_document_annotation', {
        input: {
          portalKey,
          documentId,
          kind,
          characterOffset: anchor?.characterOffset ?? currentOffset,
          quote: anchor?.quote ?? '',
          note,
          tag: tag.trim() === '' ? null : tag,
        },
      });
      if (!gate.isCurrent(request)) return;
      setAnchor(null);
      setNote('');
      setTag('');
      setStatus('Saved on this computer');
      load();
    } catch (reason) {
      if (gate.isCurrent(request)) setActionError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(request)) setIsSaving(false);
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

  const copyMarkdown = async () => {
    if (!items || items.length === 0) return;
    const request = gate.begin();
    try {
      await navigator.clipboard.writeText(annotationsToMarkdown(fileName, portalKey, documentId, items));
      if (gate.isCurrent(request)) setStatus('Copied annotations as Markdown');
    } catch {
      if (gate.isCurrent(request)) setActionError('The clipboard is unavailable. Copy the notes by hand.');
    }
  };

  const jump = (offset: number) => {
    setStatus(onJump(offset) ? null : 'That passage is in another text chunk. Use Previous or Next chunk to reach it.');
  };

  return (
    <section aria-label="Annotations" className="space-y-2 border-b border-slate-800 p-3 text-[10px] text-slate-300">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-[11px] font-semibold text-slate-200">Annotations</h3>
        <button type="button" className={button} disabled={!items || items.length === 0} onClick={() => void copyMarkdown()}>
          Copy as Markdown
        </button>
      </div>
      <p className="text-slate-400">Your notes stay on this computer. They record your reading, not what the record shows.</p>

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
          {kind === 'ISSUE' ? 'Issue label (required)' : 'Label (optional)'}
          <input className={field} maxLength={ANNOTATION_LIMITS.tagCharacters} value={tag} onChange={(event) => setTag(event.target.value)} />
        </label>
      </div>
      {needsPassage && (
        <div>
          <button type="button" className={button} onClick={markSelection}>Use selected text</button>
          {anchor && <blockquote className="mt-1 border-l border-cyan-400 pl-2 text-slate-300">{anchor.quote}</blockquote>}
        </div>
      )}
      <label className="block">
        Note{kind === 'NOTE' ? '' : ' (optional)'}
        <textarea rows={2} maxLength={ANNOTATION_LIMITS.noteCharacters} className={field} value={note} onChange={(event) => setNote(event.target.value)} />
      </label>
      <button type="button" className={button} disabled={!canSave} onClick={() => void save()}>
        {isSaving ? 'Saving…' : 'Save annotation'}
      </button>
      {actionError && <p role="alert" className="text-rose-300">{actionError}</p>}
      {status && <p role="status" className="text-emerald-300">{status}</p>}
      {loadError && (
        <p role="alert" className="text-rose-300">
          Annotations could not be read: {loadError} <button type="button" className="underline" onClick={load}>Retry</button>
        </p>
      )}
      {items && items.length === 0 && !loadError && <p className="text-slate-400">No annotations on this document yet.</p>}
      {items && items.length > 0 && (
        <ul aria-label="Saved annotations" className="flex flex-col gap-1.5">
          {items.map((item) => (
            <li key={item.id} className="rounded border border-slate-800 p-2">
              <div className="flex flex-wrap items-center gap-2 text-slate-400">
                <span className="font-semibold text-slate-200">{KIND_LABELS[item.kind]}</span>
                {item.tag && <span className="rounded border border-cyan-400/30 px-1.5 text-cyan-200">{item.tag}</span>}
                <span>character {item.characterOffset.toLocaleString('en-US')}</span>
                <button type="button" className="underline" onClick={() => jump(item.characterOffset)}>Go to passage</button>
                {confirmingRemoval === item.id ? (
                  <>
                    <button type="button" className="text-rose-300 underline" onClick={() => void remove(item.id)}>Confirm delete</button>
                    <button type="button" className="underline" onClick={() => setConfirmingRemoval(null)}>Keep</button>
                  </>
                ) : (
                  <button type="button" className="underline" aria-label={`Delete ${KIND_LABELS[item.kind].toLowerCase()} at character ${item.characterOffset}`} onClick={() => setConfirmingRemoval(item.id)}>Delete</button>
                )}
              </div>
              {item.quote && <blockquote className="mt-1 border-l border-slate-600 pl-2 text-slate-300">{item.quote}</blockquote>}
              {item.note && <p className="mt-1 whitespace-pre-wrap">{item.note}</p>}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
