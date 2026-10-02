// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';

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
      get length() { return entries.size; },
    },
  });
});

const answer = vi.hoisted(() => ({ fail: false }));
vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string) => {
    if (command !== 'get_case_scope') return Promise.resolve(null);
    return answer.fail
      ? Promise.reject(new Error('scope unavailable'))
      : Promise.resolve({ caseId: 'case-a', documents: [], requests: [{ portalKey: 'p', requestId: 4 }], findingIds: ['f1'] });
  },
}));

import { useCaseScope } from '../hooks/useCaseScope';
import { useAppStore } from '../store/useAppStore';

afterEach(() => {
  cleanup();
  act(() => useAppStore.setState({ activeCaseId: null, scopeToCase: true }));
  answer.fail = false;
});

describe('useCaseScope', () => {
  it('is off without an active case or with scoping turned off', () => {
    const { result } = renderHook(() => useCaseScope());
    expect(result.current.status).toBe('off');
    act(() => useAppStore.setState({ activeCaseId: 'case-a', scopeToCase: false }));
    expect(result.current.status).toBe('off');
  });

  it('reports a failed scope instead of an empty one, so nothing is filtered away', async () => {
    answer.fail = true;
    act(() => useAppStore.setState({ activeCaseId: 'case-a', scopeToCase: true }));
    const { result } = renderHook(() => useCaseScope());
    await waitFor(() => expect(result.current.status).toBe('failed'));
  });

  it('exposes the case findings and requests once loaded', async () => {
    act(() => useAppStore.setState({ activeCaseId: 'case-a', scopeToCase: true }));
    const { result } = renderHook(() => useCaseScope());
    await waitFor(() => expect(result.current.status).toBe('ready'));
    const ready = result.current as Extract<ReturnType<typeof useCaseScope>, { status: 'ready' }>;
    expect([...ready.value.findingIds]).toEqual(['f1']);
    expect(ready.value.requestKeys.has('p\u00004')).toBe(true);
  });
});
