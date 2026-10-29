import { useEffect, useState } from 'react';
import { formatNativeError } from '../ipc';
import { useLatestRequestGate } from './useLatestRequestGate';

export type LoadState<T> = { status: 'pending' } | { status: 'ready'; data: T } | { status: 'failed'; message: string };

const PENDING = { status: 'pending' } as const;

/**
 * Runs `load` whenever `key` changes and publishes only the result of the latest run. A result
 * for an earlier key is dropped, so a slow answer about one record never lands on another.
 */
export function useGatedLoad<T>(key: string, load: () => Promise<T>, cached?: T): LoadState<T> {
  const [held, setHeld] = useState<{ key: string; state: LoadState<T> } | null>(null);
  const gate = useLatestRequestGate();

  useEffect(() => {
    const token = gate.begin();
    setHeld(null);
    load()
      .then((data) => { if (gate.isCurrent(token)) setHeld({ key, state: { status: 'ready', data } }); })
      .catch((reason) => { if (gate.isCurrent(token)) setHeld({ key, state: { status: 'failed', message: formatNativeError(reason) } }); });
    return gate.invalidate;
    // `load` is rebuilt each render; `key` names what it loads.
  }, [key, gate]);

  // The render that first sees a new key runs before the effect above clears the old answer.
  return held !== null && held.key === key ? held.state : cached !== undefined ? { status: 'ready', data: cached } : PENDING;
}
