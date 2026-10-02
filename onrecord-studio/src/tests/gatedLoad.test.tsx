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

  it("never renders the previous key's answer under a new key", async () => {
    const answers: Record<string, string> = { a: 'about a', b: 'about b' };
    const seen: Array<{ key: string; status: string; data?: string }> = [];
    const { rerender } = renderHook(({ key }) => {
      const state = useGatedLoad(key, () => Promise.resolve(answers[key]));
      seen.push({ key, status: state.status, data: state.status === 'ready' ? state.data : undefined });
      return state;
    }, { initialProps: { key: 'a' } });
    await waitFor(() => expect(seen.at(-1)).toEqual({ key: 'a', status: 'ready', data: 'about a' }));
    rerender({ key: 'b' });
    await waitFor(() => expect(seen.at(-1)).toEqual({ key: 'b', status: 'ready', data: 'about b' }));
    expect(seen.filter((render) => render.data !== undefined && render.data !== answers[render.key])).toEqual([]);
  });
});
