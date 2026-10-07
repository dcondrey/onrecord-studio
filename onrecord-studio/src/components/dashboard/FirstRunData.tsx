import { useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { CityDataset } from '../../types';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';

interface CoreTable {
  key: string;
  dataset: CityDataset;
  year: number | null;
  label: string;
}

type TableStatus =
  | { state: 'waiting' }
  | { state: 'fetching' }
  | { state: 'done'; rows: number; rejected: number }
  | { state: 'failed'; error: string };

/** The fetch refuses a year past the backend's UTC year, which local time can reach near New Year. */
function coreTables(year = new Date().getUTCFullYear()): CoreTable[] {
  // The open copy goes first: a closed-year file counts toward a fiscal year only when it is no
  // older than the open copy it completes.
  return [
    { key: 'budget', dataset: 'OPERATING_BUDGET', year: null, label: 'Operating budget, every published fiscal year' },
    { key: 'actuals', dataset: 'OPERATING_ACTUALS', year: null, label: 'Operating actuals, every published fiscal year' },
    { key: 'open', dataset: 'GET_IT_DONE_OPEN', year: null, label: 'Get It Done reports open today, every service' },
    { key: `closed-${year - 1}`, dataset: 'GET_IT_DONE_ENCAMPMENTS', year: year - 1, label: `Get It Done Encampment reports closed in ${year - 1}` },
    { key: `closed-${year}`, dataset: 'GET_IT_DONE_ENCAMPMENTS', year, label: `Get It Done Encampment reports closed in ${year} so far` },
    { key: `calls-${year - 1}`, dataset: 'CALLS_FOR_SERVICE', year: year - 1, label: `SDPD calls for service, ${year - 1}` },
    { key: `calls-${year}`, dataset: 'CALLS_FOR_SERVICE', year, label: `SDPD calls for service, ${year} so far` },
  ];
}

/**
 * Offered once, on a library with no imported tables. It stays up for the whole run even after the
 * first table makes the library non-empty, so every table's outcome is visible.
 */
export default function FirstRunData() {
  const { controller, state } = useAutomaticDiscovery();
  const settled = useAppStore((store) => store.firstRunDataSettled);
  const gate = useLatestRequestGate();
  const [tables] = useState(coreTables);
  const [phase, setPhase] = useState<'offer' | 'running' | 'finished' | 'closed'>('offer');
  const [statuses, setStatuses] = useState<Record<string, TableStatus>>({});

  // A schema that has not loaded, or failed to load, is not an empty library.
  const empty = state?.schema != null && state.schema.datasets.length === 0;
  if (phase === 'closed' || (phase === 'offer' && (settled || !empty))) return null;

  const mark = (key: string, status: TableStatus) => setStatuses((current) => ({ ...current, [key]: status }));

  const start = async () => {
    const request = gate.begin();
    useAppStore.getState().settleFirstRunData();
    setPhase('running');
    setStatuses(Object.fromEntries(tables.map((table) => [table.key, { state: 'waiting' } as const])));
    const release = controller?.hold();
    let failed = 0;
    let imported = 0;
    try {
      for (const table of tables) {
        mark(table.key, { state: 'fetching' });
        const outcome: TableStatus = await invokeCommand('fetch_city_dataset', { dataset: table.dataset, year: table.year }).then(
          (result) => ({ state: 'done', rows: result.importedRows, rejected: result.rejectedRows }),
          (reason) => ({ state: 'failed', error: formatNativeError(reason) }),
        );
        // Counted before the gate: a table that landed after unmount still needs discovery to see it.
        if (outcome.state === 'done') imported += 1;
        if (!gate.isCurrent(request)) return;
        if (outcome.state === 'failed') failed += 1;
        mark(table.key, outcome);
      }
      if (failed === 0) useAppStore.getState().setWorkspaceView('analysis');
      else setPhase('finished');
    } finally {
      release?.();
      if (imported > 0) controller?.refresh();
    }
  };

  const skip = () => {
    useAppStore.getState().settleFirstRunData();
    setPhase('closed');
  };

  const anyImported = Object.values(statuses).some((status) => status.state === 'done');

  return (
    <section aria-label="Get San Diego's core public data" className="app-panel space-y-3 p-5">
      <div>
        <h3 className="text-sm font-semibold text-fg">Get San Diego&apos;s core public data</h3>
        <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
          Your library has no tables yet. These seven tables show what the City budgeted and spent, what residents reported
          through Get It Done, and what SDPD was called to, so the leads under What stands out have something to read.
        </p>
        <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
          They come from the City of San Diego open data portal (data.sandiego.gov) and are retrieved now. The City publishes
          them in the public domain under the Open Data Commons Public Domain Dedication and License (ODC-PDDL), for
          information only and without warranty. The current year&apos;s files hold the year so far. Shelter beds are not a City
          table; What stands out offers them separately. The files are large, and the download can take several minutes.
        </p>
      </div>
      <ul aria-label="Core tables" className="space-y-1 text-label leading-4">
        {tables.map((table) => {
          const status = statuses[table.key];
          return (
            <li key={table.key} className="flex flex-wrap gap-x-2">
              <span className="text-fg-secondary">{table.label}</span>
              {status?.state === 'waiting' && <span className="text-muted">Waiting</span>}
              {status?.state === 'fetching' && <span role="status" className="text-accent">Fetching…</span>}
              {status?.state === 'done' && (
                <span className="text-success">
                  {status.rows.toLocaleString('en-US')} rows imported{status.rejected > 0 ? `, ${status.rejected.toLocaleString('en-US')} rejected` : ''}
                </span>
              )}
              {status?.state === 'failed' && <span role="alert" className="text-warning">Not imported: {status.error}</span>}
            </li>
          );
        })}
      </ul>
      {phase === 'offer' && (
        <div className="flex flex-wrap gap-2">
          <button type="button" className="analysis-button" onClick={() => void start()}>Get the tables</button>
          <button type="button" className="secondary-action focus-ring rounded-xl border px-3.5 py-2 text-label text-fg-secondary" onClick={skip}>Skip</button>
        </div>
      )}
      {phase === 'finished' && (
        <div className="flex flex-wrap gap-2">
          {anyImported && (
            <button type="button" className="analysis-button" onClick={() => useAppStore.getState().setWorkspaceView('analysis')}>
              See what stands out
            </button>
          )}
          <button type="button" className="secondary-action focus-ring rounded-xl border px-3.5 py-2 text-label text-fg-secondary" onClick={() => setPhase('closed')}>Close</button>
        </div>
      )}
    </section>
  );
}
