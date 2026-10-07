// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../ipc';
import DataPackImport from '../components/dashboard/DataPackImport';
import type { AnalyticalImportResult, DataPackImportOutcome, DataPackSummary } from '../types';

vi.mock('../ipc', () => ({ invokeCommand: vi.fn(), formatNativeError: String }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const summary = (pack: string): DataPackSummary => ({
  pack,
  packVersion: '2026.10.6',
  schemaVersion: '1.0',
  builtAt: '2026-10-06T22:59:25Z',
  signerKeyId: 'ab'.repeat(32),
  licenceId: 'ODC-PDDL-1.0',
  licenceUrl: 'https://opendefinition.org/licenses/odc-pddl/',
  disclaimer: 'Informational only.',
  manifestSha256: 'cd'.repeat(32),
  tables: [],
});

describe('DataPackImport', () => {
  it('never shows a verification that resolves after another folder was chosen', async () => {
    const pending = new Map<string, (value: DataPackSummary) => void>();
    vi.mocked(invokeCommand).mockImplementation(((_command: string, args: { path: string }) =>
      new Promise((resolve) => pending.set(args.path, resolve))) as unknown as typeof invokeCommand);
    vi.mocked(open).mockResolvedValueOnce('/packs/first').mockResolvedValueOnce('/packs/second');
    render(<DataPackImport />);
    const choose = screen.getByRole('button', { name: 'Choose pack folder' });

    await act(async () => { fireEvent.click(choose); });
    await act(async () => { fireEvent.click(choose); });
    await act(async () => { pending.get('/packs/second')?.(summary('second-pack')); });
    await act(async () => { pending.get('/packs/first')?.(summary('first-pack')); });

    expect(screen.getByText(/second-pack/)).toBeTruthy();
    expect(screen.queryByText(/first-pack/)).toBeNull();
  });

  it('states per file whether rows were imported, none were, or the bytes were already stored', async () => {
    const files = ['a.csv', 'b.csv', 'c.csv'].map((path) => ({
      path, year: null, rows: 2, bytes: 10, partial: false, sourceUrl: 'https://example.test/', retrievedAt: '2026-10-05T23:59:15Z', datasetKind: 'REGIONAL_DATA' as const,
    }));
    const result = (importedRows: number): AnalyticalImportResult => ({
      snapshotId: 's', datasetKind: 'REGIONAL_DATA', displayName: 'd', tableName: 'analysis_t', importedRows, rejectedRows: 0, malformedRows: 0, duplicateKeyRows: 0, unconvertedCells: 0, schemaWarnings: [], sourceSha256: 'e'.repeat(64),
    });
    const outcome = (path: string, rows: number, alreadyStored: boolean): DataPackImportOutcome => ({
      path, table: 'budget_actuals', year: null, retrievedAt: '2026-10-05T23:59:15Z', result: result(rows), alreadyStored, error: null,
    });
    vi.mocked(open).mockResolvedValueOnce('/packs/money');
    vi.mocked(invokeCommand).mockImplementation(((command: string, args: { manifestSha256?: string }) => Promise.resolve(command === 'inspect_data_pack'
      ? { ...summary('money'), tables: [{ table: 'budget_actuals', files }] }
      : args.manifestSha256 === 'cd'.repeat(32) ? [outcome('a.csv', 2, false), outcome('b.csv', 0, false), outcome('c.csv', 2, true)] : [])) as unknown as typeof invokeCommand);
    render(<DataPackImport />);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Choose pack folder' })); });
    for (const checkbox of screen.getAllByRole('checkbox')) fireEvent.click(checkbox);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Import 3 files' })); });

    const items = screen.getAllByRole('listitem').map((item) => item.textContent ?? '');
    expect(items[0]).toContain('2 rows imported to analysis_t, retrieved by the pack builder on 2026-10-05.');
    expect(items[1]).toContain('No rows imported');
    expect(items[2]).toContain('Nothing imported: these bytes are already stored as analysis_t');
    expect(items[2]).not.toContain('rows imported to');
  });
});
