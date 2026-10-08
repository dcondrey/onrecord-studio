import { useEffect, useState } from 'react';
import { open as chooseFolder } from '@tauri-apps/plugin-dialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DataPackFileSummary, DataPackImportOutcome, DataPackSummary, DataPackTable, PublishedPack, PublishedPackSelection } from '../../types';
import Icon from '../ui/icons';
import { formatValue } from '../charts/core/format';
import { absent } from '../charts/core/nulls';
import ChangeTimeline from '../charts/figures/ChangeTimeline';
import StateGrid, { type StateGridCell, type StateGridRow } from '../charts/figures/StateGrid';
import { seqStep } from '../charts/marks/HeatCell';

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
      <p className="text-fg-secondary">License: {summary.licenceId} ({summary.licenceUrl})</p>
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

/** One table and year of a published pack: what a selection names, with every file it downloads. */
interface Choice {
  key: string;
  table: string;
  year: number | null;
  files: DataPackFileSummary[];
  /** In a snapshot pack, the earlier snapshots the choice does not download; `null` in any other pack. */
  earlier: number | null;
}

const retrieved = (file: DataPackFileSummary) => {
  const time = Date.parse(file.retrievedAt);
  return Number.isNaN(time) ? -Infinity : time;
};

/** The latest retrieval, the last listed on a tie, as the native side plans it. */
const latestOf = (files: DataPackFileSummary[]) =>
  files.reduce((latest, file) => (retrieved(file) >= retrieved(latest) ? file : latest));

function choicesOf(summary: DataPackSummary): Choice[] {
  const choices: Choice[] = [];
  for (const table of summary.tables) {
    for (const file of table.files) {
      const key = `${table.table}\u0000${file.year ?? ''}`;
      const existing = choices.find((choice) => choice.key === key);
      if (existing) existing.files.push(file);
      else choices.push({ key, table: table.table, year: file.year, files: [file], earlier: null });
    }
  }
  if (!summary.snapshots) return choices;
  return choices.map((choice) => ({ ...choice, files: [latestOf(choice.files)], earlier: choice.files.length - 1 }));
}

function choiceLabel(choice: Choice): string {
  const rows = choice.files.reduce((sum, file) => sum + file.rows, 0);
  const bytes = choice.files.reduce((sum, file) => sum + file.bytes, 0);
  const count = choice.files.length > 1 ? ` · ${choice.files.length} files` : '';
  const partial = choice.files.some((file) => file.partial) ? PARTIAL_NOTE : '';
  if (choice.earlier !== null) {
    const earlier = choice.earlier > 0 ? ` · ${choice.earlier.toLocaleString()} earlier ${choice.earlier === 1 ? 'snapshot' : 'snapshots'} not downloaded` : '';
    return `Latest snapshot, retrieved ${choice.files[0].retrievedAt.slice(0, 10)} · ${rows.toLocaleString()} rows · ${megabytes(bytes)}${earlier}${partial}`;
  }
  return `${period(choice.year)} · ${rows.toLocaleString()} rows · ${megabytes(bytes)}${count}${partial}`;
}

const importable = (choice: Choice) => choice.files.every((file) => file.datasetKind !== null);

/** A snapshot pack's record of a table between snapshots, or `null` when it records none. */
function snapshotHistory(table: DataPackTable): string | null {
  const parts: string[] = [];
  if (table.unchangedChecks > 0) {
    const times = table.unchangedChecks === 1 ? 'once' : `${table.unchangedChecks.toLocaleString()} times`;
    parts.push(`checked unchanged ${times}, last ${table.lastCheckedAt?.slice(0, 10) ?? 'at an unrecorded time'}`);
  }
  if (table.changeRecords > 0) {
    parts.push(`${table.changeRecords.toLocaleString()} change ${table.changeRecords === 1 ? 'record' : 'records'} between snapshots (listed, not imported)`);
  }
  return parts.length > 0 ? `Snapshots: ${parts.join(' · ')}.` : null;
}

