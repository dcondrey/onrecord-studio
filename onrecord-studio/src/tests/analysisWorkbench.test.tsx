// @vitest-environment jsdom
//
// AnalysisWorkbench writes `sql`/`sqlResult` from two independent paths: the automatic-discovery
// inspection effect, and the manual Run / budget-comparison buttons. Both call `run_analytical_sql`
// with no ordering guard between them prior to the fix, so a slower automatic query can overwrite
// a faster manual result the user asked for afterward. Both paths must go through the same
// `useLatestRequestGate` so the last request the user actually made wins regardless of resolution
// order.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { SqlQueryResult } from '../types';

const resolvers = new Map<string, (result: SqlQueryResult) => void>();
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));

const upkeep = {
  policy: { enabled: true, autoImport: true, autoRefresh: true, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 10 * 1024 ** 3 },
  effectiveAt: '2026-09-14T00:00:00Z',
  retainedFileBytes: 1048576,
  referencedDocuments: 0,
  reclaimedFileBytes: 0,
  events: [],
};

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke, formatNativeError: (error: unknown) => (error instanceof Error ? error.message : String(error)) }));
vi.mock('../store/useAppStore', () => ({
  useAppStore: Object.assign((selector: (state: { evidenceRefreshRequests: number }) => unknown) =>
    selector({ evidenceRefreshRequests: 0 }), {
    getState: () => ({
      setActivePortal: vi.fn(),
      setSearchFilters: vi.fn(),
      setSelectedRequest: vi.fn(),
      setWorkspaceView: vi.fn(),
      setPortalUrlDraft: vi.fn(),
    }),
  }),
}));

function result(column: string): SqlQueryResult {
  return { columns: [column], rows: [['value']], rowCount: 1, truncated: false, elapsedMs: 1, provenance: [] };
}

import AnalysisWorkbench from '../components/dashboard/AnalysisWorkbench';
import {
  AutomaticDiscoveryProvider,
  useDiscoveryController,
} from '../components/dashboard/AutomaticDiscoveryProvider';
import type { DiscoveryController } from '../utils/discoveryController';
import { readBudgetComparison } from '../utils/budgetComparison';
import DifferenceDots from '../components/charts/figures/DifferenceDots';

let capturedController: DiscoveryController | null = null;
function ControllerCapture() {
  capturedController = useDiscoveryController();
  return null;
}

function Harness() {
  return (
    <AutomaticDiscoveryProvider>
      <ControllerCapture />
      <AnalysisWorkbench />
    </AutomaticDiscoveryProvider>
  );
}

afterEach(() => {
  cleanup();
  resolvers.clear();
  capturedController = null;
  vi.clearAllMocks();
});

