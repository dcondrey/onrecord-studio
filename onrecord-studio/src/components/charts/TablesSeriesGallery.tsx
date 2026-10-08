import type { SqlQueryResult } from '../../types';
import { discoverySeriesModel, formatPeriod, largestDiscoveryChange, type DiscoverySelection } from '../../utils/dataDiscovery';
import Badge from '../ui/Badge';
import BarTable, { type BarTableColumn, type BarTableRow } from './figures/BarTable';
import SeriesSmallMultiples from './figures/SeriesSmallMultiples';
import { formatValue } from './core/format';
import { absent, measured } from './core/nulls';

// Harness-only fixtures for the bar-table and small-multiples figures: every absence and claim
// state the live views can reach, at the column counts that stress the fit ladder at 1024px.

const pct = (value: number) => formatValue(value, { kind: 'percent', digits: 0 });

const DISPOSITIONS = [
  { key: 'RELEASED', label: 'Released', short: 'Released' },
  { key: 'RELEASED_IN_PART', label: 'Released in part', short: 'In part' },
  { key: 'WITHHELD', label: 'Withheld', short: 'Withheld' },
  { key: 'NO_RECORDS', label: 'No responsive records', short: 'No records' },
  { key: 'WITHDRAWN', label: 'Withdrawn', short: 'Withdrawn' },
  { key: 'NOT_CLASSIFIED', label: 'Notice not classified', short: 'Not classified', absence: true },
  { key: 'NOT_RECORDED', label: 'Disposition not recorded', short: 'Not recorded', absence: true },
];

const DEPARTMENTS: { name: string; counts: number[] }[] = [
  { name: 'Environmental Services Department', counts: [21, 14, 7, 9, 2, 6, 1] },
  { name: 'San Diego Police Department', counts: [22, 14, 6, 9, 2, 7, 0] },
  { name: 'Homelessness Strategies and Solutions Department', counts: [20, 14, 8, 9, 2, 5, 0] },
  { name: 'Office of the City Attorney', counts: [1, 0, 1, 0, 0, 1, 0] },
];

function DispositionsExample() {
  const columns: BarTableColumn[] = DISPOSITIONS.map((d) => ({
    key: d.key, label: d.label, short: d.short, scale: 'own-0-100', sortable: true,
    ...(d.absence ? { note: 'not a disposition', hatched: true, barWidth: 28 } : { barWidth: 36 }),
  }));
  const rows: BarTableRow[] = DEPARTMENTS.map((department) => {
    const total = department.counts.reduce((sum, n) => sum + n, 0);
    const small = total < 5;
    return {
      key: department.name,
      name: department.name,
      dimmed: small,
      header: (
        <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
          <span className="font-medium text-fg">{department.name}</span>
          <span className="viz-num font-mono text-code text-muted">n={total}</span>
          {small && <Badge tone="neutral" dashed>Small sample</Badge>}
        </span>
      ),
      cells: Object.fromEntries(DISPOSITIONS.map((d, i) => [d.key, { value: measured(department.counts[i] / total), text: pct(department.counts[i] / total) }])),
    };
  });
  return (
    <BarTable
      title="Dispositions by department"
      headline="Environmental Services withheld 12% of its requests in full."
      claim="descriptive"
      sources={{ tables: ['requests', 'agencies', 'documents'], note: 'Descriptive count, not a finding' }}
      summary="Four departments; the Office of the City Attorney has 3 requests and is a small sample."
      status="ready"
      rowHeader="Department"
      layout="stacked"
      columns={columns}
      rows={rows}
      sort={{ key: 'WITHHELD', direction: 'descending' }}
      onSort={() => {}}
      hatchLabel="No disposition (not classified or not recorded)"
      legend={[{ key: 'share', label: 'Share of the department’s requests', swatch: 'rect', tone: 'mark' }]}
      twin={{ caption: 'Dispositions', columns: [{ key: 'name', label: 'Department' }, ...DISPOSITIONS.map((d) => ({ key: d.key, label: d.label, align: 'end' as const }))], rows: DEPARTMENTS.map((department) => ({ name: department.name, ...Object.fromEntries(DISPOSITIONS.map((d, i) => [d.key, department.counts[i]])) })) }}
    />
  );
}

