// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../ipc';
import DataPackImport from '../components/dashboard/DataPackImport';
import type { AnalyticalImportResult, DataPackImportOutcome, DataPackSummary, PublishedPack } from '../types';

vi.mock('../ipc', () => ({ invokeCommand: vi.fn(), formatNativeError: String }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const summary = (pack: string): DataPackSummary => ({
  pack,
  snapshots: false,
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
      path, year: null, rows: 2, bytes: 10, partial: false, sourceUrl: 'https://example.test/', retrievedAt: '2026-10-05T23:59:15Z', datasetKind: 'REGIONAL_DATA' as const, licence: null,
    }));
    const result = (importedRows: number): AnalyticalImportResult => ({
      snapshotId: 's', datasetKind: 'REGIONAL_DATA', displayName: 'd', tableName: 'analysis_t', importedRows, rejectedRows: 0, malformedRows: 0, duplicateKeyRows: 0, unconvertedCells: 0, schemaWarnings: [], sourceSha256: 'e'.repeat(64),
    });
    const outcome = (path: string, rows: number, alreadyStored: boolean): DataPackImportOutcome => ({
      path, table: 'budget_actuals', year: null, retrievedAt: '2026-10-05T23:59:15Z', result: result(rows), alreadyStored, error: null,
    });
    vi.mocked(open).mockResolvedValueOnce('/packs/money');
    vi.mocked(invokeCommand).mockImplementation(((command: string, args: { manifestSha256?: string }) => Promise.resolve(command === 'inspect_data_pack'
      ? { ...summary('money'), tables: [{ table: 'budget_actuals', files, unchangedChecks: 0, lastCheckedAt: null, changeRecords: 0 }] }
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

  const packs: PublishedPack[] = ['san-diego-money', 'san-diego-reports'].map((pack) => ({
    pack, repositoryUrl: `https://huggingface.co/datasets/Writerslogic/onrecord-${pack}`,
  }));

  it('shows a published pack from its signed manifest and downloads only the chosen years', async () => {
    const file = (year: number, bytes: number) => ({
      path: `originals/police_calls/${year}.csv`, year, rows: 10, bytes, partial: false, sourceUrl: 'https://example.test/', retrievedAt: '2026-10-05T23:59:15Z', datasetKind: 'REGIONAL_DATA' as const, licence: null,
    });
    const manifest = { ...summary('san-diego-reports'), tables: [{ table: 'police_calls', files: [file(2024, 2_000_000), file(2025, 3_000_000)], unchangedChecks: 0, lastCheckedAt: null, changeRecords: 0 }] };
    vi.mocked(invokeCommand).mockImplementation(((command: string) => Promise.resolve(
      command === 'list_published_packs' ? packs : command === 'fetch_published_pack_manifest' ? manifest : [],
    )) as unknown as typeof invokeCommand);
    await act(async () => { render(<DataPackImport />); });
    await act(async () => { fireEvent.change(screen.getByRole('combobox', { name: 'Published pack' }), { target: { value: 'san-diego-reports' } }); });

    expect(invokeCommand).toHaveBeenCalledWith('fetch_published_pack_manifest', { pack: 'san-diego-reports' });
    expect(screen.getByText(/Built 2026-10-06T22:59:25Z/)).toBeTruthy();
    expect(screen.getByText('Informational only.')).toBeTruthy();
    fireEvent.click(screen.getByRole('checkbox', { name: /^2025 · 10 rows · 3 MB/ }));
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Download and import 3 MB' })); });

    expect(invokeCommand).toHaveBeenCalledWith('fetch_published_pack_files', {
      pack: 'san-diego-reports', packVersion: '2026.10.6', manifestSha256: 'cd'.repeat(32), selection: [{ table: 'police_calls', year: 2025 }],
    });
  });

  it('offers a snapshot table as its latest snapshot alone, whatever its earlier ones are', async () => {
    const snapshot = (stamp: string, bytes: number, datasetKind: 'REGIONAL_DATA' | null) => ({
      path: `originals/call_types/${stamp}/types.csv`, year: null, rows: 10, bytes, partial: false, sourceUrl: 'https://example.test/', retrievedAt: `${stamp}T13:23:00Z`, datasetKind, licence: null,
    });
    const files = [snapshot('2026-10-05', 1_000_000, null), snapshot('2026-10-07', 2_000_000, 'REGIONAL_DATA'), snapshot('2026-10-06', 4_000_000, 'REGIONAL_DATA')];
    const manifest = { ...summary('san-diego-snapshots'), snapshots: true, tables: [{ table: 'call_types', files, unchangedChecks: 0, lastCheckedAt: null, changeRecords: 2 }] };
    vi.mocked(invokeCommand).mockImplementation(((command: string) => Promise.resolve(
      command === 'list_published_packs' ? packs : command === 'fetch_published_pack_manifest' ? manifest : [],
    )) as unknown as typeof invokeCommand);
    await act(async () => { render(<DataPackImport />); });
    await act(async () => { fireEvent.change(screen.getByRole('combobox', { name: 'Published pack' }), { target: { value: 'san-diego-reports' } }); });

    const choice = screen.getByRole('checkbox', { name: /^Latest snapshot, retrieved 2026-10-07 · 10 rows · 2 MB · 2 earlier snapshots not downloaded$/ });
    fireEvent.click(choice);
    expect(screen.getByRole('button', { name: 'Download and import 2 MB' })).toBeTruthy();
  });

  it('never shows a published manifest that resolves after another pack was chosen', async () => {
    const pending = new Map<string, (value: DataPackSummary) => void>();
    vi.mocked(invokeCommand).mockImplementation(((command: string, args: { pack?: string }) => (command === 'list_published_packs'
      ? Promise.resolve(packs)
      : new Promise((resolve) => pending.set(args.pack ?? '', resolve)))) as unknown as typeof invokeCommand);
    await act(async () => { render(<DataPackImport />); });
    const choose = (pack: string) => fireEvent.change(screen.getByRole('combobox', { name: 'Published pack' }), { target: { value: pack } });

    await act(async () => { choose('san-diego-money'); });
    await act(async () => { choose('san-diego-reports'); });
    await act(async () => { pending.get('san-diego-reports')?.(summary('reports-manifest')); });
    await act(async () => { pending.get('san-diego-money')?.(summary('money-manifest')); });

    expect(screen.getByText(/reports-manifest/)).toBeTruthy();
    expect(screen.queryByText(/money-manifest/)).toBeNull();
  });
});