describe('AnalysisWorkbench SQL result gating', () => {
  it('keeps the manual run result even when a slower automatic inspection query resolves after it', async () => {
    bridge.invoke.mockImplementation(async (name: string, args?: { sql?: string }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [], columns: [], templates: [] };
        case 'list_analytical_import_candidates':
          return [];
        case 'query_corpus_patterns':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        case 'run_analytical_sql':
          return new Promise((resolve) => resolvers.set(String(args?.sql ?? ''), resolve));
        default:
          return null;
      }
    });

    render(<Harness />);
    await waitFor(() => expect(capturedController).not.toBeNull());

    // Automatic discovery starts inspecting a slow query.
    act(() => {
      capturedController!.inspect('SELECT slow_query');
    });
    await waitFor(() =>
      expect((screen.getByLabelText('Analytical SQL') as HTMLTextAreaElement).value).toBe('SELECT slow_query'),
    );
    await waitFor(() => expect(resolvers.has('SELECT slow_query')).toBe(true));

    // The user edits the SQL and runs it manually before the automatic query resolves.
    fireEvent.change(screen.getByLabelText('Analytical SQL'), { target: { value: 'SELECT fast_query' } });
    fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    await waitFor(() => expect(resolvers.has('SELECT fast_query')).toBe(true));

    // Manual result resolves first (it's fast).
    act(() => {
      resolvers.get('SELECT fast_query')!(result('manual_col'));
    });
    await screen.findByText('manual_col');

    // The automatic query resolves afterward. Without the gate, this silently overwrites the
    // manual result the user is now looking at.
    await act(async () => {
      resolvers.get('SELECT slow_query')!(result('auto_col'));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.queryByText('auto_col')).toBeNull();
    expect(screen.getByText('manual_col')).toBeTruthy();
  });

  it('discards a stale automatic result after the user switches to a different query without running it', async () => {
    const template = {
      templateId: 't1',
      title: 'Regional preset',
      description: 'Loads a canned regional query',
      sql: 'SELECT * FROM analysis_regional_data LIMIT 50',
      denominator: '',
    };
    bridge.invoke.mockImplementation(async (name: string, args?: { sql?: string }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [], columns: [], templates: [template] };
        case 'list_analytical_import_candidates':
          return [];
        case 'query_corpus_patterns':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        case 'run_analytical_sql':
          return new Promise((resolve) => resolvers.set(String(args?.sql ?? ''), resolve));
        default:
          return null;
      }
    });

    render(<Harness />);
    await waitFor(() => expect(capturedController).not.toBeNull());

    // Automatic discovery starts inspecting a slow query.
    act(() => {
      capturedController!.inspect('SELECT slow_query');
    });
    await waitFor(() =>
      expect((screen.getByLabelText('Analytical SQL') as HTMLTextAreaElement).value).toBe('SELECT slow_query'),
    );
    await waitFor(() => expect(resolvers.has('SELECT slow_query')).toBe(true));

    // The user picks a different query from the template list without ever clicking Run, so
    // nothing supersedes the gate through `runGatedSql`.
    fireEvent.click(await screen.findByRole('button', { name: /Regional preset/ }));
    await waitFor(() =>
      expect((screen.getByLabelText('Analytical SQL') as HTMLTextAreaElement).value).toBe(template.sql),
    );

    // The automatic query resolves afterward, for SQL the editor no longer shows.
    await act(async () => {
      resolvers.get('SELECT slow_query')!(result('auto_col'));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.queryByText('auto_col')).toBeNull();
  });

  it('discards a stale automatic result after the user edits the SQL by hand without running it', async () => {
    bridge.invoke.mockImplementation(async (name: string, args?: { sql?: string }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [], columns: [], templates: [] };
        case 'list_analytical_import_candidates':
          return [];
        case 'query_corpus_patterns':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        case 'run_analytical_sql':
          return new Promise((resolve) => resolvers.set(String(args?.sql ?? ''), resolve));
        default:
          return null;
      }
    });

    render(<Harness />);
    await waitFor(() => expect(capturedController).not.toBeNull());

    act(() => {
      capturedController!.inspect('SELECT slow_query');
    });
    await waitFor(() =>
      expect((screen.getByLabelText('Analytical SQL') as HTMLTextAreaElement).value).toBe('SELECT slow_query'),
    );
    await waitFor(() => expect(resolvers.has('SELECT slow_query')).toBe(true));

    // The user types over the query by hand and never clicks Run.
    fireEvent.change(screen.getByLabelText('Analytical SQL'), { target: { value: 'SELECT edited_by_hand' } });

    await act(async () => {
      resolvers.get('SELECT slow_query')!(result('auto_col'));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.queryByText('auto_col')).toBeNull();
  });

  it('discards a stale automatic result after the user picks a dataset preset without running it', async () => {
    const dataset = {
      snapshotId: 'snap-1',
      datasetKind: 'REGIONAL_DATA' as const,
      displayName: 'Regional dataset fixture',
      tableName: 'analysis_regional_data_fixture',
      importedRows: 10,
      rejectedRows: 0,
      sourceUri: null,
      sourceSha256: 'a'.repeat(64),
      retrievedAt: '2026-09-01T00:00:00Z',
      observationPeriodStart: null,
      observationPeriodEnd: null,
      partialDataset: false,
      validationSummary: 'ok',
      unconvertedByColumn: {},
    };
    bridge.invoke.mockImplementation(async (name: string, args?: { sql?: string }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [dataset], columns: [], templates: [] };
        case 'list_analytical_import_candidates':
          return [];
        case 'query_corpus_patterns':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        case 'run_analytical_sql':
          return new Promise((resolve) => resolvers.set(String(args?.sql ?? ''), resolve));
        default:
          return null;
      }
    });

    render(<Harness />);
    await waitFor(() => expect(capturedController).not.toBeNull());

    act(() => {
      capturedController!.inspect('SELECT slow_query');
    });
    await waitFor(() =>
      expect((screen.getByLabelText('Analytical SQL') as HTMLTextAreaElement).value).toBe('SELECT slow_query'),
    );
    await waitFor(() => expect(resolvers.has('SELECT slow_query')).toBe(true));

    // The user picks the dataset preset without ever clicking Run.
    fireEvent.click(await screen.findByRole('button', { name: /Regional dataset fixture/ }));
    await waitFor(() =>
      expect((screen.getByLabelText('Analytical SQL') as HTMLTextAreaElement).value).toBe(
        'SELECT * FROM analysis_regional_data_fixture LIMIT 50',
      ),
    );

    await act(async () => {
      resolvers.get('SELECT slow_query')!(result('auto_col'));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.queryByText('auto_col')).toBeNull();
  });

  it('clears the loading indicator once a superseded automatic query settles, even though its result is discarded', async () => {
    const template = {
      templateId: 't1',
      title: 'Regional preset',
      description: 'Loads a canned regional query',
      sql: 'SELECT * FROM analysis_regional_data LIMIT 50',
      denominator: '',
    };
    bridge.invoke.mockImplementation(async (name: string, args?: { sql?: string }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [], columns: [], templates: [template] };
        case 'list_analytical_import_candidates':
          return [];
        case 'query_corpus_patterns':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        case 'run_analytical_sql':
          return new Promise((resolve) => resolvers.set(String(args?.sql ?? ''), resolve));
        default:
          return null;
      }
    });

    render(<Harness />);
    await waitFor(() => expect(capturedController).not.toBeNull());

    act(() => {
      capturedController!.inspect('SELECT slow_query');
    });
    await screen.findByText('Loading supporting records');
    await waitFor(() => expect(resolvers.has('SELECT slow_query')).toBe(true));

    // The user moves on to a different query before the automatic fetch settles. Its *result*
    // must be discarded (covered above), but its *own* loading indicator must still clear when
    // the fetch finishes — nothing else is going to turn it off.
    fireEvent.click(await screen.findByRole('button', { name: /Regional preset/ }));

    await act(async () => {
      resolvers.get('SELECT slow_query')!(result('auto_col'));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.queryByText('Loading supporting records')).toBeNull();
  });

  // A rerun that fails must not leave the previous query's table under the new text: that would
  // present a result for a query the editor no longer shows.
  it('clears the previous result the moment a new run starts, so a failed run shows none', async () => {
    const rejecters = new Map<string, (reason: Error) => void>();
    bridge.invoke.mockImplementation(async (name: string, args?: { sql?: string }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [], columns: [], templates: [] };
        case 'list_analytical_import_candidates':
        case 'query_corpus_patterns':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        case 'run_analytical_sql':
          return new Promise((resolve, reject) => {
            resolvers.set(String(args?.sql ?? ''), resolve);
            rejecters.set(String(args?.sql ?? ''), reject);
          });
        default:
          return null;
      }
    });

    render(<Harness />);
    const editor = await screen.findByLabelText('Analytical SQL');
    fireEvent.change(editor, { target: { value: 'SELECT first_query' } });
    fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    await waitFor(() => expect(resolvers.has('SELECT first_query')).toBe(true));
    act(() => resolvers.get('SELECT first_query')!(result('first_col')));
    await screen.findByText('first_col');

    fireEvent.change(editor, { target: { value: 'SELECT missing_table' } });
    fireEvent.click(screen.getByRole('button', { name: 'Run' }));
    await waitFor(() => expect(rejecters.has('SELECT missing_table')).toBe(true));
    await act(async () => {
      rejecters.get('SELECT missing_table')!(new Error('Catalog Error: missing_table does not exist'));
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.queryByText('first_col')).toBeNull();
  });
});

