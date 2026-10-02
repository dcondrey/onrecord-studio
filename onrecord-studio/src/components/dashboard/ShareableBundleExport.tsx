import { useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { BundlePreview, ShareableBundleResult } from '../../types/bundle';

interface ShareableBundleExportProps {
  /** The findings on screen. Each can be left out of the bundle before review. */
  findings: readonly { id: string; headline: string }[];
}

type Step =
  | { kind: 'select' }
  | { kind: 'preview'; ids: string[]; preview: BundlePreview }
  | { kind: 'done'; result: ShareableBundleResult };

function selectionKey(ids: readonly string[]) {
  return JSON.stringify([...ids].sort());
}

/**
 * Shares findings as one read-only HTML file. Nothing is written until the person has read what
 * the file will contain and confirmed it; the confirmation names the content hash that was shown.
 */
export default function ShareableBundleExport({ findings }: ShareableBundleExportProps) {
  const gate = useLatestRequestGate();
  const [excluded, setExcluded] = useState<ReadonlySet<string>>(new Set());
  const [step, setStep] = useState<Step>({ kind: 'select' });
  const [confirmed, setConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const selected = findings.map((finding) => finding.id).filter((id) => !excluded.has(id));
  const currentKey = selectionKey(selected);
  const currentKeyRef = useRef(currentKey);
  currentKeyRef.current = currentKey;

  // A review or export requested for one selection must never be presented for another.
  useEffect(() => {
    gate.invalidate();
    setStep({ kind: 'select' });
    setConfirmed(false);
    setBusy(false);
    setError('');
  }, [currentKey, gate]);

  function toggle(id: string) {
    setExcluded((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function review() {
    const token = gate.begin();
    const requestedKey = currentKey;
    setBusy(true);
    setError('');
    try {
      const preview = await invokeCommand('preview_shareable_bundle', { request: { findingIds: selected } });
      if (!gate.isCurrent(token) || currentKeyRef.current !== requestedKey) return;
      setConfirmed(false);
      setStep({ kind: 'preview', ids: selected, preview });
    } catch (reason) {
      if (!gate.isCurrent(token) || currentKeyRef.current !== requestedKey) return;
      setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(token)) setBusy(false);
    }
  }

  async function write(ids: string[], preview: BundlePreview) {
    const token = gate.begin();
    const requestedKey = currentKey;
    setBusy(true);
    setError('');
    try {
      const result = await invokeCommand('export_shareable_bundle', {
        request: { findingIds: ids, confirmedContentSha256: preview.contentSha256, targetPath: null },
      });
      if (!gate.isCurrent(token) || currentKeyRef.current !== requestedKey) return;
      setStep({ kind: 'done', result });
    } catch (reason) {
      if (!gate.isCurrent(token) || currentKeyRef.current !== requestedKey) return;
      setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(token)) setBusy(false);
    }
  }

  return (
    <section className="app-panel p-4" aria-label="Shareable bundle">
      <h3 className="font-semibold text-fg">Shareable read-only bundle</h3>
      <p className="mt-2 text-label leading-4 text-muted">
        Writes one HTML file anyone can open without this app. It has no edit controls, loads nothing from the
        network, and holds only masked text from public records. Findings that rest on a private or removed record
        are left out. You review exactly what it will contain before anything is written.
      </p>
      {step.kind === 'select' && (
        <>
          <ul className="mt-3 max-h-40 overflow-y-auto text-label leading-4 text-fg-secondary" aria-label="Findings to share">
            {findings.map((finding) => (
              <li key={finding.id}>
                <label className="flex items-start gap-2">
                  <input
                    type="checkbox"
                    checked={!excluded.has(finding.id)}
                    onChange={() => toggle(finding.id)}
                    aria-label={`Include: ${finding.headline}`}
                  />
                  <span>{finding.headline}</span>
                </label>
              </li>
            ))}
          </ul>
          <button
            type="button"
            className="focus-ring mt-3 rounded border border-control px-3 py-1.5 text-sm text-fg disabled:opacity-50"
            disabled={busy || selected.length === 0}
            onClick={() => void review()}
          >
            {busy ? 'Preparing…' : `Review ${selected.length} finding(s) to share`}
          </button>
        </>
      )}
      {step.kind === 'preview' && (
        <div className="mt-3 text-label leading-4 text-fg-secondary" role="region" aria-label="What will be shared">
          <p className="font-semibold text-fg">
            This file will contain {step.preview.included.length} finding(s) and name{' '}
            {step.preview.sourceNames.length} source file(s).
          </p>
          <ul className="mt-2 list-disc pl-5" aria-label="Findings that will be shared">
            {step.preview.included.map((item) => (
              <li key={item.findingId}>
                {item.headline} ({item.severity}; {item.basisLabel}; {item.sourceCount} source(s),{' '}
                {item.excerptCount} excerpt(s), {item.maskedValueCount} value(s) masked)
              </li>
            ))}
          </ul>
          {step.preview.sourceNames.length > 0 && (
            <p className="mt-2">Sources named: {step.preview.sourceNames.join('; ')}</p>
          )}
          {step.preview.excluded.length > 0 && (
            <>
              <p className="mt-2 font-semibold text-fg">
                Left out ({step.preview.excluded.length}):
              </p>
              <ul className="list-disc pl-5" aria-label="Findings left out">
                {step.preview.excluded.map((item) => (
                  <li key={item.findingId}>
                    {item.headline ?? item.findingId}: {item.explanation}
                  </li>
                ))}
              </ul>
            </>
          )}
          <p className="mt-2 break-all text-muted">Content SHA-256 {step.preview.contentSha256}</p>
          <label className="mt-3 flex items-start gap-2">
            <input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} />
            <span>
              I have reviewed this list and want to write a shareable file with exactly this content. Masking is
              structural and is not a guarantee that no sensitive value remains.
            </span>
          </label>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              className="focus-ring rounded border border-control px-3 py-1.5 text-sm text-fg disabled:opacity-50"
              disabled={busy || !confirmed || step.preview.included.length === 0}
              onClick={() => void write(step.ids, step.preview)}
            >
              {busy ? 'Writing…' : 'Write shareable file'}
            </button>
            <button
              type="button"
              className="focus-ring rounded border border-control px-3 py-1.5 text-sm text-fg-secondary"
              disabled={busy}
              onClick={() => setStep({ kind: 'select' })}
            >
              Back
            </button>
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="mt-3 text-sm text-danger">
          {error}
        </p>
      )}
      {step.kind === 'done' && (
        <div role="status" className="mt-3 rounded border border-line-strong bg-canvas/70 p-3 text-label leading-4 text-fg-secondary">
          <p className="font-semibold text-fg">
            Wrote {step.result.findingCount} finding(s); {step.result.excludedCount} left out.
          </p>
          <p className="mt-1 break-all">{step.result.outputPath}</p>
          <p className="mt-1 break-all">Content SHA-256 {step.result.contentSha256}</p>
          <p className="mt-1 text-muted">Generated {step.result.generatedAt}</p>
        </div>
      )}
    </section>
  );
}
