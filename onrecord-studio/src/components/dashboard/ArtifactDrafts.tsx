import { useCallback, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DraftedArtifact } from '../../types';

/**
 * Renders "Draft follow-up request" and "Draft appeal" buttons for one finding, and shows the
 * generated draft inline with editing, copy-to-clipboard, and download-as-.txt affordances.
 *
 * Every specific claim in the draft traces back to the finding — the generator never fabricates
 * facts. The user is free to edit the draft in place before sending; copy and download preserve
 * the edited text, not the original.
 */
export default function ArtifactDrafts({ findingId }: { findingId: string }) {
  const [draft, setDraft] = useState<DraftedArtifact | null>(null);
  const [editedText, setEditedText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<'followup' | 'appeal' | null>(null);
  const gate = useLatestRequestGate();

  const runDraft = useCallback(
    (kind: 'followup' | 'appeal') => {
      const request = gate.begin();
      setBusy(kind);
      setError(null);
      const command =
        kind === 'followup' ? 'draft_followup_from_finding' : 'draft_appeal_from_finding';
      invokeCommand(command, { findingId })
        .then((result) => {
          if (!gate.isCurrent(request)) return;
          setDraft(result);
          setEditedText(result.generatedText);
        })
        .catch((reason) => {
          if (!gate.isCurrent(request)) return;
          setError(String(reason));
        })
        .finally(() => {
          if (gate.isCurrent(request)) setBusy(null);
        });
    },
    [findingId, gate],
  );

  const copyToClipboard = useCallback(async () => {
    if (editedText == null) return;
    try {
      await navigator.clipboard.writeText(editedText);
    } catch (reason) {
      setError(`Copy to clipboard failed: ${String(reason)}`);
    }
  }, [editedText]);

  const download = useCallback(() => {
    if (draft == null || editedText == null) return;
    const blob = new Blob([editedText], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const filename = `${draft.kind.toLowerCase()}-${draft.subjectFindingId}.txt`;
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = filename;
    document.body.appendChild(anchor);
    anchor.click();
    document.body.removeChild(anchor);
    URL.revokeObjectURL(url);
  }, [draft, editedText]);

  return (
    <div
      className="mt-2 rounded-lg border border-slate-800/70 bg-slate-950/40 p-3"
      aria-label="Drafted artifacts"
    >
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          onClick={() => runDraft('followup')}
          disabled={busy === 'followup'}
          className="focus-ring rounded-md border border-slate-700 bg-slate-900/60 px-3 py-1 text-[10px] font-semibold text-slate-200 transition hover:border-cyan-400/40 disabled:opacity-60"
        >
          {busy === 'followup' ? 'Drafting…' : 'Draft follow-up request'}
        </button>
        <button
          type="button"
          onClick={() => runDraft('appeal')}
          disabled={busy === 'appeal'}
          className="focus-ring rounded-md border border-slate-700 bg-slate-900/60 px-3 py-1 text-[10px] font-semibold text-slate-200 transition hover:border-amber-400/40 disabled:opacity-60"
        >
          {busy === 'appeal' ? 'Drafting…' : 'Draft appeal'}
        </button>
      </div>

      {error && (
        <div role="alert" className="mt-2 rounded-md border border-rose-500/20 bg-rose-500/[0.04] px-2 py-1.5 text-[9px] text-rose-200">
          {error}
        </div>
      )}

      {draft && editedText != null && (
        <div className="mt-3 flex flex-col gap-2">
          <div className="flex flex-wrap items-baseline justify-between gap-2">
            <div className="text-[9px] uppercase tracking-wider text-slate-400">
              {draft.kind === 'FOLLOWUP_REQUEST' ? 'Drafted follow-up request' : 'Drafted appeal'}
              {' · '}template {draft.templateId}
              {' · '}jurisdiction {draft.jurisdiction}
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={copyToClipboard}
                className="focus-ring rounded-md border border-slate-700 px-2 py-0.5 text-[9px] font-medium text-slate-300 transition hover:border-cyan-400/40"
              >
                Copy
              </button>
              <button
                type="button"
                onClick={download}
                className="focus-ring rounded-md border border-slate-700 px-2 py-0.5 text-[9px] font-medium text-slate-300 transition hover:border-cyan-400/40"
              >
                Download .txt
              </button>
            </div>
          </div>
          <textarea
            value={editedText}
            onChange={(event) => setEditedText(event.target.value)}
            aria-label="Drafted artifact text (editable)"
            className="min-h-[220px] w-full resize-y rounded-md border border-slate-800 bg-slate-950/70 p-3 font-mono text-[10px] leading-relaxed text-slate-200 focus:border-cyan-400/40 focus:outline-none focus:ring-1 focus:ring-cyan-400/30"
          />
          {draft.citedAuthorities.length > 0 && (
            <details className="text-[9px] text-slate-400">
              <summary className="cursor-pointer">
                Cited authorities ({draft.citedAuthorities.length})
              </summary>
              <ul className="ml-4 mt-1 list-disc space-y-0.5">
                {draft.citedAuthorities.map((authority, index) => (
                  <li key={`${authority.statute}-${index}`}>{authority.statute}</li>
                ))}
              </ul>
            </details>
          )}
          <p className="text-[8px] italic text-slate-400">
            The app produces drafts only — nothing is sent. Review and edit before submitting.
          </p>
        </div>
      )}
    </div>
  );
}
