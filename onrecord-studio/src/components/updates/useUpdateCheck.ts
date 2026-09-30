import { useCallback, useEffect, useRef, useState } from 'react';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { checkForUpdate, describeUpdateError, isTauriRuntime, type PendingUpdate } from './updateService';

export const CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000;

export type UpdateState =
  | { phase: 'idle' }
  | { phase: 'available'; update: PendingUpdate }
  | { phase: 'installing'; update: PendingUpdate }
  | { phase: 'error'; message: string; update: PendingUpdate | null };

export function useUpdateCheck(intervalMs: number = CHECK_INTERVAL_MS) {
  const gate = useLatestRequestGate();
  const [state, setState] = useState<UpdateState>({ phase: 'idle' });
  const [dismissed, setDismissed] = useState<string | null>(null);
  const installing = useRef(false);

  const check = useCallback(async () => {
    if (installing.current) return;
    const token = gate.begin();
    try {
      const update = await checkForUpdate();
      if (!gate.isCurrent(token)) return;
      setState(update ? { phase: 'available', update } : { phase: 'idle' });
    } catch (error) {
      if (!gate.isCurrent(token)) return;
      // A failed background check leaves an already-offered update in place.
      setState((current) => ({
        phase: 'error',
        message: describeUpdateError(error),
        update: current.phase === 'available' ? current.update : null,
      }));
    }
  }, [gate]);

  const install = useCallback(async () => {
    const update = state.phase === 'available' || state.phase === 'error' ? state.update : null;
    if (!update) return;
    const token = gate.begin();
    installing.current = true;
    setState({ phase: 'installing', update });
    try {
      await update.install();
    } catch (error) {
      installing.current = false;
      if (!gate.isCurrent(token)) return;
      setState({ phase: 'error', message: describeUpdateError(error), update });
    }
  }, [gate, state]);

  useEffect(() => {
    if (!isTauriRuntime()) return undefined;
    void check();
    const timer = window.setInterval(() => void check(), intervalMs);
    return () => window.clearInterval(timer);
  }, [check, intervalMs]);

  const later = useCallback(() => {
    if (state.phase === 'available' || state.phase === 'error') {
      setDismissed(state.update?.version ?? '');
    }
  }, [state]);

  return { state, dismissed, install, later, check } as const;
}
