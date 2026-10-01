// @vitest-environment jsdom
import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { useResolutionClusters } from '../hooks/useResolutionClusters';
import type { ResolutionClusterView } from '../types';

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand, formatNativeError: String }));

const view = (entityId: string): ResolutionClusterView => ({
  entityId,
  key: { kind: 'PERSON', value: entityId },
  members: [],
  reasons: [],
  contradictions: [],
  resolutionId: `resolution-${entityId}`,
  ledgerHead: 'head',
  corpusStateDigest: 'digest',
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

beforeEach(() => invokeCommand.mockReset());

describe('useResolutionClusters', () => {
  it('discards a result for an entity the caller has already moved off', async () => {
    const first = deferred<ResolutionClusterView | null>();
    const second = deferred<ResolutionClusterView | null>();
    invokeCommand.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const { result, rerender } = renderHook(({ id }) => useResolutionClusters(id), { initialProps: { id: 'a' as string | null } });
    rerender({ id: 'b' });
    second.resolve(view('b'));
    await waitFor(() => expect(result.current.status).toBe('ready'));
    await act(async () => {
      first.resolve(view('a'));
    });
    expect(result.current).toMatchObject({ status: 'ready', entityId: 'b' });
  });

  it('says an unknown entity is gone rather than showing an empty cluster', async () => {
    invokeCommand.mockResolvedValueOnce(null);
    const { result } = renderHook(() => useResolutionClusters('rebuilt-away'));
    await waitFor(() => expect(result.current.status).toBe('gone'));
  });
});
