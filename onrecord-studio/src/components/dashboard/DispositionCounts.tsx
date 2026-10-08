import { Fragment, useEffect, useState, type ReactNode } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DepartmentDispositions, Disposition } from '../../types';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import Badge from '../ui/Badge';
import InlineError from '../ui/InlineError';
import MethodNote from '../ui/MethodNote';
import Skeleton from '../ui/Skeleton';
import Table, { Td, Th } from '../ui/Table';
import ObjectLink from '../objects/ObjectLink';
import Button from '../ui/Button';
import BarTable, { type BarTableColumn, type BarTableRow } from '../charts/figures/BarTable';
import { formatValue } from '../charts/core/format';
import { measured } from '../charts/core/nulls';

/** Below this many requests one request moves a department's share by a fifth or more. */
const SMALL_DEPARTMENT_REQUESTS = 5;

const DISPOSITION_LABELS: Record<Disposition, string> = {
  RELEASED: 'Released',
  RELEASED_IN_PART: 'Released in part',
  WITHHELD: 'Withheld',
  NO_RECORDS: 'No responsive records',
  WITHDRAWN: 'Withdrawn',
  NOT_CLASSIFIED: 'Notice not classified',
  NOT_RECORDED: 'Disposition not recorded',
};

const DISPOSITION_ORDER = Object.keys(DISPOSITION_LABELS) as Disposition[];

/** Header labels for a frame too narrow for the full ones; the header's title keeps the full label. */
const DISPOSITION_SHORT: Record<Disposition, string> = {
  RELEASED: 'Released',
  RELEASED_IN_PART: 'In part',
  WITHHELD: 'Withheld',
  NO_RECORDS: 'No records',
  WITHDRAWN: 'Withdrawn',
  NOT_CLASSIFIED: 'Not classified',
  NOT_RECORDED: 'Not recorded',
};

type Held =
  | { status: 'loading' }
  | { status: 'failed'; message: string }
  | { status: 'ready'; rows: DepartmentDispositions[] };

function departmentKey(row: DepartmentDispositions): string {
  return `${row.portalSubdomain}\u0000${row.agencyId ?? ''}`;
}

function share(count: number, total: number): string {
  return total === 0 ? '' : `${((count / total) * 100).toFixed(1)}%`;
}

/**
 * How each department's requests were closed, counted from the agency's closure notice. A
 * descriptive count of what the agency answered, never a finding that the answer was wrong.
 * `portalSubdomain` null counts every portal.
 */
