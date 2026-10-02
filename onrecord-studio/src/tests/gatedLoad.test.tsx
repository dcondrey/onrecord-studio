// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { useGatedLoad } from '../hooks/useGatedLoad';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe('useGatedLoad', () => {
  it('never publishes a result for a key the caller has moved past', async () => {
    const slow = deferred<string>();
    const fast = deferred<string>();
    const loads: Record<string, Promise<string>> = { a: slow.promise, b: fast.promise };
    const { result, rerender } = renderHook(({ key }) => useGatedLoad(key, () => loads[key]), { initialProps: { key: 'a' } });
    rerender({ key: 'b' });
    await act(async () => { fast.resolve('about b'); });
    await waitFor(() => expect(result.current).toEqual({ status: 'ready', data: 'about b' }));
    await act(async () => { slow.resolve('about a'); });
    expect(result.current).toEqual({ status: 'ready', data: 'about b' });
  });
});
