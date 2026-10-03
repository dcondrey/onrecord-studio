import { Fragment, useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DepartmentDispositions, Disposition } from '../../types';
import MethodNote from '../ui/MethodNote';
import ObjectLink from '../objects/ObjectLink';

/** Below this many requests one request moves a department's share by a fifth or more. */
export const SMALL_DEPARTMENT_REQUESTS = 5;

export const DISPOSITION_LABELS: Record<Disposition, string> = {
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

  return (
    <section aria-label="Dispositions by department" className="space-y-2">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <h3 className="text-sm font-semibold text-fg">Dispositions by department</h3>
        <MethodNote label="How dispositions are counted">
          <p>
            Each request is counted once, from the closure notice its portal published, by the first rule its
            words match: <strong>Withdrawn</strong> (“request withdrawn”, “withdrawn this request”, or a request
            stored as withdrawn); <strong>No responsive records</strong> (“no responsive documents” or “no
            responsive records”); <strong>Withheld</strong> (“not released”, or “any documents responsive to the
            request are being withheld”); <strong>Released in part</strong> (“released except”, “released
            redacted”, “released withheld”); <strong>Released</strong> (“released”).
          </p>
          <p className="mt-1.5">
            <strong>Disposition not recorded</strong> means no closure notice is stored for the request: it is still
            open, or it closed without one. <strong>Notice not classified</strong> means a notice is stored but no
            rule matches its wording, or its text has not been extracted. Neither is counted as any disposition.
            Every share is of the department’s requests in scope, these two included. A department with fewer
            than {SMALL_DEPARTMENT_REQUESTS} requests is marked small, because one request moves its shares by a
            fifth or more. These counts describe what each agency answered; they are not findings.
          </p>
        </MethodNote>
      </div>
      {held.status === 'loading' && <p className="text-label text-muted">Counting dispositions…</p>}
      {held.status === 'failed' && (
        <p role="alert" className="text-label text-danger">Dispositions could not be counted: {held.message}</p>
      )}
      {held.status === 'ready' && held.rows.length === 0 && (
        <p className="text-label text-muted">No requests are stored for this scope.</p>
      )}
      {held.status === 'ready' && held.rows.length > 0 && (
        <div className="overflow-x-auto rounded-xl border border-line bg-canvas/40">
          <table className="w-full text-left text-label" aria-label="Dispositions by department">
            <thead className="border-b border-line text-caption uppercase tracking-wider text-muted">
              <tr>
                <th scope="col" className="px-3 py-2">Department</th>
                <th scope="col" className="px-3 py-2">Portal</th>
                <th scope="col" className="px-3 py-2">Requests (denominator)</th>
                {DISPOSITION_ORDER.map((disposition) => (
                  <th key={disposition} scope="col" className="px-3 py-2">{DISPOSITION_LABELS[disposition]}</th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-line">
              {held.rows.map((row) => {
                const key = departmentKey(row);
                const small = row.totalRequests < SMALL_DEPARTMENT_REQUESTS;
                const name = row.agencyName ?? 'Department not recorded';
                const open = row.buckets.find((bucket) => `${key}\u0000${bucket.disposition}` === expanded);
                return (
                  <Fragment key={key}>
                    <tr className="text-fg-secondary">
                      <td className="px-3 py-2 font-semibold">
                        {row.agencyId === null ? (
                          <span className="text-muted">{name}</span>
                        ) : (
                          <ObjectLink target={{ kind: 'agency', portalKey: row.portalSubdomain, agencyId: row.agencyId, label: name }} className="text-fg decoration-line-strong hover:text-accent">{name}</ObjectLink>
                        )}
                        {small && (
                          <span
                            title={`${row.totalRequests} requests; one request moves each share by a fifth or more.`}
                            className="ml-2 rounded-full border border-line-strong px-2 py-0.5 text-caption font-normal text-muted"
                          >
                            small (n={row.totalRequests})
                          </span>
                        )}
                      </td>
                      <td className="px-3 py-2 text-muted">{row.portalSubdomain}</td>
                      <td className="px-3 py-2 font-mono">{row.totalRequests.toLocaleString()}</td>
                      {DISPOSITION_ORDER.map((disposition) => {
                        const bucket = row.buckets.find((candidate) => candidate.disposition === disposition);
                        const count = bucket?.requests.length ?? 0;
                        const bucketKey = `${key}\u0000${disposition}`;
                        return (
                          <td key={disposition} className="px-3 py-2 font-mono">
                            {count === 0 ? (
                              <span className="text-muted">0</span>
                            ) : (
                              <button
                                type="button"
                                aria-expanded={expanded === bucketKey}
                                aria-label={`${DISPOSITION_LABELS[disposition]}: ${count} of ${row.totalRequests} requests, ${name}`}
                                onClick={() => setExpanded((current) => (current === bucketKey ? null : bucketKey))}
                                className="focus-ring rounded px-1 hover:text-accent"
                              >
                                {count.toLocaleString()} <span className="text-muted">{share(count, row.totalRequests)}</span>
                              </button>
                            )}
                          </td>
                        );
                      })}
                    </tr>
                    {open && (
                      <tr>
                        <td colSpan={3 + DISPOSITION_ORDER.length} className="bg-surface/40 px-3 py-2">
                          <span className="mr-2 text-caption uppercase tracking-wider text-muted">
                            {DISPOSITION_LABELS[open.disposition]} · {name}
                          </span>
                          <span className="inline-flex flex-wrap gap-x-3 gap-y-1">
                            {open.requests.map((request) => (
                              <ObjectLink
                                key={request.requestId}
                                target={{ kind: 'request', portalKey: row.portalSubdomain, requestId: request.requestId, label: request.rpid ?? undefined }}
                                className="font-mono text-fg decoration-line-strong hover:text-accent"
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
          </table>
        </div>
      )}
    </section>
  );
}
