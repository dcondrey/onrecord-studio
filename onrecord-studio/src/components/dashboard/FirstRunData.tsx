import { useAppStore } from '../../store/useAppStore';
import { firstRunTableKey, firstRunTables, useFirstRunDataStore, type FirstRunTable } from '../../store/useFirstRunDataStore';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';
import Button from '../ui/Button';

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
  // Before a run there is no per-table status to show, so the closed Encampment years read as one row.
  const rows = running || runTables.length || partial ? tables.map((table) => ({ key: firstRunTableKey(table), table, label: describe(table, currentYear).label })) : offerRows(tables);

  return (
    <section aria-label="Get San Diego's core public data" className="app-panel space-y-4 p-5">
      <div>
        <h3 className="text-heading text-fg">
          {partial ? 'San Diego’s core public data is incomplete' : 'Get San Diego’s core public data'}
        </h3>
        <p className="mt-1 max-w-prose text-body text-muted">
          {partial
            ? `${unfetched.length} of these tables ${unfetched.length === 1 ? 'was' : 'were'} not imported, so the leads that read ${unfetched.length === 1 ? 'it' : 'them'} have less to go on. Fetch the missing ones now, or later from Analysis.`
            : `${numberWord(tables.length)} public tables from data.sandiego.gov: budgets, Get It Done reports and SDPD calls. Large download; it continues in the background.`}
        </p>
      </div>
      <ul aria-label="Core tables" className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-6 text-body">
        {rows.map(({ key, table, label }) => {
          const status = table ? statuses[firstRunTableKey(table)] : undefined;
          return (
            <li key={key} className="col-span-2 grid grid-cols-subgrid items-baseline border-b border-line py-1.5 last:border-b-0">
              <span className="text-fg-secondary">{label}</span>
              <span className="text-right text-caption tabular-nums">
                {status?.state === 'waiting' && <span className="text-muted">Waiting</span>}
                {status?.state === 'fetching' && <span role="status" className="text-fg-secondary">Fetching…</span>}
                {status?.state === 'done' && (
                  <span className="text-success">
                    {status.rows.toLocaleString('en-US')} rows imported{status.rejected > 0 ? `, ${status.rejected.toLocaleString('en-US')} rejected` : ''}
                  </span>
                )}
                {status?.state === 'failed' && <span role="alert" className="text-danger">Not imported: {status.error}</span>}
                {table && !status && partial && <span role="alert" className="text-warning">Not fetched: the download stopped before reaching it</span>}
              </span>
            </li>
          );
        })}
      </ul>
      <details className="text-caption text-muted">
        <summary className="focus-ring w-fit cursor-pointer rounded hover:text-fg">Details</summary>
        <div className="mt-2 max-w-prose space-y-2">
          <p>
            The tables come from the City of San Diego open data portal (data.sandiego.gov) and are retrieved now. The City
            publishes them in the public domain under the Open Data Commons Public Domain Dedication and License (ODC-PDDL), for
            information only and without warranty. The current year&apos;s files hold the year so far.
          </p>
          <ul className="list-disc space-y-0.5 pl-4">
            {tables.map((table) => {
              const { label, leads } = describe(table, currentYear);
              return <li key={firstRunTableKey(table)}>{label}: {leads}.</li>;
            })}
          </ul>
          <p>
            Shelter beds are not fetched here: UCSD&apos;s Homelessness Hub publishes the emergency shelter inventory with no
            license stated. Until you fetch it, {CAPACITY} shows under What stands out as missing data, with a button to fetch
            it. The closed Encampment years listed are enough for that comparison to run as soon as the inventory is imported;
            its last count is January 2024.
          </p>
        </div>
      </details>
      {!running && (
        <div className="flex flex-wrap gap-2">
          <Button variant="primary" onClick={() => void run(controller)}>
            {partial ? 'Get the missing tables' : 'Get the tables'}
          </Button>
          {partial && anyImported && (
            <Button onClick={() => useAppStore.getState().setWorkspaceView('analysis')}>See what stands out</Button>
          )}
          <Button variant="ghost" onClick={dismiss}>{partial ? 'Close' : 'Not now'}</Button>
        </div>
      )}
    </section>
  );
}

const NUMBER_WORDS = ['No', 'One', 'Two', 'Three', 'Four', 'Five', 'Six', 'Seven', 'Eight', 'Nine', 'Ten', 'Eleven', 'Twelve'];
function numberWord(count: number) {
  return NUMBER_WORDS[count] ?? count.toLocaleString('en-US');
}

/** The offer lists the closed Encampment years as one span instead of a row per year. */
function offerRows(tables: FirstRunTable[]): { key: string; table: FirstRunTable | null; label: string }[] {
  const currentYear = new Date().getUTCFullYear();
  const closedYears = tables.filter((table) => table.dataset === 'GET_IT_DONE_ENCAMPMENTS' && table.year !== null).map((table) => table.year as number);
  const rows: { key: string; table: FirstRunTable | null; label: string }[] = [];
  for (const table of tables) {
    if (table.dataset === 'GET_IT_DONE_ENCAMPMENTS' && closedYears.length > 1) {
      if (rows.some((row) => row.key === 'encampment-years')) continue;
      rows.push({ key: 'encampment-years', table: null, label: `Get It Done encampment reports closed, ${Math.min(...closedYears)}–${Math.max(...closedYears)}` });
      continue;
    }
    rows.push({ key: firstRunTableKey(table), table, label: describe(table, currentYear).label });
  }
  return rows;
}
