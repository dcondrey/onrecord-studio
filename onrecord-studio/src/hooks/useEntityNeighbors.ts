import { useCallback, useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../ipc';
import type { CorpusEntityEdge, CorpusEntityNode, EntityNeighborCounts, EntityNeighborFilter, EntityNeighborPage } from '../types';
import { useLatestRequestGate } from './useLatestRequestGate';

/** Rows per request. A page is a unit of transfer; the total comes from the count. */
export const NEIGHBOR_PAGE_SIZE = 200;
/** Pages one Load all reads before stopping to offer a continue, so a hub cannot hold the index indefinitely. */
export const LOAD_ALL_PAGE_GUARD = 20;

export type NeighborList =
  | { status: 'loading' }
  | { status: 'missing' }
  | { status: 'failed' }
  | {
      status: 'ready';
      centre: CorpusEntityNode;
      edges: CorpusEntityEdge[];
      nextCursor: string | null;
      /** `null` until the count answers, or when it failed: an unknown total is never shown as zero. */
      counts: EntityNeighborCounts | null;
      countFailed: boolean;
      loadingMore: boolean;
      moreError: string | null;
      /** Load all stopped at its page guard with more remaining. */
      guardStopped: boolean;
    };

type ReadyList = Extract<NeighborList, { status: 'ready' }>;

interface Held {
  /** The entity and filter the list answers. An answer for any other identity is not a statement about this one. */
  identity: string;
  list: NeighborList;
}

/**
 * Every neighbour of one entity, a page at a time, for one filter. The identity of a list is the
 * entity plus the filter; anything that resolves for a different identity is discarded, so a slow
 * page for the entity the reader left can never be shown as the current entity's relationships.
 */
export function useEntityNeighbors(entityId: string | null, filter: EntityNeighborFilter) {
  const gate = useLatestRequestGate();
  const filterKey = JSON.stringify(filter);
  const identity = entityId === null ? null : `${entityId}\u0000${filterKey}`;
  const [held, setHeld] = useState<Held | null>(null);
  const [countHeld, setCountHeld] = useState<{ identity: string; counts: EntityNeighborCounts | null } | null>(null);
  const live = useRef({ token: 0, inFlight: false, identity: null as string | null });
  const heldRef = useRef<Held | null>(null);
  const centreRead = useRef<{ entityId: string; centre: CorpusEntityNode } | null>(null);
  heldRef.current = held;

  useEffect(() => {
    if (entityId === null || identity === null) return;
    const token = gate.begin();
    live.current = { token, inFlight: false, identity };
    const args = { entityId, filter: JSON.parse(filterKey) as EntityNeighborFilter };
    void invokeCommand('page_corpus_entity_neighbors', { ...args, pageSize: NEIGHBOR_PAGE_SIZE })
      .then((page) => {
        if (!gate.isCurrent(token)) return;
        setHeld({
          identity,
          list: page
            ? {
                status: 'ready',
                centre: page.center,
                edges: page.edges,
                nextCursor: page.nextCursor,
                counts: null,
                countFailed: false,
                loadingMore: false,
                moreError: null,
                guardStopped: false,
              }
            : { status: 'missing' },
        });
      })
      .catch(() => {
        if (gate.isCurrent(token)) setHeld({ identity, list: { status: 'failed' } });
      });
    // Held apart from the list: the count and the first page answer in either order.
    void invokeCommand('count_corpus_entity_neighbors', args)
      .then((counts) => {
        if (gate.isCurrent(token)) setCountHeld({ identity, counts });
      })
      .catch(() => {
        if (gate.isCurrent(token)) setCountHeld({ identity, counts: null });
      });
  }, [entityId, filterKey, gate, identity]);

  const patch = useCallback((forIdentity: string, token: number, change: (list: ReadyList) => ReadyList) => {
    if (!gate.isCurrent(token) || live.current.identity !== forIdentity) return;
    setHeld((current) =>
      current?.identity === forIdentity && current.list.status === 'ready'
        ? { identity: forIdentity, list: change(current.list) }
        : current,
    );
  }, [gate]);

  /** Reads up to `pages` further pages. Returns when the list ends, the guard is reached, or the list is replaced. */
  const read = useCallback(
    async (pages: number) => {
      const current = heldRef.current;
      const { token, identity: forIdentity } = live.current;
      if (!current || current.identity !== forIdentity || current.list.status !== 'ready' || live.current.inFlight) return;
      if (current.list.nextCursor === null || entityId === null) return;
      live.current.inFlight = true;
      patch(forIdentity, token, (list) => ({ ...list, loadingMore: true, moreError: null, guardStopped: false }));
      let cursor: string | null = current.list.nextCursor;
      try {
        for (let index = 0; index < pages && cursor !== null; index += 1) {
          const page: EntityNeighborPage | null = await invokeCommand('page_corpus_entity_neighbors', {
            entityId,
            filter: JSON.parse(filterKey) as EntityNeighborFilter,
            cursor,
            pageSize: NEIGHBOR_PAGE_SIZE,
          });
          if (!gate.isCurrent(token)) return;
          const next: string | null = page?.nextCursor ?? null;
          const more = page?.edges ?? [];
          cursor = next;
          patch(forIdentity, token, (list) => {
            const held = new Set(list.edges.map((edge) => edge.neighbor.entityId));
            return {
              ...list,
              edges: [...list.edges, ...more.filter((edge) => !held.has(edge.neighbor.entityId))],
              nextCursor: next,
            };
          });
        }
        patch(forIdentity, token, (list) => ({ ...list, loadingMore: false, guardStopped: cursor !== null && pages > 1 }));
      } catch (reason) {
        patch(forIdentity, token, (list) => ({ ...list, loadingMore: false, moreError: String(reason) }));
      } finally {
        if (gate.isCurrent(token)) live.current.inFlight = false;
      }
    },
    [entityId, filterKey, gate, patch],
  );

  const loadMore = useCallback(() => read(1), [read]);
  const loadAll = useCallback(() => read(LOAD_ALL_PAGE_GUARD), [read]);

  const answered: NeighborList = held !== null && held.identity === identity ? held.list : { status: 'loading' };
  const counted = countHeld !== null && countHeld.identity === identity ? countHeld : null;
  const list: NeighborList =
    answered.status === 'ready'
      ? { ...answered, counts: counted?.counts ?? null, countFailed: counted !== null && counted.counts === null }
      : answered;
  // The centre belongs to the entity, not to the filter, so changing a filter does not blank the
  // graph while the filtered list reloads.
  if (list.status === 'ready' && entityId !== null) centreRead.current = { entityId, centre: list.centre };
  const centre = centreRead.current?.entityId === entityId ? centreRead.current.centre : null;
  return { list, centre, loadMore, loadAll };
}
