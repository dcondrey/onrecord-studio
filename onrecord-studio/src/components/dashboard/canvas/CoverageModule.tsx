import { useMemo } from 'react';
import { useAppStore } from '../../../store/useAppStore';
import type { AnalyticalDatasetSummary } from '../../../types';
import { formatValue } from '../../charts/core/format';
import { measured } from '../../charts/core/nulls';
import BarTable, { type BarTableRow } from '../../charts/figures/BarTable';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import Skeleton from '../../ui/Skeleton';
import { useAutomaticDiscovery } from '../AutomaticDiscoveryProvider';
import CanvasModule from './CanvasModule';

interface PackGroup {
  key: string;
  label: string;
  /** The pack id as published, or null for tables imported by hand. */
  pack: string | null;
  tables: AnalyticalDatasetSummary[];
  rows: number;
  partial: number;
}

const OWN_IMPORTS = 'own';
const STEPS = ['var(--viz-ord-3)', 'var(--viz-ord-2)'] as const;

/** The published pack a table was installed from, as the import recorded it; null when none was. */
export function packOf(dataset: AnalyticalDatasetSummary): string | null {
  try {
    const pack = (JSON.parse(dataset.validationSummary) as { packOrigin?: { pack?: unknown } } | null)?.packOrigin?.pack;
    return typeof pack === 'string' && pack.trim() ? pack.trim() : null;
  } catch {
    return null;
  }
}