/** The change timeline when the manifest reports its change records; the summary line otherwise. */
function SnapshotHistory({ table, pack }: { table: DataPackTable; pack: string }) {
  // A manifest summary from before change records were reported carries no `changes` at all.
  const changes: DataPackTable['changes'] | undefined = table.changes;
  if (Array.isArray(changes) && (changes.length > 0 || table.unchangedChecks > 0)) {
    return <ChangeTimeline table={table.table} changes={changes} unchangedChecks={table.unchangedChecks} lastCheckedAt={table.lastCheckedAt} storageKey={`pack-changes:${pack}:${table.table}`} />;
  }
  const history = snapshotHistory(table);
  return history ? <p className="text-muted">{history}</p> : null;
}

const ALL_YEARS = 'all';

/** One cell of the coverage matrix: what a table holds for one year, as the import picker offers it. */
interface CoverageChoice {
  key: string;
  table: string;
  year: number | null;
  files: DataPackFileSummary[];
}

function fileCell(choice: CoverageChoice, maxRows: number, selected: boolean): StateGridCell {
  const rows = choice.files.reduce((sum, file) => sum + file.rows, 0);
  const bytes = choice.files.reduce((sum, file) => sum + file.bytes, 0);
  const latest = latestOf(choice.files);
  const partial = choice.files.some((file) => file.partial);
  const where = `${choice.table}, ${period(choice.year)}`;
  const tip = {
    rows: [
      { key: 'rows', value: rows.toLocaleString(), label: 'rows' },
      { key: 'size', value: megabytes(bytes), label: 'size' },
      { key: 'retrieved', value: latest.retrievedAt.slice(0, 10), label: 'retrieved by the pack builder' },
      ...(latest.licence ? [{ key: 'licence', value: latest.licence.id, label: 'licence' }] : []),
    ],
    note: partial ? 'Period not over: later rows may be published.' : undefined,
  };
  if (choice.files.some((file) => file.datasetKind === null)) {
    return { kind: 'state', tone: 'neutral', glyph: 'bar', text: 'not imported', name: `${where}: in the pack, but this app does not import this file.`, tip };
  }
  const text = formatValue(rows, { kind: 'count', compact: true });
  return {
    kind: 'count', value: rows, step: seqStep(rows, maxRows), glyph: partial ? 'triangle' : 'circle', actionable: true, tip,
    text: selected ? `${text} ✓` : text,
    tableText: `${rows.toLocaleString()} rows · ${megabytes(bytes)} · retrieved ${latest.retrievedAt.slice(0, 10)}${partial ? ' · partial' : ''}`,
    name: `${where}: ${rows.toLocaleString()} rows, retrieved ${latest.retrievedAt.slice(0, 10)}${partial ? ', period not over' : ''}. ${selected ? 'Selected for import.' : 'Not selected.'} Press Enter to ${selected ? 'deselect' : 'select'} it for import.`,
  };
}