describe('AnalysisWorkbench City dataset fetch', () => {
  it('sends a year only for the tables the publisher splits by year', async () => {
    const sent: Record<string, unknown> = {};
    bridge.invoke.mockImplementation(async (name: string, args?: { dataset: string; year: unknown }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [], columns: [], templates: [] };
        case 'list_analytical_import_candidates':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        // Refused, so each fetch ends without the schema reload a successful import triggers.
        case 'fetch_city_dataset':
          sent[args!.dataset] = args!.year;
          throw new Error('offline');
        default:
          return null;
      }
    });
    // Without the discovery provider: the fetch's request does not depend on it, and its refresh
    // after every fetch is what made this loop slow.
    render(<AnalysisWorkbench />);
    const select = await screen.findByLabelText('City dataset') as HTMLSelectElement;
    const fetch = screen.getByRole('button', { name: 'Fetch and import' });
    await waitFor(() => expect(fetch).toHaveProperty('disabled', false));
    for (const option of Array.from(select.options)) {
      fireEvent.change(select, { target: { value: option.value } });
      fireEvent.click(fetch);
      await waitFor(() => expect(fetch).toHaveProperty('disabled', false));
      expect(sent).toHaveProperty(option.value);
    }
    // The fetch refuses a year for a table published as one file.
    const year = expect.any(Number);
    expect(sent).toEqual({ NIBRS_OFFENSES: year, CALLS_FOR_SERVICE: year, GET_IT_DONE_ENCAMPMENTS: year, GET_IT_DONE_OUTCOMES: year, GET_IT_DONE_OPEN: null, SCHOOLS: null, TRANSIT_STOPS: null, OPERATING_BUDGET: null,
      RIPA_STOPS: null, RIPA_STOP_REASONS: null, RIPA_STOP_RESULTS: null, RIPA_PROPERTY_SEIZURE_BASIS: null, SHELTER_INVENTORY: null, OPERATING_ACTUALS: null,
      CAPITAL_BUDGET: null, CAPITAL_ACTUALS: null, CITY_PROPERTIES: null, CITY_PARCELS: null, CITY_LEASES: null,
      CALL_TYPES: null, DISPOSITION_CODES: null, BEAT_CODES: null, BUDGET_ACCOUNTS: null, BUDGET_DEPARTMENTS: null, BUDGET_FUNDS: null,
      PARKING_CITATIONS_FIRST_HALF: year, PARKING_CITATIONS_SECOND_HALF: year, FIRE_EMS_INCIDENTS: year,
      APPROVALS_CREATED: year, APPROVALS_ISSUED: year, APPROVALS_CLOSED: year,
      TRAFFIC_COLLISIONS: null, TRAFFIC_COLLISION_PEOPLE: null, HATE_CRIMES: null });
  });
});