const FACTORS = [
  { key: 'money at stake', label: 'Money', short: 'Money' },
  { key: 'scale', label: 'Scale', short: 'Scale' },
  { key: 'recurrence', label: 'Recurrence', short: 'Recur.' },
  { key: 'record contradicts statement', label: 'Contradiction', short: 'Contra.' },
  { key: 'counted statutory or policy violation', label: 'Counted violation (legal claim)', short: 'Counted', counted: true },
  { key: 'evidence strength', label: 'Evidence strength', short: 'Evid.' },
  { key: 'freshness', label: 'Freshness', short: 'Fresh' },
];

const LEADS: { name: string; factors: Partial<Record<string, number>>; unassessed?: boolean }[] = [
  { name: 'Storage contract H-24-1180 paid $1,486,200 against a $1,250,000 ceiling', factors: { 'money at stake': 2500, scale: 150, 'counted statutory or policy violation': 1800, 'evidence strength': 1880, freshness: 400 } },
  { name: '"No responsive records" contradicted by a later release from another department', factors: { scale: 150, 'record contradicts statement': 2000, 'evidence strength': 1720, freshness: 400 } },
  { name: 'City and County denials share an identical 64-word exemption paragraph', factors: { scale: 1200, recurrence: 400, 'evidence strength': 1280, freshness: 400 } },
  { name: 'No notices produced for clearances on February 12 and 13', factors: { scale: 300, 'evidence strength': 0, freshness: 400 }, unassessed: true },
];

function LeadsExample() {
  const points = (bp: number) => Math.round(bp / 100);
  const columns: BarTableColumn[] = [
    { key: 'score', label: 'Score', scale: 'own-max', max: 6730, barWidth: 44 },
    ...FACTORS.map((factor): BarTableColumn => ({
      key: factor.key, label: factor.label, short: factor.short, title: factor.key, scale: 'shared', barWidth: 32,
      note: factor.counted ? undefined : 'pattern signal', claim: factor.counted ? 'counted' : undefined,
      missing: { text: '·', title: 'not in this finding’s record' },
    })),
  ];
  const rows: BarTableRow[] = LEADS.map((lead, index) => {
    const total = Object.values(lead.factors).reduce<number>((sum, bp) => sum + (bp ?? 0), 0);
    return {
      key: lead.name,
      name: lead.name,
      header: <span className="grid min-w-[14rem] max-w-[22rem] gap-1"><span className="text-caption tabular-nums text-muted">#{index + 1}</span><span className="line-clamp-2 font-medium text-fg">{lead.name}</span></span>,
      cells: {
        score: { value: measured(total), text: String(points(total)) },
        ...Object.fromEntries(Object.entries(lead.factors).map(([key, bp]) => [key, key === 'evidence strength' && lead.unassessed
          ? { value: absent('not-recorded', 'The finding’s confidence was not assessed'), text: 'not assessed' }
          : { value: measured(bp ?? 0), text: `+${points(bp ?? 0)}` }])),
      },
    };
  });
  return (
    <BarTable
      title="Ranked leads"
      headline="#1 scores 67: most from money at stake (+25) and evidence strength (+19)."
      claim="descriptive"
      sources={{ tables: ['findings'], note: 'Attention score ranks reading order; it is not newsworthiness and not a claim about any record' }}
      summary="Four leads; factor columns share one 0 to 25 point scale."
      status="ready"
      layout="stacked"
      rowHeader="Lead"
      columns={columns}
      rows={rows}
      sharedMax={2500}
      twin={{ caption: 'Ranked leads', columns: [{ key: 'name', label: 'Lead' }, ...FACTORS.map((factor) => ({ key: factor.key, label: factor.key, align: 'end' as const }))], rows: LEADS.map((lead) => ({ name: lead.name, ...Object.fromEntries(FACTORS.map((factor) => [factor.key, lead.factors[factor.key] ?? 'not in this finding’s record'])) })) }}
    />
  );
}

const GROUPS: { name: string | null; sum: number | null; share: number; missing: number }[] = [
  { name: 'Fund 100000', sum: 61_200_000, share: 41.2, missing: 0 },
  { name: 'Fund 200450', sum: 18_400_000, share: 22.9, missing: 12 },
  { name: 'Fund 600010', sum: -2_310_000, share: 9.4, missing: 3 },
  { name: 'Fund 700000', sum: null, share: 3.1, missing: 44 },
  { name: null, sum: 410_000, share: 1.6, missing: 0 },
];

