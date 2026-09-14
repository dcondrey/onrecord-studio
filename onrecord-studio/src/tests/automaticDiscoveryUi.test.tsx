// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import fixture from '../../src-tauri/tests/fixtures/automatic-discovery.json';

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), stage: vi.fn(), view: vi.fn(), pattern: vi.fn(), portal: vi.fn() }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));
vi.mock('../store/useAppStore', () => ({ useAppStore: Object.assign((selector: (s: unknown) => unknown) => selector({ evidenceRefreshRequests: 0 }), { getState: () => ({ setPortalUrlDraft: bridge.stage, setWorkspaceView: bridge.view, setPatternEvidenceSelection: bridge.pattern, setActivePortal: bridge.portal }) }) }));
import AutomaticDiscovery from '../components/dashboard/AutomaticDiscovery';
import { AutomaticDiscoveryProvider } from '../components/dashboard/AutomaticDiscoveryProvider';
const upkeep = { policy: { enabled: true, autoImport: true, autoRefresh: true, refreshHours: 24, retentionMode: 'ORIGINALS', retainedFileBudgetBytes: 10 * 1024 ** 3 }, effectiveAt: '2026-09-14T00:00:00Z', retainedFileBytes: 1048576, referencedDocuments: 0, reclaimedFileBytes: 0, events: [] };
afterEach(() => { cleanup(); vi.clearAllMocks(); });
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
    await screen.findByText('3.6% missing in amount');
    expect(bridge.invoke.mock.calls.some(([name]) => name === 'run_analytical_sql')).toBe(true);
    fireEvent.click(screen.getAllByRole('button', { name: 'Explore this finding' })[0]);
    expect(bridge.view).toHaveBeenCalledWith('analysis');
    expect(screen.getAllByText(/SHA-256 aaaa/).length).toBeGreaterThan(0);
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
    fireEvent.click(screen.getByRole('button', { name: 'Choose budget data' }));
    expect(bridge.stage).toHaveBeenCalledWith('https://data.sandiego.gov/datasets/operating-budget');
    fireEvent.click(screen.getByText(/Data upkeep & storage/));
    fireEvent.change(await screen.findByLabelText('New acquisitions'), { target: { value: 'REFERENCE_ONLY' } });
    expect(screen.getByText(/publishers can change or remove files/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Save upkeep settings' }));
    await screen.findByText('Upkeep settings saved.');
    expect(bridge.invoke).toHaveBeenCalledWith('set_data_maintenance_policy', { policy: { ...upkeep.policy, retentionMode: 'REFERENCE_ONLY' } });
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
    expect(screen.getByRole('alert').textContent).toContain('Catalog failed');
    fireEvent.click(screen.getByRole('button', { name: 'Inspect document evidence' }));
    await waitFor(() => expect(bridge.pattern).toHaveBeenCalledWith({ patternId: 'p1', portalKey: 'city', documentId: 2, characterStart: 15 }));
    expect(bridge.view).toHaveBeenCalledWith('patterns');
  });
});
