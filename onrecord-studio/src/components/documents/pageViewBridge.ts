import { useSyncExternalStore } from 'react';
import type { PageRectangle, PageRegionState } from '../../types';

/** A rectangle the reader has drawn and not yet saved, with the page text found inside it. */
export interface PendingRegion {
  rectangle: PageRectangle;
  /** Text layer text inside the rectangle. Empty over an image or a scan. */
  quote: string;
}

export interface PageViewState {
  /** The page view is mounted for this document. */
  open: boolean;
  pending: PendingRegion | null;
  states: Readonly<Record<string, PageRegionState>>;
  selectedId: string | null;
  /** Bumped by whoever saves or deletes a mark, so the page view reads the list again. */
  revision: number;
  /** Bumped when a list entry asks the page view to show its region. */
  jump: number;
}

const initial: PageViewState = { open: false, pending: null, states: {}, selectedId: null, revision: 0, jump: 0 };

const stores = new Map<string, { state: PageViewState; listeners: Set<() => void> }>();

function entry(key: string) {
  let found = stores.get(key);
  if (!found) {
    found = { state: initial, listeners: new Set() };
    stores.set(key, found);
  }
  return found;
}

/**
 * The page view and the annotations panel are siblings in the viewer and neither owns the other.
 * What they share is held per document, so state for one document is never shown on another.
 */
export const pageViewBridge = {
  update(key: string, change: Partial<PageViewState> | ((state: PageViewState) => Partial<PageViewState>)) {
    const target = entry(key);
    target.state = { ...target.state, ...(typeof change === 'function' ? change(target.state) : change) };
    target.listeners.forEach((listener) => listener());
  },
  reset(key: string) {
    const target = stores.get(key);
    if (!target) return;
    target.state = initial;
    target.listeners.forEach((listener) => listener());
  },
  get: (key: string): PageViewState => entry(key).state,
};

export function usePageViewState(key: string): PageViewState {
  return useSyncExternalStore(
    (listener) => {
      const target = entry(key);
      target.listeners.add(listener);
      return () => target.listeners.delete(listener);
    },
    () => entry(key).state,
  );
}

export const pageViewKey = (portalKey: string, documentId: number) => `${portalKey}:${documentId}`;