function GroupsExample() {
  const money = (value: number) => formatValue(value, { kind: 'currency', compact: true, signed: value < 0 });
  return (
    <BarTable
      title="Observed value by fund"
      headline="Fund 100000 accounts for 41.2% of records."
      sources={{ tables: ['analysis_budget_actuals'] }}
      summary="Five groups; a negative sum draws left of zero; missing values are hatched outside the measure."
      status="ready"
      rowHeader="fund"
      layout="stacked"
      hatchLabel="Values missing (outside the measure)"
      columns={[
        { key: 'value', label: 'Sum', scale: 'own-max', barWidth: 88 },
        { key: 'share', label: 'Share of records', scale: 'own-0-100', barWidth: 48 },
        { key: 'missing', label: 'Missing', note: 'outside the measure', scale: 'own-max', hatched: true, barWidth: 36 },
      ]}
      rows={GROUPS.map((group) => ({
        key: group.name ?? 'blank',
        name: group.name ?? '(blank in source)',
        header: <span className={group.name === null ? 'text-muted' : 'text-fg'}>{group.name ?? '(blank in source)'}</span>,
        cells: {
          value: group.sum === null ? { value: absent('not-recorded', 'No reported values in this group'), text: 'not measured' } : { value: measured(group.sum), text: money(group.sum) },
          share: { value: measured(group.share / 100), text: `${group.share}%` },
          missing: { value: measured(group.missing), text: group.missing ? `${group.missing} missing` : '0' },
        },
      }))}
      twin={{ caption: 'Groups', columns: [{ key: 'name', label: 'Group' }, { key: 'sum', label: 'Sum', align: 'end' }], rows: GROUPS.map((group) => ({ name: group.name ?? '(blank in source)', sum: group.sum ?? absent('not-recorded', 'no reported values') })) }}
    />
  );
}

export function BarTableExamples() {
  return (
    <div className="grid gap-8">
      <DispositionsExample />
      <LeadsExample />
      <GroupsExample />
    </div>
  );
}

const CHANGE_COLUMNS = ['group_value', 'discovery_period', 'source_rows', 'missing_values', 'observed_value', 'previous_period', 'comparison_status', 'change', 'change_percent'];

/** A 'changes' result as the check returns it: LAG over observed periods within each group. */
function changesResult(series: Record<string, [period: string, value: number | null, missing: number][]>, step: (a: string, b: string) => number): SqlQueryResult {
  const rows: unknown[][] = [];
  for (const [group, points] of Object.entries(series)) {
    points.forEach(([period, value, missing], i) => {
      const previous = points[i - 1];
      const status = !previous ? 'No baseline' : step(previous[0], period) !== 1 ? 'Missing intervening periods'
        : missing > 0 || previous[2] > 0 || value === null || previous[1] === null ? 'Incomplete measurements' : 'Adjacent periods with reported values';
      const change = status === 'Adjacent periods with reported values' && previous && value !== null && previous[1] !== null ? value - previous[1] : null;
      rows.push([group, period, 120, missing, value, previous?.[0] ?? null, status, change, change !== null && previous?.[1] ? Math.round((10000 * change) / Math.abs(previous[1])) / 100 : null]);
    });
  }
  rows.sort((a, b) => Math.abs(Number(b[7] ?? 0)) - Math.abs(Number(a[7] ?? 0)));
  return { columns: CHANGE_COLUMNS, rows, rowCount: rows.length, truncated: false, elapsedMs: 4, provenance: [] };
}

const years = (a: string, b: string) => Number(b.slice(0, 4)) - Number(a.slice(0, 4));
const months = (a: string, b: string) => (Number(b.slice(0, 4)) - Number(a.slice(0, 4))) * 12 + Number(b.slice(5, 7)) - Number(a.slice(5, 7));

