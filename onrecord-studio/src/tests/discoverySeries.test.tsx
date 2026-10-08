// @vitest-environment jsdom
//
// When the 'changes' check is truncated, discovery reads whole series with one more query after the
// run finishes. A series for an earlier run that resolves after a later run has drawn its own must
// never be drawn as the later run's series.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { AnalyticalSchema, SqlQueryResult } from '../types';
import fixture from '../../src-tauri/tests/fixtures/discovery.json';
import { discoverySeriesModel, SERIES_ORDER } from '../utils/dataDiscovery';

const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../store/useAppStore', () => ({ useAppStore: { getState: () => ({}) } }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));
import DataDiscovery from '../components/dashboard/DataDiscovery';

const CHANGE_COLUMNS = ['group_value', 'discovery_period', 'source_rows', 'missing_values', 'observed_value', 'previous_period',
  'previous_value', 'previous_rows', 'previous_missing_values', 'comparison_status', 'change', 'change_percent'];

function result(columns: string[], rows: unknown[][], truncated = false): SqlQueryResult {
  return { columns, rows, rowCount: rows.length, truncated, elapsedMs: 1, provenance: [] };
}

function seriesRows(group: string): unknown[][] {
  return [
    [group, 22, 10, 0, 10, null, null, null, null, 'No baseline', null, null],
    [group, 23, 12, 0, 12, 22, 10, 10, 0, 'Adjacent periods with reported values', 2, 20],
  ];
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('discovery series', () => {
  it('discards a truncated run’s series query that resolves after a later run’s', async () => {
    let group = 'Alpha';
    const series: { group: string; resolve(value: SqlQueryResult): void }[] = [];
    bridge.invoke.mockImplementation((_name: string, { sql }: { sql: string }) => {
      const runGroup = group;
      if (sql.includes(SERIES_ORDER)) return new Promise((resolve) => series.push({ group: runGroup, resolve }));
      if (sql.includes('record_share_percent')) return Promise.resolve(result(['group_value', 'source_rows', 'missing_values', 'observed_value', 'record_share_percent'], [[runGroup, 22, 0, 22, 100]]));
      if (sql.includes('comparison_status')) return Promise.resolve(result(CHANGE_COLUMNS, seriesRows(runGroup).slice(1), true));
      return Promise.resolve(result(['source_rows'], [[22]]));
    });
    render(<DataDiscovery schema={fixture.schema as AnalyticalSchema} onInspect={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('Discovery groups'), { target: { value: 'group' } });
    fireEvent.change(screen.getByLabelText('Discovery time field'), { target: { value: 'year' } });

    fireEvent.click(screen.getByRole('button', { name: 'Discover patterns' }));
    await waitFor(() => expect(series).toHaveLength(1));
    group = 'Bravo';
    fireEvent.click(screen.getByRole('button', { name: 'Discover patterns' }));
    await waitFor(() => expect(series).toHaveLength(2));
    expect(series.map((query) => query.group)).toEqual(['Alpha', 'Bravo']);

    await act(async () => series[1].resolve(result(CHANGE_COLUMNS, seriesRows('Bravo'))));
    expect(await screen.findByRole('img', { name: /^Bravo: 2 periods/ })).toBeTruthy();
    await act(async () => series[0].resolve(result(CHANGE_COLUMNS, seriesRows('Alpha'))));

    expect(screen.queryByRole('img', { name: /^Alpha:/ })).toBeNull();
    expect(screen.getByRole('img', { name: /^Bravo: 2 periods/ })).toBeTruthy();
  });

  it('says why a truncated series was not read instead of loading forever', async () => {
    bridge.invoke.mockImplementation((_name: string, { sql }: { sql: string }) => {
      if (sql.includes(SERIES_ORDER)) return Promise.resolve(result(CHANGE_COLUMNS, seriesRows('Alpha')));
      if (sql.includes('record_share_percent')) return Promise.reject(new Error('groups failed'));
      if (sql.includes('comparison_status')) return Promise.resolve(result(CHANGE_COLUMNS, seriesRows('Alpha').slice(1), true));
      return Promise.resolve(result(['source_rows'], [[22]]));
    });
    render(<DataDiscovery schema={fixture.schema as AnalyticalSchema} onInspect={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('Discovery groups'), { target: { value: 'group' } });
    fireEvent.change(screen.getByLabelText('Discovery time field'), { target: { value: 'year' } });
    fireEvent.click(screen.getByRole('button', { name: 'Discover patterns' }));
    await screen.findByText('Analysis complete.');
    expect(await screen.findByText(/Series not read: the groups check returned no result/)).toBeTruthy();
    expect(bridge.invoke.mock.calls.some(([, args]) => (args as { sql: string }).sql.includes(SERIES_ORDER))).toBe(false);
    expect(screen.queryByText('Loading…')).toBeNull();
  });

  it('breaks a series at periods no group has rows for instead of joining across them', () => {
    const rows = [
      ['A', 2022, 5, 0, 5, null, null, null, null, 'No baseline', null, null],
      ['A', 2025, 9, 0, 9, 2022, 5, 5, 0, 'Missing intervening periods', null, null],
      ['B', 2022, 1, 0, 1, null, null, null, null, 'No baseline', null, null],
      ['B', 2023, 2, 0, 2, 2022, 1, 1, 0, 'Adjacent periods with reported values', 1, 100],
    ];
    const model = discoverySeriesModel(result(CHANGE_COLUMNS, rows), ['A', 'B'], 'year_code');
    // 2023 is B's, so A is absent there; 2024 is no one's, so a gap slot stands between 2023 and 2025.
    expect(model.slots.map((slot) => (slot.gap ? 'gap' : slot.label))).toEqual(['2022', '2023', 'gap', '2025']);
    expect(model.groups[0].cells.map((cell) => cell?.observed ?? null)).toEqual([5, null, null, 9]);
  });
});
