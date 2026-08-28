import { useCallback, useEffect, useMemo, useRef } from 'react';

/**
 * Issues monotonically increasing request tokens and invalidates them on unmount.
 *
 * Native IPC calls cannot currently be aborted. Components must therefore check a token before
 * publishing a result, an error, or a loading-state transition. Keeping that policy here prevents
 * a slower, superseded request from making a false statement about the currently selected record.
 */
export function useLatestRequestGate() {
  const sequence = useRef(0);

  const begin = useCallback(() => {
    sequence.current += 1;
    return sequence.current;
  }, []);

  const isCurrent = useCallback((token: number) => sequence.current === token, []);

  const invalidate = useCallback(() => {
    sequence.current += 1;
  }, []);

  useEffect(() => invalidate, [invalidate]);

  return useMemo(() => ({ begin, isCurrent, invalidate } as const), [begin, invalidate, isCurrent]);
}