describe('AnalysisWorkbench hybrid search result gating', () => {
  it('keeps the latest search results even when an earlier search resolves after it', async () => {
    const hybridResolvers = new Map<string, (results: unknown[]) => void>();
    bridge.invoke.mockImplementation(async (name: string, args?: { query?: string }) => {
      switch (name) {
        case 'get_analytical_schema':
          return { datasets: [], columns: [], templates: [] };
        case 'list_analytical_import_candidates':
          return [];
        case 'query_corpus_patterns':
          return [];
        case 'get_data_maintenance_status':
          return upkeep;
        case 'hybrid_search':
          return new Promise((resolve) => hybridResolvers.set(String(args?.query ?? ''), resolve));
        default:
          return null;
      }
    });

    const hit = (stableId: string, fileName: string) => ({
      stableId,
      portalKey: 'portal-1',
      documentId: 'doc-1',
      agencyName: 'Agency',
      jurisdiction: 'California',
      retrievalStatus: 'RETRIEVED',
      semanticScore: null,
      fileName,
      title: null,
      excerpt: 'excerpt',
      characterStart: 0,
      characterEnd: 10,
    });

    render(<Harness />);
    await waitFor(() => expect(capturedController).not.toBeNull());

    const queryInput = screen.getByLabelText('Document evidence query');
    fireEvent.change(queryInput, { target: { value: 'first query' } });
    fireEvent.click(screen.getByRole('button', { name: 'Search' }));
    await waitFor(() => expect(hybridResolvers.has('first query')).toBe(true));

    fireEvent.change(queryInput, { target: { value: 'second query' } });
    fireEvent.click(screen.getByRole('button', { name: 'Search' }));
    await waitFor(() => expect(hybridResolvers.has('second query')).toBe(true));

    // The newer search resolves first.
    await act(async () => {
      hybridResolvers.get('second query')!([hit('fresh', 'FRESH_FILE')]);
      await Promise.resolve();
      await Promise.resolve();
    });
    await screen.findByText('FRESH_FILE');

    // The older search resolves afterward. Without its own gate, this would silently overwrite
    // the results for the query the user actually asked for last.
    await act(async () => {
      hybridResolvers.get('first query')!([hit('stale', 'STALE_FILE')]);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(screen.queryByText('STALE_FILE')).toBeNull();
    expect(screen.getByText('FRESH_FILE')).toBeTruthy();
  });
});

describe('budget comparison difference chart', () => {
  const columns = ['report_fy', 'fund_number', 'funds_center_number', 'account_number', 'budget_departments', 'actuals_departments', 'budget_amount', 'actuals_amount', 'budget_rows', 'actuals_rows', 'budget_missing_amounts', 'actuals_missing_amounts', 'comparison_status', 'actual_minus_budget', 'variance_percent'];
  const row = (account: string, budget: unknown, actuals: unknown, status: string, difference: unknown, percent: unknown) => ['25', '100000', '1914', account, 'Police', 'Police', budget, actuals, 1, 1, 0, 0, status, difference, percent];

  it('compares only matched lines, keeps decimal digits, and lists every other status as not compared', () => {
    const lines = readBudgetComparison({
      columns,
      rows: [
        row('512000', '1000000.10', '1236200.35', 'Matched', '236200.25', '23.62'),
        row('512001', '500000.00', '400000.00', 'Matched', '-100000.00', '-20.00'),
        // A difference cell beside a non-Matched status is not a comparison.
        row('512002', null, '90000.00', 'Missing budget baseline', '0', '0'),
        row('512003', '7000.00', null, 'Missing actuals', null, null),
      ],
    });
    if (!lines) throw new Error('comparison columns not recognised');
    expect(lines.filter((line) => line.difference !== null).map((line) => line.account)).toEqual(['512000', '512001']);
    expect(readBudgetComparison({ columns: ['a', 'b'], rows: [] })).toBeNull();

    const compared = lines.flatMap((line) => (line.difference && line.budget && line.actuals ? [{ key: line.key, label: line.label, context: line.departments, from: line.budget, to: line.actuals, difference: line.difference, percent: line.variancePercent }] : []));
    const uncompared = lines.filter((line) => line.difference === null).map((line) => ({ key: line.key, label: line.label, status: line.status }));
    render(<DifferenceDots title="Actuals minus budget" sources={{ tables: ['analysis_budget', 'analysis_actuals'] }} compared={compared} uncompared={uncompared} fromLabel="budget" toLabel="actual" truncated={false} />);

    expect(screen.getByText(/exceeds its budget amount by the most: \+\$236K \(\+23\.62%\)/)).toBeTruthy();
    expect(screen.getAllByRole('img', { name: /a difference of/ })).toHaveLength(2);
    expect(screen.getByRole('img', { name: /100000 \/ 1914 \/ 512000: actual \$1,236,200\.35 against budget \$1,000,000\.10, a difference of \+\$236,200\.25/ })).toBeTruthy();
    expect(screen.getByLabelText('Not compared').textContent).toMatch(/Missing actuals 1 · Missing budget baseline 1/);
    expect(screen.queryByRole('img', { name: /512002/ })).toBeNull();
    cleanup();

    // A result cut at the row limit is a subset: no line is called the largest, and the plot says so.
    render(<DifferenceDots title="Actuals minus budget" sources={{ tables: ['analysis_budget', 'analysis_actuals'] }} compared={compared} uncompared={uncompared} fromLabel="budget" toLabel="actual" truncated />);
    expect(screen.queryByText(/by the most/)).toBeNull();
    expect(screen.getByText(/^Among the 2 compared lines returned, the largest excess over the budget amount is .*512000: \+\$236K.*cut at the row limit/)).toBeTruthy();
    expect(screen.getByRole('heading', { name: 'Actuals minus budget: largest among returned rows' })).toBeTruthy();
    // Unmatched lines sort last, so a cut result can hold none of them: the gap is stated, never read as 'all matched'.
    cleanup();
    render(<DifferenceDots title="Actuals minus budget" sources={{ tables: ['analysis_budget'] }} compared={compared} uncompared={[]} fromLabel="budget" toLabel="actual" truncated />);
    expect(screen.getByText(/could not be compared sort after compared ones, so any beyond the limit are not counted here/)).toBeTruthy();
  });
});