export default function DispositionCounts({ portalSubdomain }: { portalSubdomain: string | null }) {
  const gate = useLatestRequestGate();
  const [held, setHeld] = useState<Held>({ status: 'loading' });
  const savedSources = useAppStore((state) => state.portals);

  useEffect(() => {
    const token = gate.begin();
    setHeld({ status: 'loading' });
    invokeCommand('get_department_dispositions', { portalSubdomain })
      .then((rows) => {
        if (!gate.isCurrent(token)) return;
        setHeld({ status: 'ready', rows });
      })
      .catch((error: unknown) => {
        if (!gate.isCurrent(token)) return;
        setHeld({ status: 'failed', message: formatNativeError(error) });
      });
  }, [gate, portalSubdomain]);

  const spansPortals = held.status === 'ready' && new Set(held.rows.map((row) => row.portalSubdomain)).size > 1;
  const portalName = (portalKey: string) =>
    savedSources.find(({ registration }) => registration.portalKey === portalKey)?.registration.displayName ?? portalKey;

  const method = (
          <MethodNote label="How dispositions are counted">
            <p>
              Each request is counted once, from the closure notice stored for it, by the first rule its
              words match: <strong>Withdrawn</strong> (“request withdrawn”, “withdrawn this request”, or a request
              stored as withdrawn); <strong>No responsive records</strong> (“no responsive documents” or “no
              responsive records”); <strong>Withheld</strong> (“not released”, or “any documents responsive to the
              request are being withheld”); <strong>Released in part</strong> (“released except”, “released
              redacted”, “released withheld”); <strong>Released</strong> (“released”).
            </p>
            <p className="mt-1.5">
              <strong>Disposition not recorded</strong> means no closure notice is stored for the request: it is still
              open, its portal’s closing text was not collected (only hosted NextRequest portals are read for it, and
              only by syncs since that was added), or it closed without one. It says nothing about how the agency
              answered. <strong>Notice not classified</strong> means a notice is stored but no
              rule matches its wording, or its text has not been extracted. Neither is counted as any disposition.
              Every share is of the department’s requests in scope, these two included. A department with fewer
              than {SMALL_DEPARTMENT_REQUESTS} requests is marked small, because one request moves its shares by a
              fifth or more. These counts describe what each agency answered; they are not findings.
            </p>
          </MethodNote>
  );

  return (
    <section aria-label="Dispositions by department" className="app-panel overflow-hidden px-5 py-4">
      {held.status === 'failed' ? (
        <div className="grid gap-3">
          <div className="flex flex-wrap items-start justify-between gap-2">
            <h3 className="text-heading text-fg">Dispositions by department</h3>
            {method}
          </div>
          <InlineError summary="Dispositions could not be counted." detail={held.message} />
        </div>
      ) : held.status === 'loading' ? (
        <div className="grid gap-3">
          <h3 className="text-heading text-fg">Dispositions by department</h3>
          <Skeleton variant="lines" lines={4} label="Counting dispositions" />
        </div>
      ) : (
        <DispositionBarTable rows={held.rows} portalName={spansPortals ? portalName : null} method={method} />
      )}
    </section>
  );
}

/** The five dispositions a closure notice can state, in the figure's fixed order. */
const SHARE_COLUMNS: readonly Disposition[] = ['RELEASED', 'RELEASED_IN_PART', 'WITHHELD', 'NO_RECORDS', 'WITHDRAWN'];
/** Requests with no disposition: real requests, never a disposition, so drawn hatched. */
const ABSENCE_COLUMNS: readonly Disposition[] = ['NOT_CLASSIFIED', 'NOT_RECORDED'];
const VISIBLE_DEPARTMENTS = 12;
const DEFAULT_SORT = 'WITHHELD';

const percent = (fraction: number) => formatValue(fraction, { kind: 'percent', digits: 0 });

function countOf(row: DepartmentDispositions, disposition: Disposition): number {
  return row.buckets.find((bucket) => bucket.disposition === disposition)?.requests.length ?? 0;
}

/**
 * The deterministic headline. With the default sort it names the department that withholds or
 * partly releases the largest share; once a column is sorted, the leader in that column. Only
 * departments with at least SMALL_DEPARTMENT_REQUESTS requests can lead.
 */
export function dispositionHeadline(rows: readonly DepartmentDispositions[], sortKey: Disposition | null): string {
  const eligible = rows.filter((row) => row.totalRequests >= SMALL_DEPARTMENT_REQUESTS && row.agencyId !== null);
  if (eligible.length === 0) {
    return `Every department has fewer than ${SMALL_DEPARTMENT_REQUESTS} requests; shares move by 20% or more per request.`;
  }
  const shareOf = (row: DepartmentDispositions) => sortKey === null
    ? (countOf(row, 'WITHHELD') + countOf(row, 'RELEASED_IN_PART')) / row.totalRequests
    : countOf(row, sortKey) / row.totalRequests;
  const leader = eligible.reduce((best, row) => (shareOf(row) > shareOf(best) ? row : best));
  const name = leader.agencyName ?? 'Department not recorded';
  const tail = `${percent(shareOf(leader))} of ${leader.totalRequests.toLocaleString('en-US')} requests.`;
  return sortKey === null
    ? `${name} withholds or partly releases the largest share: ${tail}`
    : `${name} has the highest ${DISPOSITION_LABELS[sortKey].toLowerCase()} share: ${tail}`;
}