/** A tables × years matrix of what the pack holds. A year with no file is an empty cell, never 0 rows. */
export function coverageMatrix(summary: DataPackSummary, choices: readonly CoverageChoice[], selected: ReadonlySet<string>) {
  const listed = choices.flatMap((choice) => (choice.year === null ? [] : [choice.year]));
  // Every year between the first and last listed: a year inside the span with no file is a gap.
  const years = listed.length ? Array.from({ length: Math.max(...listed) - Math.min(...listed) + 1 }, (_, i) => Math.min(...listed) + i) : [];
  const oneFile = choices.some((choice) => choice.year === null);
  const columns = [...years.map(String), ...(oneFile ? [ALL_YEARS] : [])];
  const maxRows = Math.max(0, ...choices.map((choice) => choice.files.reduce((sum, file) => sum + file.rows, 0)));
  const positions: { row: number; col: number }[] = [];
  const keys: (string | null)[][] = [];
  let present = 0;
  let missing = 0;
  const rows: StateGridRow[] = summary.tables.map((table, i) => {
    const own = choices.filter((choice) => choice.table === table.table);
    const splitByYear = own.some((choice) => choice.year !== null);
    const latest = own.flatMap((choice) => choice.files).reduce<DataPackFileSummary | null>((best, file) => (!best || retrieved(file) >= retrieved(best) ? file : best), null);
    const freshness = [
      latest ? new Date(latest.retrievedAt).toLocaleDateString('en-US', { month: 'short', day: 'numeric', year: 'numeric', timeZone: 'UTC' }) : null,
      table.unchangedChecks > 0 ? `${table.unchangedChecks} unchanged` : null,
      table.changeRecords > 0 ? `${table.changeRecords} ${table.changeRecords === 1 ? 'change' : 'changes'}` : null,
    ].filter(Boolean).join(' · ');
    keys.push([]);
    return {
      key: table.table,
      label: table.table,
      trailing: { text: freshness || 'no retrieval date' },
      cells: columns.map((column, j): StateGridCell => {
        const year = column === ALL_YEARS ? null : Number(column);
        const choice = own.find((candidate) => candidate.year === year);
        keys[i].push(choice?.key ?? null);
        if (choice) {
          present += 1;
          const isSelected = selected.has(choice.key);
          if (isSelected) positions.push({ row: i, col: j });
          return fileCell(choice, maxRows, isSelected);
        }
        if (year !== null && !splitByYear) {
          return { kind: 'gap', absence: absent('not-covered', 'published as one file covering every year'), glyph: 'bar', text: 'one file', name: `${table.table}, ${year}: published as one file, not by year.` };
        }
        if (year === null) {
          return { kind: 'gap', absence: absent('not-covered', 'published by year, not as one file'), glyph: 'bar', text: 'by year', name: `${table.table}: published by year, not as one file.` };
        }
        missing += 1;
        return { kind: 'gap', absence: absent('not-covered', `no ${year} file in this pack`), glyph: 'ring', text: 'no file', name: `${table.table}, ${year}: no file in this pack. Not zero rows.` };
      }),
    };
  });
  const partial = choices.filter((choice) => choice.files.some((file) => file.partial)).length;
  const headline = `${present} of ${present + missing} table-years have a file in this pack${missing > 0 ? `; ${missing} have none` : ''}${partial > 0 ? `; ${partial} cover a period not yet over` : ''}.`;
  return { columns, rows, positions, keys, headline };
}

function PackCoverage({ summary, choices, selected, onToggle }: { summary: DataPackSummary; choices: readonly CoverageChoice[]; selected: ReadonlySet<string>; onToggle(key: string): void }) {
  const matrix = coverageMatrix(summary, choices, selected);
  return (
    <StateGrid
      title="What this pack holds"
      storageKey={`pack-coverage:${summary.pack}`}
      headline={matrix.headline}
      sources={{ tables: ['pack manifest'], note: `${summary.pack} ${summary.packVersion}, built ${summary.builtAt.slice(0, 10)}` }}
      summary={`${summary.tables.length} tables by ${matrix.columns.length} years. Activate a file's cell to select it for import.`}
      status="ready"
      rowHeader="Table"
      trailingHeader="Freshness"
      columns={matrix.columns.map((column) => ({ key: column, label: column === ALL_YEARS ? 'All years' : column }))}
      rows={matrix.rows}
      selected={matrix.positions}
      onActivate={(row, col) => {
        const key = matrix.keys[row]?.[col];
        if (key) onToggle(key);
      }}
      legend={[
        { key: 'file', label: 'File in the pack (darker = more rows; ▲ period not over)', swatch: 'rect', tone: 'mark' },
      ]}
    />
  );
}

/** A folder pack's files as matrix cells: the latest file of each table and year. */
function folderChoices(summary: DataPackSummary): CoverageChoice[] {
  const choices: CoverageChoice[] = [];
  for (const table of summary.tables) {
    const years = [...new Set(table.files.map((file) => file.year))];
    for (const year of years) {
      const latest = latestOf(table.files.filter((file) => file.year === year));
      choices.push({ key: latest.path, table: table.table, year, files: [latest] });
    }
  }
  return choices;
}

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
          {summary.tables.length > 0 && <PackCoverage summary={summary} choices={choices} selected={selected} onToggle={toggle} />}
          {summary.tables.map((table) => (
            <fieldset key={table.table} className="space-y-1">
              <legend className="font-semibold text-fg">{table.table}</legend>
              <SnapshotHistory table={table} pack={summary.pack} />
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
          {summary.tables.length > 0 && <PackCoverage summary={summary} choices={folderChoices(summary)} selected={selected} onToggle={toggle} />}
          {summary.tables.map((table) => (
            <fieldset key={table.table} className="space-y-1">
              <legend className="font-semibold text-fg">{table.table}</legend>
              <SnapshotHistory table={table} pack={summary.pack} />
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
