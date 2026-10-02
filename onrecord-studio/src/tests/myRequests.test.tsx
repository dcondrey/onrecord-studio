// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import MyRequestsPanel from '../components/tracker/MyRequestsPanel';
import { useMyRequests } from '../components/tracker/useMyRequests';
import { localInputToRfc3339 } from '../components/tracker/rfc3339';
import type { ClaimedRequest, RequestDeadline } from '../types';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand: invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(() => {
  cleanup();
  invoke.mockReset();
});

const claim = (id: string, overrides: Partial<ClaimedRequest> = {}): ClaimedRequest => ({
  id,
  portalKey: null,
  reference: `ref-${id}`,
  title: null,
  sourceUrl: null,
  jurisdiction: 'CA_CPRA',
  claimedAt: '2026-01-01T00:00:00Z',
  events: [],
  suggestions: [],
  importedEmails: [],
  ...overrides,
});

const deadline = (claimId: string, overrides: Partial<RequestDeadline> = {}): RequestDeadline => ({
  claimId,
  model: 'CALIFORNIA_CPRA',
  analysis: null,
  countdown: { kind: 'DAYS_REMAINING', days: 4 },
  determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY',
  violationAsserted: false,
  basisStatement: 'No determination is recorded.',
  notes: [],
  pendingSuggestionCount: 0,
  ...overrides,
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe('local date conversion', () => {
  it('keeps the wall-clock date and time and appends an offset, never converting to UTC', () => {
    expect(localInputToRfc3339('2026-01-05T23:30')).toMatch(/^2026-01-05T23:30:00[+-]\d{2}:\d{2}$/);
    expect(localInputToRfc3339('')).toBeNull();
    expect(localInputToRfc3339('2026-13-45T99:99')).toBeNull();
    expect(localInputToRfc3339('not a date')).toBeNull();
  });
});

describe('useMyRequests', () => {
  it('discards a refresh that was superseded by a newer one, whatever order they resolve in', async () => {
    const first = deferred<ClaimedRequest[]>();
    const second = deferred<ClaimedRequest[]>();
    const lists = [first, second];
    invoke.mockImplementation((command: string, args: { claimId?: string }) => {
      if (command === 'list_claimed_requests') return lists.shift()!.promise;
      if (command === 'get_request_deadline') return Promise.resolve(deadline(args.claimId!));
      return Promise.reject(new Error(command));
    });
    const { result } = renderHook(() => useMyRequests(true));
    await waitFor(() => expect(lists).toHaveLength(1));
    act(() => {
      void result.current.refresh();
    });
    await act(async () => {
      second.resolve([claim('newer')]);
    });
    await waitFor(() => expect(result.current.claims.map((c) => c.id)).toEqual(['newer']));
    await act(async () => {
      first.resolve([claim('stale')]);
    });
    await act(async () => {});
    expect(result.current.claims.map((c) => c.id)).toEqual(['newer']);
    expect(Object.keys(result.current.deadlines)).toEqual(['newer']);
  });

  it('keeps isBusy true while a second mutation is in flight, even after the first one settles', async () => {
    const firstOp = deferred<void>();
    const secondOp = deferred<void>();
    invoke.mockImplementation((command: string, args: { claimId?: string }) => {
      if (command === 'list_claimed_requests') return Promise.resolve([]);
      if (command === 'remove_claimed_request') return firstOp.promise;
      if (command === 'update_claimed_request') return secondOp.promise;
      if (command === 'get_request_deadline') return Promise.resolve(deadline(args.claimId!));
      return Promise.reject(new Error(command));
    });
    const { result } = renderHook(() => useMyRequests(true));
    await waitFor(() => expect(result.current.isBusy).toBe(false));

    act(() => {
      void result.current.remove('a');
    });
    await waitFor(() => expect(result.current.isBusy).toBe(true));
    act(() => {
      void result.current.update({ claimId: 'a' } as never);
    });

    await act(async () => {
      firstOp.resolve();
    });
    expect(result.current.isBusy).toBe(true);

    await act(async () => {
      secondOp.resolve();
    });
    await waitFor(() => expect(result.current.isBusy).toBe(false));
  });
});

describe('MyRequestsPanel', () => {
  it('labels a hand-entered determination as a proxy and never as a violation', async () => {
    invoke.mockImplementation((command: string, args: { claimId?: string }) => {
      if (command === 'list_claimed_requests') return Promise.resolve([claim('a')]);
      if (command === 'list_saved_portals') return Promise.resolve([]);
      if (command === 'get_request_deadline') {
        return Promise.resolve(
          deadline(args.claimId!, {
            countdown: { kind: 'DETERMINATION_RECORDED', days: null },
            determinationBasis: 'EARLIEST_RECORDED_ACTIVITY_PROXY',
          }),
        );
      }
      return Promise.reject(new Error(command));
    });
    render(<MyRequestsPanel onClose={() => {}} />);
    const chip = await screen.findByText(/your entry, proxy/);
    expect(chip.closest('[data-countdown]')?.getAttribute('data-countdown')).toBe('DETERMINATION_RECORDED');
    expect(screen.queryByText(/violation/i)).toBeNull();
  });

  it('cannot confirm a suggested extension notice until a statutory reason is chosen', async () => {
    const suggested = claim('a', {
      suggestions: [
        {
          id: 's1',
          kind: 'EXTENSION_NOTICE',
          candidateOccurredAt: '2026-01-05T09:30:00-08:00',
          suggestedAt: '2026-01-06T00:00:00Z',
          evidence: {
            messageIdHash: 'h',
            subjectExcerpt: null,
            emailDate: '2026-01-05T09:30:00-08:00',
            excerpt: 'We need an extension.',
            sentenceStart: 0,
            sentenceEnd: 21,
            matchedSentence: 'We need an extension.',
          },
        },
      ],
    });
    invoke.mockImplementation((command: string, args: { claimId?: string }) => {
      if (command === 'list_claimed_requests') return Promise.resolve([suggested]);
      if (command === 'list_saved_portals') return Promise.resolve([]);
      if (command === 'get_request_deadline') return Promise.resolve(deadline(args.claimId!));
      return Promise.reject(new Error(command));
    });
    render(<MyRequestsPanel onClose={() => {}} />);
    fireEvent.click(await screen.findByRole('button', { name: /ref-a/ }));
    const confirm = await screen.findByRole('button', { name: 'Confirm this event' });
    expect((confirm as HTMLButtonElement).disabled).toBe(true);
    fireEvent.change(screen.getAllByRole('combobox').find((el) => (el as HTMLSelectElement).value === '' )!, {
      target: { value: 'VOLUMINOUS_RECORDS' },
    });
    expect((confirm as HTMLButtonElement).disabled).toBe(false);
  });
});
