// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import fixture from '../../src-tauri/tests/fixtures/automatic-discovery.json';
import calls from '../../src-tauri/tests/fixtures/call-leads.json';
import type { AnalyticalDatasetSummary } from '../types';

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), stage: vi.fn(), view: vi.fn(), pattern: vi.fn(), portal: vi.fn(), settle: vi.fn(), log: vi.fn() }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke, formatNativeError: (reason: unknown) => String(reason) }));
vi.mock('../store/useAppStore', () => ({ useAppStore: Object.assign((selector: (s: unknown) => unknown) => selector({ evidenceRefreshRequests: 0, firstRunDataSettled: true }), { getState: () => ({ setPortalUrlDraft: bridge.stage, setWorkspaceView: bridge.view, setPatternEvidenceSelection: bridge.pattern, setActivePortal: bridge.portal, settleFirstRunData: bridge.settle, appendSyncLog: bridge.log }) }) }));
import AutomaticDiscovery, { BeatMap } from '../components/dashboard/AutomaticDiscovery';
import { AutomaticDiscoveryProvider } from '../components/dashboard/AutomaticDiscoveryProvider';
import { firstRunTables, useFirstRunDataStore } from '../store/useFirstRunDataStore';
const upkeep = { policy: { enabled: true, autoImport: true, autoRefresh: true, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 10 * 1024 ** 3 }, effectiveAt: '2026-09-14T00:00:00Z', retainedFileBytes: 1048576, referencedDocuments: 0, reclaimedFileBytes: 0, events: [] };
const resetFirstRun = () => useFirstRunDataStore.setState({ running: false, tables: [], statuses: {}, unfetched: [] });
beforeEach(resetFirstRun);
afterEach(() => { cleanup(); vi.clearAllMocks(); resetFirstRun(); });
const emptyLibrary = () => bridge.invoke.mockImplementation(async (name: string) => {
  if (name === 'get_analytical_schema') return { datasets: [], columns: [], templates: [] };
  if (name === 'get_data_maintenance_status') return upkeep;
  if (name === 'fetch_city_dataset') return { importedRows: 0, rejectedRows: 0 };
  return [];
});
const mount = () => render(<AutomaticDiscoveryProvider><AutomaticDiscovery /></AutomaticDiscoveryProvider>);

