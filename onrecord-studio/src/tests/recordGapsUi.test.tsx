// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { RecordGapRequestStatus } from '../types';
import { RECORD_GAP_DEFINITIONS, type RecordGap } from '../utils/recordGaps';

const bridge = vi.hoisted(() => ({ invoke: vi.fn(), open: vi.fn() }));
vi.mock('../ipc', async (importOriginal) => ({ ...(await importOriginal<typeof import('../ipc')>()), invokeCommand: bridge.invoke }));
vi.mock('../store/useAppStore', () => ({ useAppStore: Object.assign((selector: (s: unknown) => unknown) => selector({ portals: [{ registration: { displayName: 'City of Example', originUrl: 'https://example.test', canonicalUrl: 'https://example.test/requests' } }] }), { getState: () => ({ openCasework: bridge.open }) }) }));
import RecordGaps from '../components/dashboard/RecordGaps';

const gap = (key: string): RecordGap => ({ key, leadRef: `lead-${key}`, leadTitle: `Lead ${key}`, leadQuestion: 'What became of belongings?', snapshotId: 's', definition: RECORD_GAP_DEFINITIONS[0],
  period: '2024-01-01 to 2024-12-31', identifiers: [], table: { name: 'Log', source: 'https://example.test/log.csv', retrievedOn: '2026-10-04', sha256: 'a'.repeat(64) }, sourceOrigin: 'https://example.test', answered: false });
const status = (gapKey: string): RecordGapRequestStatus => ({ gapKey, caseId: 'c', reportId: 'lead:abc', requestId: 'gap-1', state: 'DRAFT', submittedAt: null, determinationDueOn: null, deadlinePassedWithoutRecordedResponse: false, productions: 0, remainingOmissions: 0 });
afterEach(() => { cleanup(); vi.clearAllMocks(); });

describe('record gaps in the feed', () => {
  it('discards tracked requests read for an earlier set of gaps', async () => {
    let resolveFirst: (value: RecordGapRequestStatus[]) => void = () => {};
    bridge.invoke.mockImplementation((name: string, args: { gapKeys: string[] }) => name !== 'get_record_gap_requests' ? Promise.resolve([])
      : args.gapKeys[0] === 'a' ? new Promise((resolve) => { resolveFirst = resolve; }) : Promise.resolve([status('b')]));
    const view = render(<RecordGaps gaps={[gap('a')]} />);
    view.rerender(<RecordGaps gaps={[gap('b')]} />);
    await screen.findByRole('button', { name: 'Open casework' });
    resolveFirst([]);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.getByRole('button', { name: 'Open casework' })).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Track this request' })).toBeNull();
  });

  it('tracks the drafted request with the source\'s own name as custodian, then opens its casework', async () => {
    let tracked = false;
    bridge.invoke.mockImplementation(async (name: string) => {
      if (name === 'get_record_gap_requests') return tracked ? [status('a')] : [];
      if (name === 'request_record_gap') tracked = true;
      return [];
    });
    render(<RecordGaps gaps={[gap('a')]} />);
    expect(screen.getByText('Not asked.')).toBeTruthy();
    fireEvent.click(await screen.findByRole('button', { name: 'Track this request' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Open casework' }));
    expect(bridge.open).toHaveBeenCalledWith('lead:abc');
    const [, { input }] = bridge.invoke.mock.calls.find(([name]) => name === 'request_record_gap')!;
    expect(input).toMatchObject({ gapKey: 'a', custodian: 'City of Example', period: '2024-01-01 to 2024-12-31' });
    await waitFor(() => expect(screen.getByText(/no submission date is recorded/)).toBeTruthy());
  });
});
