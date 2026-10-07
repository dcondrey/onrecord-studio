import { useAppStore } from '../../store/useAppStore';
import { firstRunTableKey, firstRunTables, useFirstRunDataStore, type FirstRunTable } from '../../store/useFirstRunDataStore';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';

const CAPACITY = 'Budget, Encampment reports and shelter beds by year';

function describe(table: FirstRunTable, currentYear: number): { label: string; leads: string } {
  const soFar = table.year !== null && table.year >= currentYear ? ' so far' : '';
  switch (table.dataset) {
    case 'OPERATING_BUDGET':
      return { label: 'Operating budget, every published fiscal year', leads: `Adopted and actual spending; ${CAPACITY}; the department beside a service-report lead` };
    case 'OPERATING_ACTUALS':
      return { label: 'Operating actuals, every published fiscal year', leads: 'Adopted and actual spending, settlement accounts' };
    case 'GET_IT_DONE_OPEN':
      return { label: 'Get It Done reports open today, every service', leads: `Service-report leads; ${CAPACITY}` };
    case 'GET_IT_DONE_ENCAMPMENTS':
      return { label: `Get It Done Encampment reports closed in ${table.year}${soFar}`, leads: `${CAPACITY}; service-report leads` };
    case 'CALLS_FOR_SERVICE':
      return { label: `SDPD calls for service, ${table.year}${soFar}`, leads: 'How SLEEPER, welfare and trespass calls ended' };
    default:
      return { label: table.dataset, leads: '' };
  }
}

/**
 * Offered once, on a library with no imported tables. The run belongs to the store, so this card
 * only shows it: it stays up while the run lasts and while any table is left unimported.
 */
export default function FirstRunData() {
  const { controller, state } = useAutomaticDiscovery();
  const settled = useAppStore((store) => store.firstRunDataSettled);
  const { running, tables: runTables, statuses, unfetched, run, dismiss } = useFirstRunDataStore();

  // A schema that has not loaded, or failed to load, is not an empty library.
  const empty = state?.schema != null && state.schema.datasets.length === 0;
  const partial = !running && unfetched.length > 0;
  if (!running && !partial && (settled || !empty)) return null;

  const currentYear = new Date().getUTCFullYear();
  const tables = running || runTables.length ? runTables : partial ? unfetched : firstRunTables();
  const anyImported = Object.values(statuses).some((status) => status.state === 'done');

  return (
    <section aria-label="Get San Diego's core public data" className="app-panel space-y-3 p-5">
      <div>
        <h3 className="text-sm font-semibold text-fg">
          {partial ? 'San Diego’s core public data is incomplete' : 'Get San Diego’s core public data'}
        </h3>
        {partial ? (
          <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
            {unfetched.length} of these tables {unfetched.length === 1 ? 'was' : 'were'} not imported, so the leads that read
            {unfetched.length === 1 ? ' it' : ' them'} have less to go on. You can fetch the missing ones now, or later from Analysis.
          </p>
        ) : (
          <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
            Your library has no tables yet. These {tables.length} tables show what the City budgeted and spent, what residents
            reported through Get It Done, and what SDPD was called to, so the leads under What stands out have something to read.
            Each line names the leads it feeds.
          </p>
        )}
        <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
          They come from the City of San Diego open data portal (data.sandiego.gov) and are retrieved now. The City publishes
          them in the public domain under the Open Data Commons Public Domain Dedication and License (ODC-PDDL), for
          information only and without warranty. The current year&apos;s files hold the year so far. The files are large, and
          the download can take several minutes; it continues if you leave the overview.
        </p>
        <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
          Shelter beds are not fetched here: UCSD&apos;s Homelessness Hub publishes the emergency shelter inventory with no
          licence stated. Until you fetch it, {CAPACITY} shows under What stands out as missing data, the emergency shelter
          inventory, with a button to fetch it. The closed Encampment years listed are enough for that comparison to run as soon
          as the inventory is imported; its last count is January 2024.
        </p>
      </div>
      <ul aria-label="Core tables" className="space-y-1 text-label leading-4">
        {tables.map((table) => {
          const key = firstRunTableKey(table);
          const status = statuses[key];
          const { label, leads } = describe(table, currentYear);
          return (
            <li key={key} className="flex flex-wrap gap-x-2">
              <span className="text-fg-secondary">{label}</span>
              <span className="text-muted">Feeds: {leads}.</span>
              {status?.state === 'waiting' && <span className="text-muted">Waiting</span>}
              {status?.state === 'fetching' && <span role="status" className="text-accent">Fetching…</span>}
              {status?.state === 'done' && (
                <span className="text-success">
                  {status.rows.toLocaleString('en-US')} rows imported{status.rejected > 0 ? `, ${status.rejected.toLocaleString('en-US')} rejected` : ''}
                </span>
              )}
              {status?.state === 'failed' && <span role="alert" className="text-warning">Not imported: {status.error}</span>}
              {!status && partial && <span role="alert" className="text-warning">Not fetched: the download stopped before reaching it</span>}
            </li>
          );
        })}
      </ul>
      {!running && (
        <div className="flex flex-wrap gap-2">
          <button type="button" className="analysis-button" onClick={() => void run(controller)}>
            {partial ? 'Get the missing tables' : 'Get the tables'}
          </button>
          {partial && anyImported && (
            <button type="button" className="secondary-action focus-ring rounded-xl border px-3.5 py-2 text-label text-fg-secondary" onClick={() => useAppStore.getState().setWorkspaceView('analysis')}>
              See what stands out
            </button>
          )}
          <button type="button" className="secondary-action focus-ring rounded-xl border px-3.5 py-2 text-label text-fg-secondary" onClick={dismiss}>
            {partial ? 'Close' : 'Skip'}
          </button>
        </div>
      )}
    </section>
  );
}
