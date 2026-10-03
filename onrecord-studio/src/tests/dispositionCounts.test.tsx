// @vitest-environment jsdom
//
// Disposition counts are statements about the selected portal's departments. A count for the
// portal the reader just left must never render under the portal they moved to.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';
import type { DepartmentDispositions } from '../types';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before the store import.
vi.hoisted(() => {
  const entries = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => entries.get(key) ?? null,
      setItem: (key: string, value: string) => void entries.set(key, String(value)),
      removeItem: (key: string) => void entries.delete(key),
      clear: () => entries.clear(),
      key: (index: number) => [...entries.keys()][index] ?? null,
      get length() {
        return entries.size;
      },
    },
  });
});

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock('../ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../ipc')>()),
  invokeCommand,
}));

import DispositionCounts from '../components/dashboard/DispositionCounts';

function department(portalSubdomain: string, agencyName: string): DepartmentDispositions {
  return {
    portalSubdomain,
    agencyId: 1,
    agencyName,
    totalRequests: 1,
    buckets: [{ disposition: 'NO_RECORDS', requests: [{ requestId: 1, rpid: '24-1' }] }],
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

afterEach(() => {
  cleanup();
  invokeCommand.mockReset();
});

describe('DispositionCounts', () => {
  it('drops a response for a portal that is no longer selected', async () => {
    const first = deferred<DepartmentDispositions[]>();
    const second = deferred<DepartmentDispositions[]>();
    invokeCommand.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);

    const { rerender } = render(<DispositionCounts portalSubdomain="city-a" />);
    rerender(<DispositionCounts portalSubdomain="city-b" />);
    expect(invokeCommand).toHaveBeenNthCalledWith(2, 'get_department_dispositions', { portalSubdomain: 'city-b' });

    await act(async () => {
      second.resolve([department('city-b', 'Parks')]);
      await second.promise;
    });
    await act(async () => {
      first.resolve([department('city-a', 'Police')]);
      await first.promise;
    });

    expect(screen.getByText('Parks')).toBeTruthy();
    expect(screen.queryByText('Police')).toBeNull();
  });
});
