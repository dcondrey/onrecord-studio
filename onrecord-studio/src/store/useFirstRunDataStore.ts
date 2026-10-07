import { create } from 'zustand';
import { formatNativeError, invokeCommand } from '../ipc';
import type { CityDataset } from '../types';
import { EARLIEST_USEFUL_CLOSED_YEAR } from '../utils/capacityLeads';
import type { DiscoveryController } from '../utils/discoveryController';
import { useAppStore } from './useAppStore';

export interface FirstRunTable {
  dataset: CityDataset;
  year: number | null;
}

export type FirstRunTableStatus =
  | { state: 'waiting' }
  | { state: 'fetching' }
  | { state: 'done'; rows: number; rejected: number }
  | { state: 'failed'; error: string };

export const firstRunTableKey = (table: FirstRunTable) => `${table.dataset}:${table.year ?? 'all'}`;

/** The UCSD layer the capacity lead reads for beds is `HIC_ES_2014_2024`: its last count is January
 * 2024, set beside fiscal 2024, so no later fiscal year can be compared. */
const LAST_INVENTORY_FISCAL_YEAR = 2024;

/**
 * The capacity lead compares at least two fiscal years that have beds and a full count of reports
 * filed. Closed reports are published by close year, so a fiscal year is counted only with the
 * open copy and every closed file from the calendar year it began (see `encampmentReportCoverage`).
 * The last two fiscal years with beds and an ended year therefore need every closed file from the
 * year before the earlier one began. Dates are UTC because the fetch refuses a year past the
 * backend's UTC year, which local time can reach near New Year.
 */
export function firstRunTables(now = new Date()): FirstRunTable[] {
  const year = now.getUTCFullYear();
  const lastEndedFiscalYear = now.toISOString().slice(5, 10) > '07-01' ? year : year - 1;
  const latestCompared = Math.min(lastEndedFiscalYear, LAST_INVENTORY_FISCAL_YEAR);
  const closed: FirstRunTable[] = [];
  for (let closedYear = Math.max(latestCompared - 2, EARLIEST_USEFUL_CLOSED_YEAR); closedYear <= year; closedYear += 1) {
    closed.push({ dataset: 'GET_IT_DONE_ENCAMPMENTS', year: closedYear });
  }
  // The open copy goes first: a current-year closed file counts toward a fiscal year only when it
  // is no older than the open copy it completes.
  return [
    { dataset: 'OPERATING_BUDGET', year: null },
    { dataset: 'OPERATING_ACTUALS', year: null },
    { dataset: 'GET_IT_DONE_OPEN', year: null },
    ...closed,
    { dataset: 'CALLS_FOR_SERVICE', year: year - 1 },
    { dataset: 'CALLS_FOR_SERVICE', year },
  ];
}

/**
 * The tables a retry fetches. A new open copy makes every current-year closed file already imported
 * older than it, so the current-year closed file is fetched again right after it (`firstRunTables`).
 */
export function retryTables(unfetched: FirstRunTable[], now = new Date()): FirstRunTable[] {
  if (!unfetched.some((table) => table.dataset === 'GET_IT_DONE_OPEN')) return unfetched;
  const closed: FirstRunTable = { dataset: 'GET_IT_DONE_ENCAMPMENTS', year: now.getUTCFullYear() };
  return unfetched
    .filter((table) => firstRunTableKey(table) !== firstRunTableKey(closed))
    .flatMap((table) => table.dataset === 'GET_IT_DONE_OPEN' ? [table, closed] : [table]);
}

const UNFETCHED_STORAGE_KEY = 'onrecord-first-run-unfetched';
const FETCHED_DATASETS = new Set<string>(['OPERATING_BUDGET', 'OPERATING_ACTUALS', 'GET_IT_DONE_OPEN', 'GET_IT_DONE_ENCAMPMENTS', 'CALLS_FOR_SERVICE']);

