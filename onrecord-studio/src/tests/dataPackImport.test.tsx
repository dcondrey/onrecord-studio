// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../ipc';
import DataPackImport, { coverageMatrix } from '../components/dashboard/DataPackImport';
import { RECORD_GAP_DEFINITIONS, type RecordGap } from '../utils/recordGaps';
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

    const items = within(screen.getByRole('list', { name: 'Data pack import results' })).getAllByRole('listitem').map((item) => item.textContent ?? '');
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

  it('draws a year with no file as an empty cell, never a zero-row one, and selects a file from its cell', async () => {
    const file = (year: number) => ({ path: `police_calls/${year}.csv`, year, rows: 1_200, bytes: 10, partial: false, sourceUrl: 'https://example.test/', retrievedAt: '2026-10-05T23:59:15Z', datasetKind: 'CALLS_FOR_SERVICE' as const, licence: null });
    const manifest = { ...summary('calls'), tables: [{ table: 'police_calls', files: [file(2024), file(2026)], unchangedChecks: 0, lastCheckedAt: null, changeRecords: 0, changes: [] }] };
    vi.mocked(open).mockResolvedValueOnce('/packs/calls');
    vi.mocked(invokeCommand).mockImplementation(((command: string) => Promise.resolve(command === "list_published_packs" ? [] : manifest)) as unknown as typeof invokeCommand);
    render(<DataPackImport />);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Choose pack folder' })); });

    const gap = screen.getByRole('gridcell', { name: /^police_calls, 2025: no file in this pack/ });
    expect(gap.querySelector('[data-absence="not-covered"]')).toBeTruthy();
    expect(gap.textContent).not.toMatch(/\b0\b/);
    expect(screen.getByText('2 of 3 table-years have a file in this pack; 1 has none.')).toBeTruthy();

    fireEvent.keyDown(screen.getByRole('gridcell', { name: /^police_calls, 2026: 1,200 rows/ }), { key: 'Enter' });
    expect((screen.getByRole('checkbox', { name: /^2026 · 1,200 rows/ }) as HTMLInputElement).checked).toBe(true);
  });

  it('states a whole-records change pair as not separated, never as 0 changed', async () => {
    const original = (stamp: string) => ({ path: `originals/call_types/${stamp}/types.csv`, year: null, rows: 10, bytes: 10, partial: false, sourceUrl: 'https://example.test/', retrievedAt: `${stamp}T13:23:00Z`, datasetKind: 'REGIONAL_DATA' as const, licence: null });
    const changes = [
      { previous: original('2026-09-01').path, previousRetrievedAt: '2026-09-01T13:23:00Z', current: original('2026-09-15').path, currentRetrievedAt: '2026-09-15T13:23:00Z', added: 4, removed: 1, changed: 2, wholeRecords: false },
      { previous: original('2026-09-15').path, previousRetrievedAt: '2026-09-15T13:23:00Z', current: original('2026-10-01').path, currentRetrievedAt: '2026-10-01T13:23:00Z', added: 9, removed: 9, changed: null, wholeRecords: true },
    ];
    const manifest = { ...summary('snapshots'), snapshots: true, tables: [{ table: 'call_types', files: [original('2026-09-01'), original('2026-09-15'), original('2026-10-01')], unchangedChecks: 3, lastCheckedAt: '2026-10-06T13:23:02Z', changeRecords: 2, changes }] };
    vi.mocked(open).mockResolvedValueOnce('/packs/snapshots');
    vi.mocked(invokeCommand).mockImplementation(((command: string) => Promise.resolve(command === "list_published_packs" ? [] : manifest)) as unknown as typeof invokeCommand);
    render(<DataPackImport />);
    await act(async () => { fireEvent.click(screen.getByRole('button', { name: 'Choose pack folder' })); });

    expect(screen.getByText('call_types: +13 / −10 / 2 changed (1 pair not separated) across 3 retrievals since Sep 1, 2026; last checked Oct 6, 2026: unchanged from the latest snapshot.')).toBeTruthy();
    const whole = screen.getByRole('img', { name: /^Sep 15, 2026 to Oct 1, 2026: 9 added, 9 removed, changed not separated/ });
    expect(whole.textContent).toContain('not separated');
    fireEvent.click(screen.getAllByRole('button', { name: 'View as table' }).at(-1)!);
    const row = screen.getByRole('cell', { name: /^not separated: counted in added and removed/ }).closest('tr')!;
    expect(row.textContent).not.toMatch(/(^|[^\d,])0($|[^\d,])/);
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

  it('claims a missing file only inside a table\'s own years, spans a one-file table, and adds the records leads need', () => {
    const file = (table: string, year: number | null, rows = 10) => ({ path: `${table}/${year ?? 'all'}.csv`, year, rows, bytes: 10, partial: false, sourceUrl: `https://example.test/${table}`, retrievedAt: '2026-10-05T23:59:15Z', datasetKind: 'CALLS_FOR_SERVICE' as const, licence: null });
    const tables = [
      { table: 'budget', files: [file('budget', 2022), file('budget', 2024)] },
      { table: 'calls', files: [file('calls', 2019), file('calls', 2025)] },
      { table: 'beats', files: [file('beats', null)] },
    ].map((table) => ({ ...table, unchangedChecks: 0, lastCheckedAt: null, changeRecords: 0, changes: [] }));
    const choices = tables.flatMap((table) => table.files.map((f) => ({ key: f.path, table: table.table, year: f.year, files: [f] })));
    const gap = { key: 'g', leadRef: 'l', leadTitle: 'Repeat abatements', leadQuestion: '?', snapshotId: 's', definition: RECORD_GAP_DEFINITIONS[0], period: '2023 to 2024', identifiers: [], table: { name: 'abatements', source: null, retrievedOn: '2026-10-01', sha256: 'ab' }, sourceOrigin: null, answered: false } satisfies RecordGap;
    const datasets = [{ sourceUri: 'https://example.test/calls', retrievedAt: '2026-10-05T23:59:15Z' }] as unknown as Parameters<typeof coverageMatrix>[3];
    const matrix = coverageMatrix({ ...summary('mixed'), tables }, choices, new Set(), datasets, [gap]);
    expect(matrix.columns).toEqual(['2019', '2020', '2021', '2022', '2023', '2024', '2025']);
    const [budget, calls, beats, needed] = matrix.rows;
    // budget runs 2022-2024: 2019-2021 are outside its years, 2023 is its one missing file.
    expect(budget.cells.slice(0, 3).every((cell) => cell?.kind === 'gap' && cell.text === 'outside')).toBe(true);
    expect(budget.cells[4]).toMatchObject({ kind: 'gap', text: 'no file' });
    expect(matrix.headline).toBe('5 of 11 table-years have a file in this pack; 6 have none; 1 record needed by leads.');
    // Imported files fill the ramp; files only in the pack are outlines.
    expect(calls.cells[0]?.kind).toBe('count');
    expect(budget.cells[3]).toMatchObject({ kind: 'state', tone: 'neutral' });
    expect(beats.cells[0]?.span).toBe(7);
    expect(beats.cells.slice(1).every((cell) => cell === null)).toBe(true);
    expect(needed.cells[4]).toMatchObject({ kind: 'gap', lead: true, text: 'needed', actionable: true, span: 2 });
    expect(needed.cells[4]?.name).toContain('Repeat abatements');
  });
});
