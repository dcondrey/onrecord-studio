import { useEffect, useState } from 'react';
import { open as chooseFolder } from '@tauri-apps/plugin-dialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DataPackFileSummary, DataPackImportOutcome, DataPackSummary, PublishedPack, PublishedPackSelection } from '../../types';
import Icon from '../ui/icons';

const IMPORTING = 'Importing files';
const DOWNLOADING = 'Downloading, verifying and importing';
const PARTIAL_NOTE = ' · period not over, later rows may be published';

function megabytes(bytes: number): string {
  return `${(bytes / 1_000_000).toLocaleString(undefined, { maximumFractionDigits: 1 })} MB`;
}

function period(year: number | null): string {
  return year === null ? 'All published years' : String(year);
}

function fileLabel(file: DataPackFileSummary): string {
  return `${period(file.year)} · ${file.rows.toLocaleString()} rows · ${megabytes(file.bytes)} · retrieved ${file.retrievedAt.slice(0, 10)}${file.partial ? PARTIAL_NOTE : ''}`;
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

/** Runs one request at a time on a gate: a result, error or busy state outliving it is dropped. */
function useGatedRun() {
  const gate = useLatestRequestGate();
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

  const reset = () => {
    gate.invalidate();
    setBusy(null);
    setError(null);
  };

  return { busy, error, run, reset };
}

function Licence({ summary }: { summary: DataPackSummary }) {
  return (
    <div className="rounded border border-line p-2">
      <p className="text-fg-secondary">Licence: {summary.licenceId} ({summary.licenceUrl})</p>
      <p className="mt-1 text-muted">{summary.disclaimer}</p>
    </div>
  );
}

function Outcomes({ outcomes, label }: { outcomes: DataPackImportOutcome[]; label: string }) {
  return (
    <ul className="mt-3 space-y-1 text-label leading-relaxed" aria-label={label}>
      {outcomes.map((outcome) => (
        <li key={outcome.path}>
          <span className="font-semibold text-fg">{outcome.table}{outcome.year === null ? '' : ` ${outcome.year}`}: </span>
          <span className={outcome.result ? 'text-fg-secondary' : 'text-danger'}>{outcomeMessage(outcome)}</span>
        </li>
      ))}
    </ul>
  );
}

function Status({ busy, error }: { busy: string | null; error: string | null }) {
  return (
    <>
      {busy && <p className="mt-2 text-label text-muted" role="status">{busy}</p>}
      {error && <p className="mt-2 text-label text-danger" role="alert">{error}</p>}
    </>
  );
}

/** One table and year of a published pack: what a selection names, with every file it covers. */
interface Choice {
  key: string;
  table: string;
  year: number | null;
  files: DataPackFileSummary[];
}

function choicesOf(summary: DataPackSummary): Choice[] {
  const choices: Choice[] = [];
  for (const table of summary.tables) {
    for (const file of table.files) {
      const key = `${table.table}\u0000${file.year ?? ''}`;
      const existing = choices.find((choice) => choice.key === key);
      if (existing) existing.files.push(file);
      else choices.push({ key, table: table.table, year: file.year, files: [file] });
    }
  }
  return choices;
}

function choiceLabel(choice: Choice): string {
  const rows = choice.files.reduce((sum, file) => sum + file.rows, 0);
  const bytes = choice.files.reduce((sum, file) => sum + file.bytes, 0);
  const count = choice.files.length > 1 ? ` · ${choice.files.length} files` : '';
  const partial = choice.files.some((file) => file.partial) ? PARTIAL_NOTE : '';
  return `${period(choice.year)} · ${rows.toLocaleString()} rows · ${megabytes(bytes)}${count}${partial}`;
}

const importable = (choice: Choice) => choice.files.every((file) => file.datasetKind !== null);

function PublishedPacks({ onImported }: { onImported?: () => void }) {
  const listGate = useLatestRequestGate();
  const { busy, error, run, reset } = useGatedRun();
  const [packs, setPacks] = useState<PublishedPack[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [chosenPack, setChosenPack] = useState('');
  const [summary, setSummary] = useState<DataPackSummary | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [outcomes, setOutcomes] = useState<DataPackImportOutcome[] | null>(null);

  useEffect(() => {
    const request = listGate.begin();
    void (async () => {
      try {
        const list = await invokeCommand('list_published_packs', {});
        if (listGate.isCurrent(request)) setPacks(list);
      } catch (reason) {
        if (listGate.isCurrent(request)) setListError(formatNativeError(reason));
      }
    })();
  }, [listGate]);

  const choosePack = (pack: string) => {
    reset();
    setChosenPack(pack);
    setSummary(null);
    setSelected(new Set());
    setOutcomes(null);
    if (pack) void run('Fetching the signed manifest', () => invokeCommand('fetch_published_pack_manifest', { pack }), setSummary);
  };

  const toggle = (key: string) => setSelected((current) => {
    const next = new Set(current);
    if (!next.delete(key)) next.add(key);
    return next;
  });

  const choices = summary ? choicesOf(summary) : [];
  const chosen = choices.filter((choice) => selected.has(choice.key));
  const chosenBytes = chosen.reduce((sum, choice) => sum + choice.files.reduce((total, file) => total + file.bytes, 0), 0);

  const download = (fetched: DataPackSummary) => {
    const selection: PublishedPackSelection[] = chosen.map(({ table, year }) => ({ table, year }));
    void run(DOWNLOADING, async () => {
      try {
        return await invokeCommand('fetch_published_pack_files', {
          pack: fetched.pack,
          packVersion: fetched.packVersion,
          manifestSha256: fetched.manifestSha256,
          selection,
        });
      } finally {
        // Snapshots may have been written even if this result is no longer shown.
        onImported?.();
      }
    }, setOutcomes);
  };

  return (
    <div className="mt-4 border-t border-line pt-3">
      <h3 className="text-label font-semibold text-fg">Published packs</h3>
      <p className="mb-2 text-label leading-relaxed text-muted">Download a pack OnRecord publishes on Hugging Face. Only the signed manifest is fetched until you choose tables; each chosen file is checked against the manifest&apos;s size and SHA-256 before it is kept or imported. Some tables are hundreds of megabytes.</p>
      {listError && <p className="text-label text-danger" role="alert">{listError}</p>}
      <select aria-label="Published pack" className="analysis-input" value={chosenPack} disabled={!packs || busy === DOWNLOADING} onChange={(event) => choosePack(event.target.value)}>
        <option value="">Choose a pack</option>
        {packs?.map((pack) => <option key={pack.pack} value={pack.pack}>{pack.pack}</option>)}
      </select>
      <Status busy={busy} error={error} />
      {summary && (
        <div className="mt-3 space-y-3 text-label leading-relaxed">
          <div>
            <p className="font-semibold text-fg">{summary.pack} {summary.packVersion}</p>
            <p className="text-fg-secondary">Manifest verified: signed by publisher key {summary.signerKeyId.slice(0, 16)}. Built {summary.builtAt}; schema {summary.schemaVersion}. Each file you choose is checked against this manifest when you download it.</p>
            <p className="text-muted">Each file was retrieved by the pack builder on the date it lists, not by you. Anything published after that is not in it.</p>
          </div>
          <Licence summary={summary} />
          {summary.tables.map((table) => (
            <fieldset key={table.table} className="space-y-1">
              <legend className="font-semibold text-fg">{table.table}</legend>
              {choices.filter((choice) => choice.table === table.table).map((choice) => (
                <label key={choice.key} className="flex items-center gap-2 text-fg-secondary">
                  <input type="checkbox" disabled={!importable(choice)} checked={selected.has(choice.key)} onChange={() => toggle(choice.key)} />
                  {choiceLabel(choice)}{!importable(choice) && ' · this app does not import this file'}
                </label>
              ))}
            </fieldset>
          ))}
          <button type="button" className="analysis-button w-fit" disabled={Boolean(busy) || chosen.length === 0} onClick={() => download(summary)}>
            Download and import {megabytes(chosenBytes)}
          </button>
        </div>
      )}
      {outcomes && summary && <Outcomes outcomes={outcomes} label="Published pack import results" />}
    </div>
  );
}

export default function DataPackImport({ onImported }: { onImported?: () => void }) {
  const { busy, error, run, reset } = useGatedRun();
  const [folder, setFolder] = useState('');
  const [summary, setSummary] = useState<DataPackSummary | null>(null);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [outcomes, setOutcomes] = useState<DataPackImportOutcome[] | null>(null);

  const pickFolder = async () => {
    const chosen = await chooseFolder({ directory: true, multiple: false });
    if (typeof chosen !== 'string') return;
    reset();
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
      <Status busy={busy} error={error} />
      {summary && (
        <div className="mt-3 space-y-3 text-label leading-relaxed">
          <div>
            <p className="font-semibold text-fg">{summary.pack} {summary.packVersion}</p>
            <p className="text-fg-secondary">Verified: signed by publisher key {summary.signerKeyId.slice(0, 16)}, and every file matches its SHA-256. Built {summary.builtAt}; schema {summary.schemaVersion}.</p>
            <p className="text-muted">Each file was retrieved by the pack builder on the date it lists, not by you. Anything published after that is not in it.</p>
          </div>
          <Licence summary={summary} />
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
      {outcomes && summary && <Outcomes outcomes={outcomes} label="Data pack import results" />}
      <PublishedPacks onImported={onImported} />
    </section>
  );
}
