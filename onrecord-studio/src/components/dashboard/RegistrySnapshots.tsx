import { useEffect, useState } from 'react';
import { open as chooseFile } from '@tauri-apps/plugin-dialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { openHelp } from '../help/helpEvents';
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
    <details className="mt-4 text-label">
      <summary className="cursor-pointer text-fg-secondary">Public registry snapshots</summary>
      <div className="mt-3 space-y-3 text-muted">
        <p>
          Import a registry list you downloaded yourself so a licence number read from a document can be shown with the name it is registered to. Nothing is fetched; the file&apos;s digest is kept with every match.{' '}
          <button type="button" className="underline" onClick={() => openHelp('registry-anchoring')}>What a match means</button>
        </p>
        {REGISTRIES.map((registry) => {
          const snapshot = snapshots?.find((row) => row.registry === registry) ?? null;
          return (
            <div key={registry} className="rounded border border-line p-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-semibold text-fg-secondary">{REGISTRY_TITLES[registry]}</span>
                <button type="button" className="analysis-button" disabled={busy !== null} onClick={() => void importSnapshot(registry)}>
                  {busy === registry ? 'Importing…' : snapshot ? 'Replace snapshot' : 'Import CSV'}
                </button>
              </div>
              <p className="mt-1 break-all">
                {snapshots === null
                  ? 'Reading snapshots.'
                  : snapshot
                    ? `${describeSnapshot(snapshot)}${snapshot.skippedRowCount > 0 ? `; ${snapshot.skippedRowCount.toLocaleString('en-US')} rows skipped (no usable number or a repeat)` : ''}. Source: ${snapshot.sourcePath}`
                    : 'No snapshot imported. Licence numbers are shown as not checked.'}
              </p>
            </div>
          );
        })}
        {error && <p role="alert" className="text-warning">{error}</p>}
      </div>
    </details>
  );
}
