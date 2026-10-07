import { useState } from 'react';
import { open as chooseFolder } from '@tauri-apps/plugin-dialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DataPackFileSummary, DataPackImportOutcome, DataPackSummary } from '../../types';
import Icon from '../ui/icons';

const IMPORTING = 'Importing files';

function fileLabel(file: DataPackFileSummary): string {
  const period = file.year === null ? 'All published years' : String(file.year);
  const megabytes = (file.bytes / 1_000_000).toLocaleString(undefined, { maximumFractionDigits: 1 });
  return `${period} · ${file.rows.toLocaleString()} rows · ${megabytes} MB · retrieved ${file.retrievedAt.slice(0, 10)}${file.partial ? ' · period not over, later rows may be published' : ''}`;
}

function outcomeMessage(outcome: DataPackImportOutcome): string {
  const result = outcome.result;
  if (!result) return outcome.error ?? 'Not imported.';
  const retrieved = `retrieved by the pack builder on ${outcome.retrievedAt.slice(0, 10)}`;
  if (outcome.alreadyStored) {
    return `Nothing imported: these bytes are already stored as ${result.tableName} (${result.importedRows.toLocaleString()} rows). This pack, ${retrieved}, is recorded as another origin of that snapshot.`;
  }
  const rejected = result.rejectedRows > 0 ? ` ${result.rejectedRows.toLocaleString()} rows rejected.` : '';
  if (result.importedRows === 0) return `No rows imported: the file, ${retrieved}, holds no readable rows.${rejected}`;
  return `${result.importedRows.toLocaleString()} rows imported to ${result.tableName}, ${retrieved}.${rejected}`;
}

export default function DataPackImport({ onImported }: { onImported?: () => void }) {
  const gate = useLatestRequestGate();
  const [folder, setFolder] = useState('');
  const [summary, setSummary] = useState<DataPackSummary | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [outcomes, setOutcomes] = useState<DataPackImportOutcome[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async <T,>(label: string, operation: () => Promise<T>, apply: (value: T) => void) => {
    const request = gate.begin();
    setBusy(label);
    setError(null);
    try {
      const value = await operation();
      if (gate.isCurrent(request)) apply(value);
    } catch (reason) {
      if (gate.isCurrent(request)) setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(request)) setBusy(null);
    }
  };

  const pickFolder = async () => {
    const chosen = await chooseFolder({ directory: true, multiple: false });
    if (typeof chosen !== 'string') return;
    gate.invalidate();
    setFolder(chosen);
    setSummary(null);
    setSelected(new Set());
    setOutcomes(null);
    await run('Verifying pack', () => invokeCommand('inspect_data_pack', { path: chosen }), setSummary);
  };

  const toggle = (path: string) => setSelected((current) => {
    const next = new Set(current);
    if (!next.delete(path)) next.add(path);
    return next;
  });

  const importSelected = (inspected: DataPackSummary) => {
    const files = [...selected];
    void run(IMPORTING, async () => {
      try {
        return await invokeCommand('import_data_pack_files', { path: folder, manifestSha256: inspected.manifestSha256, files });
      } finally {
        // Snapshots may have been written even if this result is no longer shown.
        onImported?.();
      }
    }, setOutcomes);
  };

  return (
    <section className="app-panel p-4">
      <div className="mb-3 flex items-center gap-2">
        <Icon name="shield" className="h-4 w-4 text-accent" />
        <h2 className="text-sm font-semibold text-fg">Data pack import</h2>
      </div>
      <p className="mb-3 text-label leading-relaxed text-muted">Import tables from a signed OnRecord data pack. The pack is checked against the publisher key built into this app, and every file against its SHA-256, before anything is imported.</p>
      <div className="flex items-center gap-2">
        <button type="button" className="analysis-button" disabled={busy === IMPORTING} onClick={() => void pickFolder()}>Choose pack folder</button>
        {folder && <span className="min-w-0 truncate font-mono text-caption text-muted" title={folder}>{folder}</span>}
      </div>
      {busy && <p className="mt-2 text-label text-muted" role="status">{busy}</p>}
      {error && <p className="mt-2 text-label text-danger" role="alert">{error}</p>}
      {summary && (
        <div className="mt-3 space-y-3 text-label leading-relaxed">
          <div>
            <p className="font-semibold text-fg">{summary.pack} {summary.packVersion}</p>
            <p className="text-fg-secondary">Verified: signed by publisher key {summary.signerKeyId.slice(0, 16)}, and every file matches its SHA-256. Built {summary.builtAt}; schema {summary.schemaVersion}.</p>
            <p className="text-muted">Each file was retrieved by the pack builder on the date it lists, not by you. Anything published after that is not in it.</p>
          </div>
          <div className="rounded border border-line p-2">
            <p className="text-fg-secondary">Licence: {summary.licenceId} ({summary.licenceUrl})</p>
            <p className="mt-1 text-muted">{summary.disclaimer}</p>
          </div>
          {summary.tables.map((table) => (
            <fieldset key={table.table} className="space-y-1">
              <legend className="font-semibold text-fg">{table.table}</legend>
              {table.files.map((file) => (
                <label key={file.path} className="flex items-center gap-2 text-fg-secondary">
                  <input type="checkbox" disabled={file.datasetKind === null} checked={selected.has(file.path)} onChange={() => toggle(file.path)} />
                  {fileLabel(file)}{file.datasetKind === null && ' · this app does not import this file'}
                </label>
              ))}
            </fieldset>
          ))}
          <button type="button" className="analysis-button w-fit" disabled={Boolean(busy) || selected.size === 0} onClick={() => importSelected(summary)}>
            Import {selected.size === 1 ? '1 file' : `${selected.size} files`}
          </button>
        </div>
      )}
      {outcomes && summary && (
        <ul className="mt-3 space-y-1 text-label leading-relaxed" aria-label="Data pack import results">
          {outcomes.map((outcome) => (
            <li key={outcome.path}>
              <span className="font-semibold text-fg">{outcome.table}{outcome.year === null ? '' : ` ${outcome.year}`}: </span>
              <span className={outcome.result ? 'text-fg-secondary' : 'text-danger'}>{outcomeMessage(outcome)}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
