import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  AddTimelineEventInput,
  ClaimedRequest,
  ClaimRequestInput,
  ConfirmSuggestedEventInput,
  EmailImportReport,
  RequestDeadline,
  UpdateClaimedRequestInput,
} from '../../types';

export interface ImportOutcome {
  claimId: string;
  report: EmailImportReport;
}

/**
 * Claims and their deadlines. Every fetch takes a token from the gate and publishes only if it is
 * still the latest: a refresh that started before a newer one (after an edit, a removal or an
 * import) must not overwrite what the newer one showed, because these are statements about a
 * request's dates.
 */
export function useMyRequests(enabled: boolean) {
  const refreshGate = useLatestRequestGate();
  const importGate = useLatestRequestGate();
  const mutateGate = useLatestRequestGate();
  const [claims, setClaims] = useState<ClaimedRequest[]>([]);
  const [deadlines, setDeadlines] = useState<Record<string, RequestDeadline>>({});
  const [error, setError] = useState<string | null>(null);
  // Until the first list answers, there is no list: never "you track nothing".
  const [status, setStatus] = useState<'loading' | 'ready' | 'failed'>('loading');
  const [isBusy, setIsBusy] = useState(false);
  const [importOutcome, setImportOutcome] = useState<ImportOutcome | null>(null);

  const refresh = useCallback(async () => {
    const token = refreshGate.begin();
    setStatus((current) => (current === 'failed' ? 'loading' : current));
    try {
      const list = await invokeCommand('list_claimed_requests', {});
      const entries = await Promise.all(
        list.map(async (claim) => [claim.id, await invokeCommand('get_request_deadline', { claimId: claim.id })] as const),
      );
      if (!refreshGate.isCurrent(token)) return;
      setClaims(list);
      setDeadlines(Object.fromEntries(entries));
      setError(null);
      setStatus('ready');
    } catch (reason) {
      if (!refreshGate.isCurrent(token)) return;
      setError(formatNativeError(reason));
      setStatus((current) => (current === 'ready' ? 'ready' : 'failed'));
    }
  }, [refreshGate]);

  useEffect(() => {
    if (enabled) void refresh();
  }, [enabled, refresh]);

  const mutate = useCallback(
    async <T,>(operation: () => Promise<T>): Promise<T | null> => {
      const token = mutateGate.begin();
      setIsBusy(true);
      setError(null);
      try {
        const result = await operation();
        await refresh();
        return result;
      } catch (reason) {
        if (!mutateGate.isCurrent(token)) return null;
        setError(String(reason instanceof Error ? reason.message : reason));
        return null;
      } finally {
        if (mutateGate.isCurrent(token)) setIsBusy(false);
      }
    },
    [mutateGate, refresh],
  );

  const importEmails = useCallback(
    async (claimId: string, paths: string[]) => {
      const token = importGate.begin();
      setIsBusy(true);
      setError(null);
      try {
        const report = await invokeCommand('import_request_emails', { input: { claimId, paths } });
        if (!importGate.isCurrent(token)) return;
        setImportOutcome({ claimId, report });
        await refresh();
      } catch (reason) {
        if (!importGate.isCurrent(token)) return;
        setError(String(reason instanceof Error ? reason.message : reason));
      } finally {
        setIsBusy(false);
      }
    },
    [importGate, refresh],
  );

  return {
    status,
    claims,
    deadlines,
    error,
    isBusy,
    importOutcome,
    refresh,
    claim: (input: ClaimRequestInput) => mutate(() => invokeCommand('claim_request', { input })),
    update: (input: UpdateClaimedRequestInput) => mutate(() => invokeCommand('update_claimed_request', { input })),
    remove: (claimId: string) => mutate(() => invokeCommand('remove_claimed_request', { claimId })),
    addEvent: (input: AddTimelineEventInput) => mutate(() => invokeCommand('add_timeline_event', { input })),
    removeEvent: (claimId: string, eventId: string) =>
      mutate(() => invokeCommand('remove_timeline_event', { input: { claimId, eventId } })),
    confirm: (input: ConfirmSuggestedEventInput) => mutate(() => invokeCommand('confirm_suggested_event', { input })),
    reject: (claimId: string, suggestionId: string) =>
      mutate(() => invokeCommand('reject_suggested_event', { input: { claimId, suggestionId } })),
    importEmails,
  };
}
