import { useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { openHelp } from '../help/helpEvents';
import type { ResearchSnapshotResult } from '../../types';

/** Writes a read-only DuckDB snapshot of the evidence projection plus its signed manifest. */
export default function ResearchSnapshotExport() {
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ResearchSnapshotResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(await invokeCommand('export_research_snapshot', {}));
    } catch (failure) {
      setError(formatNativeError(failure));
    } finally {
      setBusy(false);
    }
  };

  const rows = result ? Object.values(result.manifest.tableRows).reduce((sum, count) => sum + count, 0) : 0;
  return (
    <details className="mt-4 text-label">
      <summary className="cursor-pointer text-fg-secondary">Research snapshot for outside tools</summary>
      <div className="mt-3 space-y-3 text-muted">
        <p>
          Writes a read-only DuckDB file of requests, documents, deadline evaluations and findings (digests, never document text) that Python, R or the DuckDB CLI can query while the app stays open, with a signed manifest naming the exact corpus state it came from.{' '}
          <button type="button" className="underline" onClick={() => openHelp('research-snapshot')}>How to cite it</button>
        </p>
        <button type="button" className="analysis-button" disabled={busy} onClick={() => void run()}>{busy ? 'Writing…' : 'Export research snapshot'}</button>
        {error && <p role="alert" className="text-warning">{error}</p>}
        {result && (
          <dl role="status" className="space-y-1 break-all">
            <div><dt className="inline font-semibold text-fg-secondary">File </dt><dd className="inline">{result.path}</dd></div>
            <div><dt className="inline font-semibold text-fg-secondary">Manifest </dt><dd className="inline">{result.manifestPath}</dd></div>
            <div><dt className="inline font-semibold text-fg-secondary">Corpus state </dt><dd className="inline font-mono">{result.manifest.corpusStateDigest}</dd></div>
            <div><dt className="inline font-semibold text-fg-secondary">Rows </dt><dd className="inline">{rows.toLocaleString()} across {Object.keys(result.manifest.tableRows).length} tables</dd></div>
          </dl>
        )}
      </div>
    </details>
  );
}