function DispositionBarTable({ rows, portalName, method }: {
  rows: DepartmentDispositions[];
  portalName: ((portalKey: string) => string) | null;
  method: ReactNode;
}) {
  const [sortKey, setSortKey] = useState<Disposition | null>(null);
  const [showAll, setShowAll] = useState(false);
  const sortBy = sortKey ?? DEFAULT_SORT;
  const byTotal = [...rows].sort((a, b) => b.totalRequests - a.totalRequests);
  const members = showAll ? byTotal : byTotal.slice(0, VISIBLE_DEPARTMENTS);
  const ordered = [...members].sort((a, b) => {
    // A request with no department is never ranked among departments.
    if ((a.agencyId === null) !== (b.agencyId === null)) return a.agencyId === null ? 1 : -1;
    const delta = countOf(b, sortBy) / Math.max(1, b.totalRequests) - countOf(a, sortBy) / Math.max(1, a.totalRequests);
    return delta !== 0 ? delta : b.totalRequests - a.totalRequests;
  });
  const total = rows.reduce((sum, row) => sum + row.totalRequests, 0);
  const unclassified = rows.reduce((sum, row) => sum + countOf(row, 'NOT_CLASSIFIED') + countOf(row, 'NOT_RECORDED'), 0);

  const columns: BarTableColumn[] = [
    ...SHARE_COLUMNS.map((disposition): BarTableColumn => ({ key: disposition, label: DISPOSITION_LABELS[disposition], short: DISPOSITION_SHORT[disposition], scale: 'own-0-100', sortable: true, barWidth: 36 })),
    ...ABSENCE_COLUMNS.map((disposition): BarTableColumn => ({
      key: disposition,
      label: DISPOSITION_LABELS[disposition],
      short: DISPOSITION_SHORT[disposition],
      note: 'not a disposition',
      title: `${DISPOSITION_LABELS[disposition]}: requests with no disposition. Drawn hatched because it is not an answer.`,
      scale: 'own-0-100',
      hatched: true,
      sortable: true,
      barWidth: 28,
    })),
  ];

  const tableRows: BarTableRow[] = ordered.map((row) => {
    const key = departmentKey(row);
    const name = row.agencyName ?? 'Department not recorded';
    const small = row.totalRequests < SMALL_DEPARTMENT_REQUESTS;
    const cells: BarTableRow['cells'] = {};
    for (const disposition of DISPOSITION_ORDER) {
      const bucket = row.buckets.find((candidate) => candidate.disposition === disposition);
      const count = bucket?.requests.length ?? 0;
      const fraction = row.totalRequests === 0 ? 0 : count / row.totalRequests;
      const ids = (bucket?.requests ?? []).slice(0, 5).map((request) => request.rpid ?? `request ${request.requestId}`);
      cells[disposition] = {
        value: measured(fraction),
        text: percent(fraction),
        active: count > 0,
        tooltip: {
          rows: [
            { key: 'count', value: count.toLocaleString('en-US'), label: `of ${row.totalRequests.toLocaleString('en-US')} requests` },
            { key: 'share', value: share(count, row.totalRequests) || '0%', label: 'share' },
          ],
          note: ids.length > 0 ? `${ids.join(', ')}${count > ids.length ? ` and ${count - ids.length} more` : ''}` : undefined,
        },
      };
    }
    return {
      key,
      name,
      dimmed: small,
      label: `${name}, ${row.totalRequests} requests: ${DISPOSITION_ORDER.map((disposition) => `${DISPOSITION_LABELS[disposition]} ${percent(countOf(row, disposition) / Math.max(1, row.totalRequests))}`).join(', ')}`,
      header: (
        <div className="grid gap-0.5">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
            {row.agencyId === null ? (
              <span className="text-muted">{name}</span>
            ) : (
              <ObjectLink target={{ kind: 'agency', portalKey: row.portalSubdomain, agencyId: row.agencyId, label: name }} className="line-clamp-2 font-medium text-fg no-underline hover:underline">{name}</ObjectLink>
            )}
            <span className="viz-num font-mono text-code text-muted">n={row.totalRequests.toLocaleString('en-US')}</span>
            {small && (
              <Badge tone="neutral" dashed title={`${row.totalRequests} requests; one request moves each share by a fifth or more.`}>
                Small sample
              </Badge>
            )}
          </div>
          {portalName && <span className="text-caption text-muted" title={row.portalSubdomain}>{portalName(row.portalSubdomain)}</span>}
        </div>
      ),
      cells,
    };
  });

  const byKey = new Map(ordered.map((row) => [departmentKey(row), row]));
  const renderBucket = (rowKey: string, columnKey: string | null) => {
    const row = byKey.get(rowKey);
    const bucket = row?.buckets.find((candidate) => candidate.disposition === columnKey);
    if (!row || !bucket) return null;
    return (
      <div>
        <span className="mr-2 text-caption font-medium text-fg-secondary">
          {DISPOSITION_LABELS[bucket.disposition]} · {row.agencyName ?? 'Department not recorded'} · {bucket.requests.length.toLocaleString('en-US')} of {row.totalRequests.toLocaleString('en-US')}
        </span>
        <span className="inline-flex flex-wrap gap-x-3 gap-y-1">
          {bucket.requests.map((request) => (
            <ObjectLink
              key={request.requestId}
              target={{ kind: 'request', portalKey: row.portalSubdomain, requestId: request.requestId, label: request.rpid ?? undefined }}
              className="font-mono text-code text-fg no-underline hover:underline"
            >
              {request.rpid ?? `request ${request.requestId}`}
            </ObjectLink>
          ))}
        </span>
      </div>
    );
  };

  return (
    <BarTable
      title="Dispositions by department"
      headline={dispositionHeadline(rows, sortKey)}
      claim="descriptive"
      sources={{ tables: ['requests', 'agencies', 'documents'], note: "Share of each department's requests by closure-notice wording. Descriptive count, not a finding" }}
      summary={`${rows.length} departments, ${total.toLocaleString('en-US')} requests. ${unclassified.toLocaleString('en-US')} requests have no disposition (notice not classified or not recorded) and are drawn hatched beside the five dispositions.`}
      status={rows.length === 0 ? 'empty' : 'ready'}
      emptyText="No requests are stored for this scope."
      rowHeader="Department"
      columns={columns}
      rows={tableRows}
      sort={{ key: sortBy, direction: 'descending' }}
      onSort={(key) => setSortKey(key === DEFAULT_SORT && sortKey === null ? null : key as Disposition)}
      drill={{ mode: 'cell', render: renderBucket }}
      twin={{ node: <CountsTable rows={ordered} portalName={portalName} /> }}
      twinLabel="Show counts table"
      hatchLabel="No disposition (not classified or not recorded)"
      layout="stacked"
      legend={[{ key: 'share', label: 'Share of the department’s requests', swatch: 'rect', tone: 'mark' }]}
      controls={method}
      footer={rows.length > VISIBLE_DEPARTMENTS && (
        <Button size="sm" variant="ghost" className="justify-self-start" aria-expanded={showAll} onClick={() => setShowAll((current) => !current)}>
          {showAll ? `Show the ${VISIBLE_DEPARTMENTS} largest` : `Show all ${rows.length} departments`}
        </Button>
      )}
    />
  );
}

