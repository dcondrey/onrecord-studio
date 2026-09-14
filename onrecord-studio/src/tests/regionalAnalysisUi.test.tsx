// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const bridge = vi.hoisted(() => ({ calls: [] as { name: string; args: Record<string, unknown> }[] }));
vi.mock('../store/useAppStore', () => ({ useAppStore: { getState: () => ({}) } }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: async (name: string, args: Record<string, unknown>) => {
  bridge.calls.push({ name, args });
  if (name === 'get_embedding_model_status') throw new Error('Model unavailable');
  if (name === 'get_embedding_index_status') return { indexedChunkCount: 0, indexedDocumentCount: 0, pendingDocumentCount: 0 };
  if (name === 'get_analytical_schema') return { datasets: [], columns: [], templates: [] };
  if (name === 'list_analytical_import_candidates') return [{ documentId: 7, portalKey: 'county', fileName: 'health.csv', localPath: '/managed/objects/abc', sourceUri: 'https://example.test/health.csv', sourceSha256: 'a'.repeat(64) }];
  if (name === 'import_analytical_dataset') return { importedRows: 12, rejectedRows: 1, malformedRows: 1, duplicateKeyRows: 0, tableName: 'analysis_regional_data_fixture', schemaWarnings: ['Source reports a partial export.'] };
  return null;
} }));
import AnalysisWorkbench from '../components/dashboard/AnalysisWorkbench';

afterEach(() => { cleanup(); bridge.calls.length = 0; });

describe('regional table import workflow', () => {
  it('offers acquired tables without a semantic model and passes their evidence identity to import', async () => {
    render(<AnalysisWorkbench />);
    await screen.findByRole('option', { name: /health.csv/ });
    fireEvent.change(screen.getByLabelText('Acquired table'), { target: { value: 'county:7' } });
    expect((screen.getByLabelText('Original source URL') as HTMLInputElement).value).toBe('https://example.test/health.csv');
    fireEvent.change(screen.getByLabelText('Observation start'), { target: { value: '2026-01-01' } });
    fireEvent.click(screen.getByRole('button', { name: 'Import table' }));
    await waitFor(() => expect(bridge.calls.some((call) => call.name === 'import_analytical_dataset')).toBe(true));
    expect(bridge.calls.find((call) => call.name === 'import_analytical_dataset')?.args).toEqual({ input: {
      datasetKind: 'REGIONAL_DATA', displayName: 'health.csv', sourceUri: 'https://example.test/health.csv', localPath: '/managed/objects/abc',
      observationPeriodStart: '2026-01-01', observationPeriodEnd: null, partialDataset: true, worksheet: null,
      sourceFileName: 'health.csv', expectedSourceSha256: 'a'.repeat(64),
    } });
    expect(await screen.findByText('Source reports a partial export.')).toBeTruthy();
    expect(screen.getByText(/12 rows imported.*1 rejected/)).toBeTruthy();
  });
});
