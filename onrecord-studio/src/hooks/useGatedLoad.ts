import { useEffect, useState } from 'react';
import { formatNativeError } from '../ipc';
import { useLatestRequestGate } from './useLatestRequestGate';

export type LoadState<T> = { status: 'pending' } | { status: 'ready'; data: T } | { status: 'failed'; message: string };

/**
 * Runs `load` whenever `key` changes and publishes only the result of the latest run. A result
 * for an earlier key is dropped, so a slow answer about one record never lands on another.
 */
export function useGatedLoad<T>(key: string, load: () => Promise<T>): LoadState<T> {
  const [state, setState] = useState<LoadState<T>>({ status: 'pending' });
  const gate = useLatestRequestGate();

  useEffect(() => {
    const token = gate.begin();
    setState({ status: 'pending' });
    load()
      .then((data) => { if (gate.isCurrent(token)) setState({ status: 'ready', data }); })
      .catch((reason) => { if (gate.isCurrent(token)) setState({ status: 'failed', message: formatNativeError(reason) }); });
    return gate.invalidate;
    // `load` is rebuilt each render; `key` names what it loads.
  }, [key, gate]); // eslint-disable-line react-hooks/exhaustive-deps

  return state;
}