/** 'san-diego-enforcement' reads as 'Enforcement': the topic word after the shared place prefix. */
function packLabel(pack: string, all: readonly string[]): string {
  const parts = all.map((name) => name.split('-'));
  let shared = 0;
  while (all.length > 1 && parts.every((segments) => segments.length > shared + 1 && segments[shared] === parts[0][shared])) shared += 1;
  const words = pack.split('-').slice(shared).join(' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function packGroups(datasets: readonly AnalyticalDatasetSummary[]): PackGroup[] {
  const byPack = new Map<string, AnalyticalDatasetSummary[]>();
  for (const dataset of datasets) {
    const key = packOf(dataset) ?? OWN_IMPORTS;
    byPack.set(key, [...(byPack.get(key) ?? []), dataset]);
  }
  const packs = [...byPack.keys()].filter((key) => key !== OWN_IMPORTS);
  return [...byPack.entries()].map(([key, tables]) => ({
    key,
    pack: key === OWN_IMPORTS ? null : key,
    label: key === OWN_IMPORTS ? 'Imported by you' : packLabel(key, packs),
    tables: [...tables].sort((a, b) => b.importedRows - a.importedRows || a.displayName.localeCompare(b.displayName)),
    rows: tables.reduce((sum, table) => sum + table.importedRows, 0),
    partial: tables.filter((table) => table.partialDataset).length,
  })).sort((a, b) => b.rows - a.rows || a.label.localeCompare(b.label));
}

/** The figure's own sentence: the largest table and its share, which the module's sentence does not state. */
function largestHeadline(datasets: readonly AnalyticalDatasetSummary[]): string {
  const total = datasets.reduce((sum, table) => sum + table.importedRows, 0);
  const largest = [...datasets].sort((a, b) => b.importedRows - a.importedRows)[0];
  if (!largest || total === 0) return 'No imported table holds a row yet.';
  const share = formatValue(largest.importedRows / total, { kind: 'percent' });
  return `${largest.displayName} is the largest table: ${largest.importedRows.toLocaleString('en-US')} rows, ${share} of the library.`;
}

const rowsText =(n: number) => formatValue(n, { kind: 'count', compact: n >= 10_000 });
const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`;

/**
 * What the imported City tables cover, one strip per published pack: each table is a segment as
 * long as its rows, on one row scale shared by every strip, so a pack's length is its share of the
 * library. A table the import marked partial or unverified is hatched. Expands into the per-table
 * figure with its table twin.
 */
export default function CoverageModule({ expanded, onToggle, span }: { expanded: boolean; onToggle(): void; span?: string }) {
  const { state } = useAutomaticDiscovery();
  const setWorkspaceView = useAppStore((app) => app.setWorkspaceView);
  const datasets = useMemo(() => state?.schema?.datasets ?? [], [state?.schema]);
  const groups = useMemo(() => packGroups(datasets), [datasets]);
  if (!state) return null;
  const total = groups.reduce((sum, group) => sum + group.rows, 0);
  const partial = groups.reduce((sum, group) => sum + group.partial, 0);
  const packs = groups.filter((group) => group.pack !== null).length;
  const scaleMax = Math.max(1, ...groups.map((group) => group.rows));
  const common = {
    id: 'canvas-coverage',
    icon: 'database' as const,
    title: 'Library coverage',
    expanded,
    onToggle,
    expandLabel: 'Show every imported table',
    collapseLabel: 'Hide every imported table',
    span,
  };

  if (!state.schema) {
    return (
      <CanvasModule {...common} headline={null}>
        <Skeleton variant="lines" lines={4} label="Reading the imported tables" />
      </CanvasModule>
    );
  }
  if (datasets.length === 0) {
    return <CanvasModule {...common} headline="No City table is imported yet." />;
  }

  const headline = `${plural(datasets.length, 'table')}${packs > 0 ? ` from ${plural(packs, 'published pack')}` : ''} hold ${total.toLocaleString('en-US')} rows${partial > 0 ? `; ${partial.toLocaleString('en-US')} ${partial === 1 ? 'is' : 'are'} partial or unverified` : ''}.`;
  const rows: BarTableRow[] = groups.flatMap((group) => group.tables.map((table) => ({
    key: table.snapshotId,
    name: table.displayName,
    group: { key: group.key, label: group.label, note: group.pack ?? undefined },
    label: `${table.displayName}: ${table.importedRows.toLocaleString('en-US')} rows${table.partialDataset ? ', partial or unverified' : ''}`,
    header: (
      <span className="flex min-w-0 flex-wrap items-center gap-2">
        <span className="min-w-0 text-body text-fg">{table.displayName}</span>
        {table.partialDataset && <Badge dashed>Partial or unverified</Badge>}
      </span>
    ),
    cells: {
      rows: { value: measured(table.importedRows), text: table.importedRows.toLocaleString('en-US') },
      rejected: { value: measured(table.rejectedRows), text: table.rejectedRows.toLocaleString('en-US') },
    },
  })));

  return (
    <CanvasModule
      {...common}
      headline={headline}
      detail={(
        <div className="grid grid-cols-1 gap-4">
          <BarTable
            title="Imported tables by pack"
            headline={largestHeadline(datasets)}
            sources={{ tables: ['analytical_snapshots'], note: 'row counts as imported; a pack is the published bundle the table was installed from' }}
            summary={`${plural(datasets.length, 'table')} in ${plural(groups.length, 'group')}. Rows share one scale across every table.`}
            status="ready"
            rowHeader="Table"
            columns={[
              { key: 'rows', label: 'Rows imported', scale: 'shared', barWidth: 160 },
              { key: 'rejected', label: 'Rows rejected', scale: 'own-max', barWidth: 48 },
            ]}
            rows={rows}
            sharedMax={Math.max(1, ...datasets.map((table) => table.importedRows))}
            dense
            twin={{
              caption: 'Imported tables by pack',
              columns: [
                { key: 'pack', label: 'Pack' },
                { key: 'table', label: 'Table' },
                { key: 'rows', label: 'Rows imported', kind: 'count', align: 'end' },
                { key: 'rejected', label: 'Rows rejected', kind: 'count', align: 'end' },
                { key: 'partial', label: 'Coverage' },
              ],
              rows: groups.flatMap((group) => group.tables.map((table) => ({
                pack: group.pack ?? group.label,
                table: table.displayName,
                rows: table.importedRows,
                rejected: table.rejectedRows,
                partial: table.partialDataset ? 'partial or unverified' : 'complete as published',
              }))),
            }}
          />
          <Button size="sm" className="w-fit" onClick={() => setWorkspaceView('analysis')}>Open the tables in Analysis</Button>
        </div>
      )}
    >
      <ul className="grid gap-3.5">
        {groups.map((group) => (
          <li key={group.key} className="grid gap-1.5">
            <div className="flex min-w-0 items-baseline justify-between gap-3">
              <span className="min-w-0 truncate text-body text-fg" title={group.pack ?? undefined}>
                {group.label}
                <span className="text-caption text-muted"> · {plural(group.tables.length, 'table')}</span>
              </span>
              <span className="shrink-0 text-caption tabular-nums text-fg-secondary">{rowsText(group.rows)} rows</span>
            </div>
            <div
              role="img"
              aria-label={`${group.label}: ${plural(group.tables.length, 'table')}, ${group.rows.toLocaleString('en-US')} rows${group.partial ? `, ${group.partial} partial or unverified` : ''}`}
              className="flex h-2.5 gap-0.5"
              style={{ width: `${Math.max(2, (group.rows / scaleMax) * 100)}%` }}
            >
              {group.tables.map((table, index) => (
                <span
                  key={table.snapshotId}
                  title={`${table.displayName}: ${table.importedRows.toLocaleString('en-US')} rows${table.partialDataset ? ' (partial or unverified)' : ''}`}
                  className="coverage-segment h-full min-w-[3px] first:rounded-l-full last:rounded-r-full"
                  data-partial={table.partialDataset || undefined}
                  style={{ flexGrow: Math.max(table.importedRows, 1), flexBasis: 0, background: STEPS[index % 2] }}
                />
              ))}
            </div>
          </li>
        ))}
      </ul>
      <ul aria-label="Legend" className="mt-4 flex flex-wrap gap-x-4 gap-y-1 text-caption text-muted">
        <li className="flex items-center gap-1.5"><span aria-hidden="true" className="h-2.5 w-3 rounded-sm" style={{ background: STEPS[0] }} />One table, by rows</li>
        {partial > 0 && <li className="flex items-center gap-1.5"><span aria-hidden="true" className="coverage-segment h-2.5 w-3 rounded-sm" data-partial style={{ background: STEPS[1] }} />Partial or unverified</li>}
      </ul>
    </CanvasModule>
  );
}
