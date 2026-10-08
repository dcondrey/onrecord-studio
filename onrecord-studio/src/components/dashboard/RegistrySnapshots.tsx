import { useEffect, useState } from 'react';
import { open as chooseFile } from '@tauri-apps/plugin-dialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { openHelp } from '../help/helpEvents';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import type { RegistryKind, RegistrySnapshotSummary } from '../../types/registry';
import { describeSnapshot, REGISTRY_TITLES } from '../../utils/registryAnchor';

const REGISTRIES: RegistryKind[] = ['CSLB_LICENSE_MASTER'];

/** Imports an operator-downloaded public-registry list so extracted identifiers can be anchored to it. */
export default function RegistrySnapshots() {
  const gate = useLatestRequestGate();
  const [snapshots, setSnapshots] = useState<RegistrySnapshotSummary[] | null>(null);
  const [busy, setBusy] = useState<RegistryKind | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => {
    const token = gate.begin();
    invokeCommand('list_registry_snapshots', {})
      .then((rows) => {
        if (gate.isCurrent(token)) setSnapshots(rows);
      })
      .catch((reason: unknown) => {
        if (gate.isCurrent(token)) setError(formatNativeError(reason));
      });
  };
  useEffect(refresh, [gate]);

  const importSnapshot = async (registry: RegistryKind) => {
    const path = await chooseFile({ multiple: false, filters: [{ name: 'Registry list', extensions: ['csv', 'txt'] }] });
    if (typeof path !== 'string') return;
    setBusy(registry);
    setError(null);
    try {
      const summary = await invokeCommand('import_registry_snapshot', { registry, localPath: path });
      setSnapshots((current) => [...(current ?? []).filter((row) => row.registry !== registry), summary]);
    } catch (reason) {
      setError(formatNativeError(reason));
    } finally {
      setBusy(null);
    }
  };

  return (
    <section aria-labelledby="registry-snapshots-title" className="space-y-3 text-body text-muted">
        <h4 id="registry-snapshots-title" className="font-medium text-fg">Public registry snapshots</h4>
        <p>
          Import a registry list you downloaded so a license number in a document shows the name it is registered to.{' '}
          <Button variant="link" onClick={() => openHelp('registry-anchoring')}>What a match means</Button>
        </p>
        {REGISTRIES.map((registry) => {
          const snapshot = snapshots?.find((row) => row.registry === registry) ?? null;
          return (
            <div key={registry} className="rounded-lg border border-line p-3">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <span className="font-medium text-fg-secondary">{REGISTRY_TITLES[registry]}</span>
                <Button size="sm" disabled={busy !== null} onClick={() => void importSnapshot(registry)}>
                  {busy === registry ? 'Importing…' : snapshot ? 'Replace snapshot' : 'Import CSV'}
                </Button>
              </div>
              <p className="mt-1 break-all text-caption">
                {snapshots === null
                  ? error ? 'Imported snapshots could not be read.' : 'Reading snapshots.'
                  : snapshot
                    ? `${describeSnapshot(snapshot)}${snapshot.skippedRowCount > 0 ? `; ${snapshot.skippedRowCount.toLocaleString('en-US')} rows skipped (no usable number or a repeat)` : ''}. Source: ${snapshot.sourcePath}`
                    : 'No snapshot imported. License numbers are shown as not checked.'}
              </p>
            </div>
          );
        })}
        {error && <InlineError summary="The registry snapshot couldn’t be imported." detail={error} />}
    </section>
  );
}
