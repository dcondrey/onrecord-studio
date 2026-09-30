import { create } from 'zustand';
import { invokeCommand } from '../ipc';
import type { StarterEntry, StarterSet } from '../types';
import { addStarterEntries, startArgsForEntry, type StarterEntryOutcome } from '../utils/starterSets';
import { useAppStore } from './useAppStore';

export type StarterEntryStatus = 'adding' | StarterEntryOutcome;

interface StarterRunState {
  runningSetId: string | null;
  statuses: Record<string, StarterEntryStatus>;
  run: (set: StarterSet, entries: readonly StarterEntry[]) => Promise<void>;
  /** Stops before the next source. The source being added when this is called still finishes. */
  cancel: () => void;
}

let cancelled = false;

/**
 * An add run lives here, not in a component. The empty-state panel that starts a run unmounts as
 * soon as the first source lands in the library, and a run tied to that component would stop
 * there and silently drop the sources the operator confirmed.
 */
export const useStarterRunStore = create<StarterRunState>((set, get) => ({
  runningSetId: null,
  statuses: {},
  run: async (starterSet, entries) => {
    if (get().runningSetId !== null || entries.length === 0) return;
    cancelled = false;
    set((state) => ({
      runningSetId: starterSet.id,
      statuses: { ...state.statuses, ...Object.fromEntries(entries.map((entry) => [entry.id, 'adding' as const])) },
    }));
    try {
      await addStarterEntries(entries, {
        isCurrent: () => !cancelled,
        start: (entry) => invokeCommand('start_data_source', startArgsForEntry(entry)),
        onResult: (entry, outcome, started) => {
          const app = useAppStore.getState();
          if (started) {
            app.upsertPortal(started.source);
            app.setPortalSyncState(started.source.portalKey, true, started.syncRunId);
            app.appendSyncLog(`${started.source.displayName}: added from the ${starterSet.title} starter set.`);
          } else if (outcome.state === 'unavailable') {
            app.appendSyncLog(`${entry.title}: unavailable from the ${starterSet.title} starter set. ${outcome.reason}`);
          }
          set((state) => ({ statuses: { ...state.statuses, [entry.id]: outcome } }));
        },
      });
    } finally {
      set((state) => ({
        runningSetId: null,
        // A source that was never attempted has no outcome; leaving it "adding" would claim work
        // that is not happening.
        statuses: Object.fromEntries(Object.entries(state.statuses).filter(([, status]) => status !== 'adding')),
      }));
    }
  },
  cancel: () => {
    cancelled = true;
  },
}));
