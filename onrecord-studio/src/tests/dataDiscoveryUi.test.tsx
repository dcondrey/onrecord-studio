// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { AnalyticalSchema } from '../types';
import fixture from '../../src-tauri/tests/fixtures/discovery.json';

const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../store/useAppStore', () => ({ useAppStore: { getState: () => ({}) } }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));
import DataDiscovery from '../components/dashboard/DataDiscovery';
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe('data discovery workflow', () => {
  it('runs without a model, retains partial results, and opens scoped records for review', async () => {
    bridge.invoke.mockImplementation(async (_name, { sql }: { sql: string }) => {
      if (sql.includes('AS logical_type')) throw new Error('One check failed');
      if (sql.includes('record_share_percent')) return { columns: ['group_value', 'source_rows'], rows: [["O'Brien", 4]], rowCount: 1, elapsedMs: 2, truncated: false, provenance: [] };
      return { columns: ['source_rows'], rows: [[27]], rowCount: 1, elapsedMs: 2, truncated: false, provenance: [] };
    });
    const inspect = vi.fn();
    render(<DataDiscovery schema={fixture.schema as AnalyticalSchema} onInspect={inspect} />);
    fireEvent.change(screen.getByLabelText('Discovery groups'), { target: { value: 'group' } });
    fireEvent.click(screen.getByRole('button', { name: 'Discover patterns' }));
    await screen.findByText('Analysis complete.');
    expect(screen.getByText('Error: One check failed')).toBeTruthy();
    expect(bridge.invoke.mock.calls.every(([name]) => name === 'run_analytical_sql')).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: 'Explore records' }));
    expect(inspect.mock.calls[0][0]).toContain("'O''Brien'");
    fireEvent.change(screen.getByLabelText('Discovery time field'), { target: { value: '' } });
    expect(screen.getByText(/Controls changed/)).toBeTruthy();
  });
  it('stops after a pending query and discloses unfinished checks', async () => {
    let resolve: (value: unknown) => void = () => {};
    bridge.invoke.mockImplementation(() => new Promise((done) => { resolve = done; }));
    render(<DataDiscovery schema={fixture.schema as AnalyticalSchema} onInspect={vi.fn()} />);
    fireEvent.click(screen.getByRole('button', { name: 'Discover patterns' }));
    fireEvent.click(screen.getByRole('button', { name: 'Stop after current query' }));
    resolve({ columns: [], rows: [], rowCount: 0, truncated: false, elapsedMs: 1, provenance: [] });
    await waitFor(() => expect(screen.getByText(/Stopped after the current query/)).toBeTruthy());
    expect(bridge.invoke).toHaveBeenCalledTimes(1);
    expect(screen.getByText(/1\/5 checks returned/)).toBeTruthy();
  });
});
