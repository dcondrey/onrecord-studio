import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';

interface SyncState {
  keys: Readonly<Record<string, string | null>>;
  set(group: string, key: string | null): void;
}

const SyncContext = createContext<SyncState | null>(null);

/**
 * Shares the crosshair's active key across aligned panels (small multiples, filer finance panels),
 * so pointing at a period in one panel marks the same period in each panel of its group.
 */
export function ChartSync({ children }: { children: ReactNode }) {
  const [keys, setKeys] = useState<Record<string, string | null>>({});
  const set = useCallback((group: string, key: string | null) => {
    setKeys((current) => (current[group] === key ? current : { ...current, [group]: key }));
  }, []);
  const value = useMemo(() => ({ keys, set }), [keys, set]);
  return <SyncContext.Provider value={value}>{children}</SyncContext.Provider>;
}

/** The active key of a sync group, or null outside a ChartSync. */
export function useSyncedKey(group: string | undefined): [string | null, ((key: string | null) => void) | null] {
  const sync = useContext(SyncContext);
  if (!sync || !group) return [null, null];
  return [sync.keys[group] ?? null, (key) => sync.set(group, key)];
}
