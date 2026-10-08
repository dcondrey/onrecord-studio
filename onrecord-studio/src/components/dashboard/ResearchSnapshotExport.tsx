import { useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { openHelp } from '../help/helpEvents';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
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
    <section aria-labelledby="research-snapshot-title" className="space-y-3 text-body text-muted">
      <h4 id="research-snapshot-title" className="font-medium text-fg">Research snapshot for outside tools</h4>
      <p>
        A read-only DuckDB file of requests, documents, deadlines and findings, with a signed manifest of the corpus state.{' '}
        <Button variant="link" onClick={() => openHelp('research-snapshot')}>How to cite it</Button>
      </p>
      <Button disabled={busy} onClick={() => void run()}>{busy ? 'Writing…' : 'Export research snapshot'}</Button>
      {error && <InlineError summary="The research snapshot couldn’t be written." detail={error} />}
      {result && (
        <dl role="status" className="space-y-1 break-all text-caption">
          <div><dt className="inline font-medium text-fg-secondary">File </dt><dd className="inline font-mono text-code">{result.path}</dd></div>
          <div><dt className="inline font-medium text-fg-secondary">Manifest </dt><dd className="inline font-mono text-code">{result.manifestPath}</dd></div>
          <div><dt className="inline font-medium text-fg-secondary">Corpus state </dt><dd className="inline font-mono text-code">{result.manifest.corpusStateDigest}</dd></div>
          <div><dt className="inline font-medium text-fg-secondary">Rows </dt><dd className="inline tabular-nums">{rows.toLocaleString()} across {Object.keys(result.manifest.tableRows).length} tables</dd></div>
        </dl>
      )}
    </section>
  );
}
