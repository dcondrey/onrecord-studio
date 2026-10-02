import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { CorpusEntityKind } from '../../types/corpus';
import type { RegistryAnchor } from '../../types/registry';
import { describeRegistryAnchor } from '../../utils/registryAnchor';

type Lookup = { kind: 'pending' } | { kind: 'failed'; message: string } | { kind: 'answered'; anchor: RegistryAnchor };

/** What the imported registry snapshot says about one extracted identifier value. */
export function RegistryAnchorNote({ identifierKind, value }: { identifierKind: CorpusEntityKind; value: string }) {
  const gate = useLatestRequestGate();
  const [lookup, setLookup] = useState<Lookup>({ kind: 'pending' });

  useEffect(() => {
    const token = gate.begin();
    setLookup({ kind: 'pending' });
    invokeCommand('lookup_registry_anchor', { identifierKind, value })
      .then((anchor) => {
        if (gate.isCurrent(token)) setLookup({ kind: 'answered', anchor });
      })
      .catch((reason: unknown) => {
        if (gate.isCurrent(token)) setLookup({ kind: 'failed', message: formatNativeError(reason) });
      });
  }, [gate, identifierKind, value]);

  if (lookup.kind === 'pending') return <p className="text-label text-muted" aria-busy="true">Checking the registry snapshot.</p>;
  if (lookup.kind === 'failed') return <p className="text-label text-warning">Registry lookup failed: {lookup.message}</p>;
  const registered = lookup.anchor.state === 'REGISTERED';
  return (
    <p className={`text-label ${registered ? 'text-fg' : 'text-muted'}`} data-registry-state={lookup.anchor.state}>
      {describeRegistryAnchor(lookup.anchor)}
      {lookup.anchor.state === 'REGISTERED' && lookup.anchor.record.detail && (
        <span className="block text-muted">{lookup.anchor.record.detail}</span>
      )}
    </p>
  );
}