function readUnfetched(): FirstRunTable[] {
  if (typeof window === 'undefined') return [];
  try {
    const parsed: unknown = JSON.parse(window.localStorage.getItem(UNFETCHED_STORAGE_KEY) ?? '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((entry): entry is FirstRunTable => typeof entry === 'object' && entry !== null
      && FETCHED_DATASETS.has((entry as FirstRunTable).dataset)
      && ((entry as FirstRunTable).year === null || Number.isInteger((entry as FirstRunTable).year)));
  } catch {
    return [];
  }
}

function writeUnfetched(tables: FirstRunTable[]) {
  try {
    if (tables.length) window.localStorage.setItem(UNFETCHED_STORAGE_KEY, JSON.stringify(tables));
    else window.localStorage.removeItem(UNFETCHED_STORAGE_KEY);
  } catch {
    // Storage unavailable: the record lasts for this session only.
  }
}

interface FirstRunDataState {
  running: boolean;
  /** The tables of the current or last run in this session. */
  tables: FirstRunTable[];
  statuses: Record<string, FirstRunTableStatus>;
  /** Tables a run set out to fetch and did not import: failed, or not reached because the app
   * closed. Persisted from the start of a run, so an interrupted run is still reported. */
  unfetched: FirstRunTable[];
  /** Runs the unfetched tables when a run left some, otherwise every core table. */
  run: (controller: Pick<DiscoveryController, 'reserve' | 'refresh'> | null) => Promise<void>;
  /** Settles the offer and drops the partial record once the operator has seen it. */
  dismiss: () => void;
}

/**
 * The run lives here, not in the card: leaving the overview unmounts the card, and a run tied to it
 * would stop there and silently drop the tables not yet started.
 */
export const useFirstRunDataStore = create<FirstRunDataState>((set, get) => ({
  running: false,
  tables: [],
  statuses: {},
  unfetched: readUnfetched(),
  run: async (controller) => {
    if (get().running) return;
    const tables = get().unfetched.length ? retryTables(get().unfetched) : firstRunTables();
    const remaining = new Set(tables.map(firstRunTableKey));
    const unfetched = () => tables.filter((table) => remaining.has(firstRunTableKey(table)));
    writeUnfetched(tables);
    set({ running: true, tables, unfetched: tables, statuses: Object.fromEntries(tables.map((table) => [firstRunTableKey(table), { state: 'waiting' } as const])) });
    const mark = (table: FirstRunTable, status: FirstRunTableStatus) => set((state) => ({ statuses: { ...state.statuses, [firstRunTableKey(table)]: status } }));
    // Only these tables are kept from discovery: it profiles and records leads from each one imported.
    const release = controller?.reserve(tables);
    const app = useAppStore.getState();
    try {
      for (const table of tables) {
        mark(table, { state: 'fetching' });
        const outcome: FirstRunTableStatus = await invokeCommand('fetch_city_dataset', { dataset: table.dataset, year: table.year }).then(
          (result) => ({ state: 'done', rows: result.importedRows, rejected: result.rejectedRows }),
          (reason) => ({ state: 'failed', error: formatNativeError(reason) }),
        );
        if (outcome.state === 'done') {
          remaining.delete(firstRunTableKey(table));
          writeUnfetched(unfetched());
          controller?.refresh();
        } else {
          app.appendSyncLog(`First-run data: ${table.dataset}${table.year === null ? '' : ` ${table.year}`} not imported. ${outcome.error}`);
        }
        mark(table, outcome);
      }
    } finally {
      release?.();
      const left = unfetched();
      if (left.length < tables.length) controller?.refresh();
      set({ running: false, unfetched: left });
      useAppStore.getState().settleFirstRunData();
      // A finished run must not take over a view the operator moved to while it ran.
      if (left.length === 0 && useAppStore.getState().workspaceView === 'overview') useAppStore.getState().setWorkspaceView('analysis');
    }
  },
  dismiss: () => {
    if (get().running) return;
    writeUnfetched([]);
    set({ unfetched: [], tables: [], statuses: {} });
    useAppStore.getState().settleFirstRunData();
  },
}));
