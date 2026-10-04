import { lazy, Suspense, useState } from 'react';
import type { DocumentSourceHistory } from '../../types';
import { describeAcquisition, describeCheck, sourceHistoryEntries } from './sourceHistoryLogic';

// pdf.js loads only when a kept copy is opened.
const KeptCopyView = lazy(() => import('./KeptCopyView'));

interface SourceHistoryPanelProps {
  documentId: number;
  portalKey: string;
  history: DocumentSourceHistory | null;
  isLoading: boolean;
  error: string | null;
}

/**
 * Every recorded fetch of one document from its source, newest first: what collection received,
 * and what each source check observed. Mount it keyed by document.
 */
export default function SourceHistoryPanel({ documentId, portalKey, history, isLoading, error }: SourceHistoryPanelProps) {
  const [openCheckId, setOpenCheckId] = useState<number | null>(null);
  const entries = history ? sourceHistoryEntries(history) : [];
  return (
    <section aria-label="Source history" className="max-h-[50vh] space-y-2 overflow-y-auto border-b border-line px-4 py-3 text-caption">
      <h3 className="text-label font-semibold text-fg">Source history</h3>
      {isLoading && <p role="status" className="text-muted">Loading source history…</p>}
      {error && <p role="alert" className="text-danger">Source history could not be read: {error}</p>}
      {history && entries.length === 0 && (
        <p className="text-muted">No fetch of this document is recorded. Check source from the source toolbar records one.</p>
      )}
      <ol className="space-y-2">
        {entries.map((entry) => {
          if (entry.kind === 'acquisition') {
            const { title, detail } = describeAcquisition(entry.acquisition);
            return (
              <li key={`acquisition-${entry.acquisition.acquisitionId}`} className="rounded-lg border border-line px-3 py-2">
                <div className="flex flex-wrap items-baseline justify-between gap-2">
                  <span className="font-medium text-fg">{title}</span>
                  <time className="font-mono text-muted" dateTime={entry.at}>{entry.at}</time>
                </div>
                <p className="text-fg-secondary">{detail}</p>
                <p className="break-all font-mono text-muted">{entry.acquisition.resolvedUrl}</p>
              </li>
            );
          }
          const { check } = entry;
          const { title, detail } = describeCheck(check);
          const canOpen = check.outcome === 'DIFFERENT_BYTES' && check.observedCopyRetained;
          const isOpen = openCheckId === check.checkId;
          return (
            <li key={`check-${check.checkId}`} className="rounded-lg border border-line px-3 py-2">
              <div className="flex flex-wrap items-baseline justify-between gap-2">
                <span className="font-medium text-fg">Source check · {title}</span>
                <time className="font-mono text-muted" dateTime={entry.at}>{entry.at}</time>
              </div>
              <p className="text-fg-secondary">{detail}</p>
              <p className="break-all font-mono text-muted">{check.requestedUrl}</p>
              {canOpen && (
                <button type="button" onClick={() => setOpenCheckId(isOpen ? null : check.checkId)} aria-expanded={isOpen} className="mt-1 rounded border border-control px-2 py-1 text-fg-secondary transition hover:border-highlight/30 hover:text-highlight">
                  {isOpen ? 'Close kept copy' : 'Open kept copy'}
                </button>
              )}
              {isOpen && (
                <Suspense fallback={<p role="status" className="text-muted">Opening kept copy…</p>}>
                  <KeptCopyView key={`${portalKey}:${documentId}:${check.checkId}`} documentId={documentId} portalKey={portalKey} checkId={check.checkId} />
                </Suspense>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
