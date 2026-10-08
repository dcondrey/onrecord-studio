import { Fragment, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DepartmentDispositions, Disposition } from '../../types';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import Badge from '../ui/Badge';
import Icon from '../ui/icons';
import InlineError from '../ui/InlineError';
import MethodNote from '../ui/MethodNote';
import Skeleton from '../ui/Skeleton';
import Table, { Td, Th } from '../ui/Table';
import ObjectLink from '../objects/ObjectLink';

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
  const [expanded, setExpanded] = useState<string | null>(null);
  const savedSources = useAppStore((state) => state.portals);

  useEffect(() => {
    const token = gate.begin();
    setHeld({ status: 'loading' });
    setExpanded(null);
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

  return (
    <section aria-label="Dispositions by department" className="app-panel overflow-hidden">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-2 border-b border-line px-5 py-4">
        <h3 className="text-heading text-fg">Dispositions by department</h3>
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
      </div>
      {held.status === 'loading' && <Skeleton variant="lines" lines={4} label="Counting dispositions" className="px-5 py-4" />}
      {held.status === 'failed' && (
        <div className="p-4"><InlineError summary="Dispositions could not be counted." detail={held.message} /></div>
      )}
      {held.status === 'ready' && held.rows.length === 0 && (
        <p className="px-5 py-4 text-body text-muted">No requests are stored for this scope.</p>
      )}
      {held.status === 'ready' && held.rows.length > 0 && (
        <Table label="Dispositions by department">
          <thead>
            <tr>
              <Th className="min-w-[14rem]">Department</Th>
              <Th numeric>Requests</Th>
              {DISPOSITION_ORDER.map((disposition) => (
                <Th key={disposition} numeric>{DISPOSITION_LABELS[disposition]}</Th>
              ))}
            </tr>
          </thead>
          <tbody>
            {held.rows.map((row) => {
              const key = departmentKey(row);
              const small = row.totalRequests < SMALL_DEPARTMENT_REQUESTS;
              const name = row.agencyName ?? 'Department not recorded';
              const open = row.buckets.find((bucket) => `${key}\u0000${bucket.disposition}` === expanded);
              return (
                <Fragment key={key}>
                  <tr>
                    <Td className="min-w-[14rem]">
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
                      {spansPortals && <p className="mt-0.5 text-caption text-muted" title={row.portalSubdomain}>{portalName(row.portalSubdomain)}</p>}
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
                              aria-label={`${DISPOSITION_LABELS[disposition]}: ${count} of ${row.totalRequests} requests, ${name}`}
                              onClick={() => setExpanded((current) => (current === bucketKey ? null : bucketKey))}
                              className="focus-ring inline-flex items-center gap-1 whitespace-nowrap rounded text-fg underline decoration-dotted decoration-fg/40 underline-offset-4 hover:decoration-solid"
                            >
                              {count.toLocaleString()} <span className="text-muted">({share(count, row.totalRequests)})</span>
                              <Icon name="chevron" size="xs" className={cn('text-muted transition-transform', isOpen && 'rotate-90')} />
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
      )}
    </section>
  );
}
