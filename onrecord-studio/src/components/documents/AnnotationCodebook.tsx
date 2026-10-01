import { useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ANNOTATION_LIMITS } from '../../types';
import type { AnnotationSettings, CodebookEntry, CodebookImportPreview } from '../../types';
import { downloadText } from './annotationLogic';
import { annotationButton as button, annotationField as field } from './annotationStyles';

const MAXIMUM_IMPORT_BYTES = 1024 * 1024;
const BLANK: CodebookEntry = { code: '', label: '', definition: '' };

/** The issue codes this device uses, with a plain-JSON exchange so a team can share one set. */
export default function AnnotationCodebook({
  settings,
  onChanged,
}: {
  settings: AnnotationSettings | null;
  onChanged: (settings: AnnotationSettings) => void;
}) {
  const gate = useLatestRequestGate();
  const [draft, setDraft] = useState<CodebookEntry>(BLANK);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);
  const [payload, setPayload] = useState('');
  const [preview, setPreview] = useState<CodebookImportPreview | null>(null);
  const [overwrite, setOverwrite] = useState<string[]>([]);

  const run = async (work: () => Promise<void>) => {
    setError(null);
    setStatus(null);
    try {
      await work();
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  };

  const save = () => run(async () => {
    const request = gate.begin();
    const next = await invokeCommand('save_codebook_entry', { entry: draft });
    if (!gate.isCurrent(request)) return;
    onChanged(next);
    setDraft(BLANK);
    setStatus(`Saved code ${draft.code.trim()}`);
  });

  const remove = (code: string) => run(async () => {
    const request = gate.begin();
    const next = await invokeCommand('delete_codebook_entry', { code });
    if (!gate.isCurrent(request)) return;
    onChanged(next);
    setRemoving(null);
  });

  const exportFile = () => run(async () => {
    const text = await invokeCommand('export_codebook', {});
    downloadText('onrecord-codebook.json', text);
    setStatus('Codebook downloaded');
  });

  const readFile = (file: File) => run(async () => {
    if (file.size > MAXIMUM_IMPORT_BYTES) throw new Error('A codebook file is at most 1 MB.');
    gate.invalidate();
    setPreview(null);
    setPayload(await file.text());
  });

  const previewImport = () => run(async () => {
    const request = gate.begin();
    const result = await invokeCommand('preview_codebook_import', { payload });
    if (!gate.isCurrent(request)) return;
    setPreview(result);
    setOverwrite([]);
  });

  const applyImport = () => run(async () => {
    const request = gate.begin();
    const next = await invokeCommand('apply_codebook_import', { payload, overwriteCodes: overwrite });
    if (!gate.isCurrent(request)) return;
    onChanged(next);
    setPreview(null);
    setPayload('');
    setStatus('Codebook imported');
  });

  const entries = settings?.codebook ?? [];

  return (
    <details className="rounded border border-slate-800 p-2">
      <summary className="cursor-pointer font-semibold text-slate-200">Codebook ({entries.length})</summary>
      <div className="mt-2 space-y-2">
        <p className="text-slate-400">Issue codes you choose from when marking a passage. A mark with a label that is not here is flagged as uncoded. Share the codebook as a plain JSON file.</p>
        {entries.length === 0 && <p className="text-slate-400">No codes yet.</p>}
        <ul aria-label="Codebook" className="space-y-1">
          {entries.map((entry) => (
            <li key={entry.code} className="rounded border border-slate-800 p-1.5">
              <span className="font-semibold text-cyan-200">{entry.code}</span> {entry.label}
              {entry.definition && <p className="whitespace-pre-wrap text-slate-400">{entry.definition}</p>}
              <div className="flex gap-2">
                <button type="button" className="underline" onClick={() => setDraft(entry)}>Edit</button>
                {removing === entry.code ? (
                  <>
                    <button type="button" className="text-rose-300 underline" onClick={() => void remove(entry.code)}>Confirm delete</button>
                    <button type="button" className="underline" onClick={() => setRemoving(null)}>Keep</button>
                    <span className="text-slate-400">Marks using this code stay and become uncoded.</span>
                  </>
                ) : (
                  <button type="button" className="underline" aria-label={`Delete code ${entry.code}`} onClick={() => setRemoving(entry.code)}>Delete</button>
                )}
              </div>
            </li>
          ))}
        </ul>
        <div className="grid gap-1.5 sm:grid-cols-2">
          <label>Code<input className={field} maxLength={ANNOTATION_LIMITS.tagCharacters} value={draft.code} onChange={(event) => setDraft({ ...draft, code: event.target.value })} /></label>
          <label>Label<input className={field} maxLength={ANNOTATION_LIMITS.codeLabelCharacters} value={draft.label} onChange={(event) => setDraft({ ...draft, label: event.target.value })} /></label>
        </div>
        <label className="block">Definition<textarea rows={2} className={field} maxLength={ANNOTATION_LIMITS.codeDefinitionCharacters} value={draft.definition} onChange={(event) => setDraft({ ...draft, definition: event.target.value })} /></label>
        <div className="flex flex-wrap gap-2">
          <button type="button" className={button} disabled={draft.code.trim() === '' || draft.label.trim() === ''} onClick={() => void save()}>Save code</button>
          <button type="button" className={button} disabled={entries.length === 0} onClick={() => void exportFile()}>Export codebook</button>
        </div>
        <label className="block">
          Open a codebook file
          <input type="file" accept=".json,application/json" className="mt-1 block w-full" onChange={(event) => { const file = event.target.files?.[0]; if (file) void readFile(file); }} />
        </label>
        <label className="block">
          Codebook JSON
          <textarea rows={3} className={`${field} font-mono`} value={payload} onChange={(event) => { gate.invalidate(); setPreview(null); setPayload(event.target.value); }} />
        </label>
        <button type="button" className={button} disabled={payload.trim() === ''} onClick={() => void previewImport()}>Preview import</button>
        {preview && (
          <div className="space-y-1.5" aria-label="Codebook import preview">
            <p role="status">{preview.added.length} new, {preview.identical} identical, {preview.conflicts.length} with a different definition.</p>
            {preview.conflicts.map((conflict) => (
              <div key={conflict.code} className="rounded border border-amber-500/30 p-1.5">
                <p className="font-semibold text-amber-200">{conflict.code} is defined differently</p>
                <p><span className="text-slate-400">Yours:</span> {conflict.local.label}. {conflict.local.definition}</p>
                <p><span className="text-slate-400">Incoming:</span> {conflict.incoming.label}. {conflict.incoming.definition}</p>
                <label className="flex items-center gap-1">
                  <input type="checkbox" checked={overwrite.includes(conflict.code)} onChange={(event) => setOverwrite(event.target.checked ? [...overwrite, conflict.code] : overwrite.filter((code) => code !== conflict.code))} />
                  Replace my definition of {conflict.code} with the incoming one
                </label>
              </div>
            ))}
            {preview.conflicts.length > 0 && <p className="text-slate-400">Definitions you leave unchecked are kept as they are.</p>}
            <button type="button" className={button} onClick={() => void applyImport()}>Apply import</button>
          </div>
        )}
        {error && <p role="alert" className="text-rose-300">{error}</p>}
        {status && <p role="status" className="text-emerald-300">{status}</p>}
      </div>
    </details>
  );
}