describe('automatic discovery landing experience', () => {
  it('shows observations without interaction and opens their evidence', async () => {
    bridge.invoke.mockImplementation(async (name: string) => {
      if (name === 'get_analytical_schema') return fixture.schema;
      if (name === 'get_data_maintenance_status') return upkeep;
      if (name === 'run_analytical_sql') return fixture.profile;
      return [];
    });
    mount();
    await screen.findByText('Coverage is partial or unverified');
    await screen.findAllByRole('button', { name: 'Explore this finding' });
    expect(screen.queryByText(/missing in amount/)).toBeNull();
    expect(bridge.invoke.mock.calls.some(([name]) => name === 'run_analytical_sql')).toBe(true);
    fireEvent.click(screen.getAllByRole('button', { name: 'Explore this finding' })[0]);
    expect(bridge.view).toHaveBeenCalledWith('analysis');
    expect(screen.getAllByText(/SHA-256 aaaa/).length).toBeGreaterThan(0);
  });
  it('draws a lead with its figure and keeps its prose under Figures as text', async () => {
    const columns = ['call_year', 'call_type', 'calls', 'cancelled_duplicate', 'blank', 'conflicting', 'arrest', 'report', 'no_report', 'unfounded', 'other', 'crisis_team', 'other_codes', 'last_call'];
    const empty = { columns: [], rows: [], rowCount: 0, truncated: false, elapsedMs: 1, provenance: [] };
    bridge.invoke.mockImplementation(async (name: string, args?: { sql?: string }) => {
      if (name === 'get_analytical_schema') return calls.schema;
      if (name === 'get_data_maintenance_status') return upkeep;
      if (name === 'run_analytical_sql') return args?.sql?.includes('AS crisis_team')
        ? { ...empty, columns, rowCount: 1, rows: [[2025, 'SLEEPER', 520, 20, 5, 0, 12, 8, 40, 3, 432, 5, null, '2025-12-31 23:00:00']] }
        : empty;
      return [];
    });
    mount();
    await screen.findByText('How the calls ended, by call type and year', {}, { timeout: 8000 });
    const prose = screen.getByText('Figures as text').closest('details')!;
    expect(prose.textContent).toContain('SLEEPER in 2025: 520 calls');
    expect(screen.getByRole('button', { name: 'View as table' })).toBeTruthy();
    expect(screen.getByText(/Crisis Response Team \(also counted under its outcome\)/)).toBeTruthy();
  });
  it('offers honest starting questions for an empty library and can save compact retention', async () => {
    bridge.invoke.mockImplementation(async (name: string, args?: { policy: unknown }) => {
      if (name === 'get_analytical_schema') return { datasets: [], columns: [], templates: [] };
      if (name === 'get_data_maintenance_status') return upkeep;
      if (name === 'set_data_maintenance_policy') return { ...upkeep, policy: args?.policy };
      return [];
    });
    mount();
    await screen.findByText(/These are starting questions/);
    fireEvent.click(screen.getByRole('button', { name: 'Choose housing reports' }));
    expect(bridge.stage).toHaveBeenCalledWith('https://www.rtfhsd.org/reports-data/');
    fireEvent.click(screen.getByText(/^Storage/));
    fireEvent.change(await screen.findByLabelText('Original download retention'), { target: { value: 'REFERENCE_ONLY' } });
    expect(screen.getByText(/Original-file layout and forensic checks require the original bytes/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Save upkeep settings' }));
    await screen.findByText('Upkeep settings saved.');
    expect(bridge.invoke).toHaveBeenCalledWith('set_data_maintenance_policy', { policy: { ...upkeep.policy, retentionMode: 'REFERENCE_ONLY' } });
  });
  // First run settled (skipped or finished) hides its card for good; this is the way back.
  it('fetches the core tables from an empty library after first run settled, and opens the overview', async () => {
    emptyLibrary();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Get San Diego’s core public data' }));
    const [first] = firstRunTables();
    expect(bridge.invoke).toHaveBeenCalledWith('fetch_city_dataset', { dataset: first.dataset, year: first.year });
    expect(bridge.view).toHaveBeenCalledWith('overview');
    await waitFor(() => expect(useFirstRunDataStore.getState().running).toBe(false));
  });
  it('disables the core-data button while a first-run fetch is running', async () => {
    emptyLibrary();
    useFirstRunDataStore.setState({ running: true });
    mount();
    const button = await screen.findByRole('button', { name: 'Get San Diego’s core public data' });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });
  it('does not offer the core fetch when the catalog failed and nothing else stands out', async () => {
    bridge.invoke.mockImplementation(async (name: string) => {
      if (name === 'get_analytical_schema') throw new Error('Catalog failed');
      if (name === 'get_data_maintenance_status') return upkeep;
      return [];
    });
    mount();
    await screen.findByText(/These are starting questions/);
    await screen.findByText(/Table checks could not run/);
    expect(screen.queryByRole('button', { name: 'Get San Diego’s core public data' })).toBeNull();
  });
  it('surfaces document connections even when the analytical catalog fails', async () => {
    bridge.invoke.mockImplementation(async (name: string) => {
      if (name === 'get_analytical_schema') throw new Error('Catalog failed');
      if (name === 'get_data_maintenance_status') return upkeep;
      if (name === 'query_corpus_patterns') return [{ patternId: 'p1', kind: 'SHARED_ENTITY', title: 'Recurring program', explanation: 'Two indexed records name the program.', confidenceBasisPoints: 8000, algorithmVersion: 'fixture', evidence: [{ portalKey: 'city', requestId: 1, documentId: 2, characterStart: 15 }] }];
      return [];
    });
    mount();
    await screen.findByText('Recurring program');
    expect(screen.getByText(/Table checks could not run/).closest('[role="status"]')?.textContent).toContain('Catalog failed');
    expect(screen.queryByRole('button', { name: 'Get San Diego’s core public data' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Inspect document evidence' }));
    await waitFor(() => expect(bridge.pattern).toHaveBeenCalledWith({ patternId: 'p1', portalKey: 'city', documentId: 2, characterStart: 15 }));
    expect(bridge.view).toHaveBeenCalledWith('patterns');
  });
});

describe('the beat map', () => {
  const table = (snapshotId: string): AnalyticalDatasetSummary => ({ ...(fixture.schema.datasets[0] as AnalyticalDatasetSummary), snapshotId, tableName: `analysis_${snapshotId}`, displayName: `Police beats ${snapshotId}` });
  const shape = (west: number) => JSON.stringify({ type: 'Polygon', coordinates: [[[west, 32.7], [west + 0.01, 32.7], [west + 0.01, 32.71], [west, 32.71], [west, 32.7]]] });
  const answer = (beat: string) => ({ columns: ['beat', '_geometry', '_crs'], rows: [[beat, shape(-117.1), null]], rowCount: 1, truncated: false, elapsedMs: 1, provenance: [] });

  it('discards boundaries that resolve after the beat table changed', async () => {
    let releaseA: (value: unknown) => void = () => {};
    bridge.invoke.mockImplementation((_name: string, args: { sql: string }) => (args.sql.includes('analysis_beats_a')
      ? new Promise((resolve) => { releaseA = resolve; })
      : Promise.resolve(answer('999'))));
    const props = { counts: [{ beat: '521', calls: 5 }, { beat: '999', calls: 3 }], noBeat: 2, citywide: 10, sources: { tables: ['pd_calls_for_service_2025'] } };
    const view = render(<BeatMap boundary={{ table: table('beats_a'), crs: true }} {...props} />);
    view.rerender(<BeatMap boundary={{ table: table('beats_b'), crs: true }} {...props} />);
    await screen.findByRole('img', { name: /Beat 999: 3 SLEEPER calls/ });
    await act(async () => {
      releaseA(answer('521'));
      await Promise.resolve();
    });
    expect(screen.queryByRole('img', { name: /Beat 521: 5 SLEEPER calls/ })).toBeNull();
    expect(screen.getByRole('img', { name: /Beat 999: 3 SLEEPER calls/ })).toBeTruthy();
    expect(screen.getByText(/SLEEPER calls: no place recorded/)).toBeTruthy();
  });
});