/** The exact counts, with each bucket's request links: the copyable twin of the bar-table. */
function CountsTable({ rows, portalName }: { rows: DepartmentDispositions[]; portalName: ((portalKey: string) => string) | null }) {
  const [expanded, setExpanded] = useState<string | null>(null);
  return (
    <Table label="Dispositions by department">
      <thead>
        <tr>
          <Th className="min-w-[12rem] py-2 align-bottom">Department</Th>
          <Th numeric className="py-2 align-bottom">Requests</Th>
          {DISPOSITION_ORDER.map((disposition) => (
            <Th key={disposition} numeric className="h-auto min-w-[5.5rem] whitespace-normal py-2 align-bottom leading-4">{DISPOSITION_LABELS[disposition]}</Th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row) => {
          const key = departmentKey(row);
          const small = row.totalRequests < SMALL_DEPARTMENT_REQUESTS;
          const name = row.agencyName ?? 'Department not recorded';
          const open = row.buckets.find((bucket) => `${key}\u0000${bucket.disposition}` === expanded);
          return (
            <Fragment key={key}>
              <tr>
                <Td className="min-w-[12rem]">
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
                    {row.agencyId === null ? (
                      <span className="text-muted">{name}</span>
                    ) : (
                      <ObjectLink target={{ kind: 'agency', portalKey: row.portalSubdomain, agencyId: row.agencyId, label: name }} className="line-clamp-2 font-medium text-fg no-underline hover:underline">{name}</ObjectLink>
                    )}
                    {small && (
                      <Badge tone="neutral" dashed title={`${row.totalRequests} requests; one request moves each share by a fifth or more.`}>
                        Small sample
                      </Badge>
                    )}
                  </div>
                  {portalName && <p className="mt-0.5 text-caption text-muted" title={row.portalSubdomain}>{portalName(row.portalSubdomain)}</p>}
                </Td>
                <Td numeric>{row.totalRequests.toLocaleString()}</Td>
                {DISPOSITION_ORDER.map((disposition) => {
                  const bucket = row.buckets.find((candidate) => candidate.disposition === disposition);
                  const count = bucket?.requests.length ?? 0;
                  const bucketKey = `${key}\u0000${disposition}`;
                  const isOpen = expanded === bucketKey;
                  return (
                    <Td key={disposition} numeric>
                      {count === 0 ? (
                        <span className="text-muted">0</span>
                      ) : (
                        <button
                          type="button"
                          aria-expanded={isOpen}
                          onClick={() => setExpanded((current) => (current === bucketKey ? null : bucketKey))}
                          className="focus-ring inline-flex flex-col items-end rounded text-right"
                        >
                          <span className="sr-only">{`${DISPOSITION_LABELS[disposition]}, ${name}: `}</span>
                          <span className={cn('text-fg underline decoration-fg/40 underline-offset-4 hover:decoration-solid', isOpen ? 'decoration-solid' : 'decoration-dotted')}>{count.toLocaleString()}</span>
                          <span className="sr-only">{` of ${row.totalRequests.toLocaleString()} requests, `}</span>
                          <span className="text-caption text-muted">{share(count, row.totalRequests)}</span>
                        </button>
                      )}
                    </Td>
                  );
                })}
              </tr>
              {open && (
                <tr>
                  <td colSpan={2 + DISPOSITION_ORDER.length} className="border-b border-line bg-raised/40 px-3 py-2">
                    <span className="mr-2 text-caption font-medium text-fg-secondary">
                      {DISPOSITION_LABELS[open.disposition]} · {name}
                    </span>
                    <span className="inline-flex flex-wrap gap-x-3 gap-y-1">
                      {open.requests.map((request) => (
                        <ObjectLink
                          key={request.requestId}
                          target={{ kind: 'request', portalKey: row.portalSubdomain, requestId: request.requestId, label: request.rpid ?? undefined }}
                          className="font-mono text-code text-fg no-underline hover:underline"
                        >
                          {request.rpid ?? `request ${request.requestId}`}
                        </ObjectLink>
                      ))}
                    </span>
                  </td>
                </tr>
              )}
            </Fragment>
          );
        })}
      </tbody>
    </Table>
  );
}
