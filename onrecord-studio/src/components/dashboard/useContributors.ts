import { useCallback, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { LocalContributorIdentity, TrustedContributor } from '../../types';

export type ContributorsState =
  | { status: 'loading' }
  | { status: 'failed'; error: string }
  | { status: 'ready'; contributors: TrustedContributor[]; identity: LocalContributorIdentity | null };

/** The contributor trust list and this installation's own key id, shared by the sharing panel and the import preview. */
export function useContributors() {
  const gate = useLatestRequestGate();
  const [state, setState] = useState<ContributorsState>({ status: 'loading' });

  const reload = useCallback(() => {
    const request = gate.begin();
    Promise.all([
      invokeCommand('list_trusted_contributors', {}),
      invokeCommand('get_local_contributor_identity', {}).catch(() => null),
    ])
      .then(([contributors, identity]) => {
        if (gate.isCurrent(request)) setState({ status: 'ready', contributors, identity });
      })
      .catch((reason) => {
        if (gate.isCurrent(request)) setState({ status: 'failed', error: formatNativeError(reason) });
      });
  }, [gate]);

  useEffect(() => {
    reload();
  }, [reload]);

  const trust = useCallback(
    async (keyId: string, label: string) => {
      await invokeCommand('trust_contributor', { keyId, label });
      reload();
    },
    [reload],
  );
  const untrust = useCallback(
    async (keyId: string) => {
      await invokeCommand('untrust_contributor', { keyId });
      reload();
    },
    [reload],
  );

  return { state, reload, trust, untrust };
}

/** Eight groups of eight, so two people can read a key to each other and compare. */
export function groupKeyId(keyId: string): string {
  return keyId.match(/.{1,8}/g)?.join(' ') ?? keyId;
}