function SeriesExample({ title, result, mode, syncGroup }: { title: string; result: SqlQueryResult; mode: DiscoverySelection['periodMode']; syncGroup?: string }) {
  const model = discoverySeriesModel(result, null, mode);
  const largest = largestDiscoveryChange(result);
  const panelOf = largest ? model.groups.find((group) => group.value === largest.group) : undefined;
  const bracket = largest && panelOf ? {
    panelKey: panelOf.key,
    lineKey: 'value',
    from: model.slots.findIndex((slot) => slot.key === String(largest.from)),
    to: model.slots.findIndex((slot) => slot.key === String(largest.to)),
    label: `${formatValue(largest.change, { kind: 'count', compact: true, signed: true })} ${formatPeriod(largest.from, mode)}→${formatPeriod(largest.to, mode)}`,
  } : null;
  return (
    <SeriesSmallMultiples
      title={title}
      headline={largest ? `Largest change: ${String(largest.group)} ${largest.change >= 0 ? 'rose' : 'fell'} ${formatValue(Math.abs(largest.change), { kind: 'count', compact: true })} from ${formatPeriod(largest.from, mode)} to ${formatPeriod(largest.to, mode)}.` : null}
      sources={{ tables: ['analysis_service_requests'] }}
      summary={`${model.groups.length} groups over ${model.slots.filter((slot) => !slot.gap).length} periods.`}
      status="ready"
      slots={model.slots}
      panels={model.groups.map((group) => ({
        key: group.key,
        title: group.label,
        lines: [{
          key: 'value', label: 'Observed value', role: 'primary',
          values: group.cells.map((cell, i) => (cell && cell.observed !== null ? measured(cell.observed) : absent('not-recorded', model.slots[i].gap ? 'No rows for any group in this period' : 'No rows for this group in this period'))),
          incomplete: new Set(group.cells.flatMap((cell, i) => (cell && (cell.missingValues ?? 0) > 0 ? [i] : []))),
        }],
        summary: group.label,
      }))}
      kind="count"
      yScale="shared"
      bracket={bracket && bracket.from >= 0 && bracket.to >= 0 ? bracket : null}
      describe={(panelKey, slot) => {
        const cell = model.groups.find((group) => group.key === panelKey)?.cells[slot];
        return cell ? { rows: [{ key: 'v', value: cell.observed === null ? 'not measured' : String(cell.observed), label: 'observed' }], note: cell.status ?? undefined } : { rows: [], note: model.slots[slot].title ?? 'No rows for this group in this period' };
      }}
      table={{ caption: title, columns: [{ key: 'group', label: 'Group' }, { key: 'period', label: 'Period' }], rows: model.groups.flatMap((group) => model.slots.map((slot) => ({ group: group.label, period: slot.title ?? slot.label }))) }}
      syncGroup={syncGroup}
    />
  );
}

export function SeriesExamples() {
  // Every group skips 2019–2022: one gap four years wide. District 9 alone skips 2017. District 7's
  // 2024 has missing values (a hollow marker).
  const yearly = changesResult({
    'Council District 3': [['2015-01-01', 610, 0], ['2016-01-01', 702, 0], ['2017-01-01', 655, 0], ['2018-01-01', 690, 0], ['2023-01-01', 819, 0], ['2024-01-01', 1212, 0]],
    'Council District 7': [['2015-01-01', 380, 0], ['2016-01-01', 402, 0], ['2017-01-01', 377, 0], ['2018-01-01', 415, 0], ['2023-01-01', 520, 0], ['2024-01-01', 498, 6]],
    'Council District 9': [['2015-01-01', 210, 0], ['2016-01-01', 244, 0], ['2018-01-01', 260, 0], ['2023-01-01', 301, 0], ['2024-01-01', 288, 0]],
  }, years);
  const monthly = changesResult({
    'Sleeper calls': [['2024-01-01', 290, 0], ['2024-02-01', 301, 0], ['2024-03-01', 322, 0], ['2024-07-01', 340, 0], ['2024-08-01', 362, 4], ['2024-09-01', 351, 0], ['2024-10-01', 410, 0], ['2024-11-01', 398, 0], ['2024-12-01', 405, 0]],
    'Welfare checks': [['2024-01-01', 120, 0], ['2024-02-01', 131, 0], ['2024-03-01', 118, 0], ['2024-07-01', 140, 0], ['2024-08-01', 152, 0], ['2024-09-01', 149, 0], ['2024-10-01', 160, 0], ['2024-11-01', 171, 0], ['2024-12-01', 166, 0]],
  }, months);
  return (
    <div className="grid gap-8">
      <SeriesExample title="Changes over time (year)" result={yearly} mode="year" />
      <SeriesExample title="Changes over time (month)" result={monthly} mode="month" />
    </div>
  );
}
