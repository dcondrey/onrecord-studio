import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DataPackChange, DataPackChangeKind, DataPackChangeRow, DataPackChangeRows, DataPackLocation } from '../../types';

/** Rows per page: enough to scan, few enough that a page never stalls the panel. */
export const CHANGE_ROWS_PAGE = 100;

const KIND_LABEL: Record<DataPackChangeKind, string> = {
  added: 'Record added',
  removed: 'Record removed',
  changed: 'Field changed',
  column_added: 'Column added',
  column_removed: 'Column removed',
};

function fileName(path: string): string {
  return path.split('/').pop() ?? path;
}

/** A cell the row does not carry is stated as such; a blank the publisher wrote is shown as blank. */
function Cell({ value, none }: { value: string | null; none: string }) {
  if (value === null) return <span className="text-muted" title={none}>—<span className="sr-only">{none}</span></span>;
  if (value === '') return <span className="italic text-muted">(blank)</span>;
  return <>{value}</>;
}

function recordLabel(row: DataPackChangeRow): string | null {
  if (row.publisherRecordId === null) return null;
  return row.keyOccurrence !== null && row.keyOccurrence > 1 ? `${row.publisherRecordId} (repeat ${row.keyOccurrence})` : row.publisherRecordId;
}

interface ChangeRecordRowsProps {
  location: DataPackLocation;
  manifestSha256: string;
  change: DataPackChange;
  onClose(): void;
}

/**
 * One change record's rows, read from the pack a page at a time. The record is checked against
 * the inspected manifest on every read, so a pack changed since inspection is refused, not shown.
 * Mount it keyed by the record's path so paging starts at the first row for each record.
 */
export default function ChangeRecordRows({ location, manifestSha256, change, onClose }: ChangeRecordRowsProps) {
  const gate = useLatestRequestGate();
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<DataPackChangeRows | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The shown page stays while the next one is read, so paging never collapses the panel.
  const [loading, setLoading] = useState(true);
  // Compared by value: a caller may rebuild the location object on every render.
  const locationKey = JSON.stringify(location);

  useEffect(() => {
    const request = gate.begin();
    setLoading(true);
    setError(null);
    void (async () => {
      try {
        const rows = await invokeCommand('read_data_pack_change_record', { location: JSON.parse(locationKey) as DataPackLocation, manifestSha256, path: change.path, offset, limit: CHANGE_ROWS_PAGE });
        if (!gate.isCurrent(request)) return;
        setPage(rows);
        setLoading(false);
      } catch (reason) {
        if (!gate.isCurrent(request)) return;
        setError(formatNativeError(reason));
        setLoading(false);
      }
    })();
  }, [gate, locationKey, manifestSha256, change.path, offset]);

  const notes = page ? [...new Set(page.rows.map((row) => row.note).filter((note): note is string => note !== null))] : [];
  const title = `${fileName(change.previous)} → ${fileName(change.current)}`;

  return (
    <section aria-label={`Change record rows: ${title}`} className="mt-2 rounded-md border border-line bg-surface/40">
      <header className="flex items-center justify-between gap-3 border-b border-line px-3 py-2">
        <div className="min-w-0">
          <p className="truncate text-label font-semibold text-fg" title={change.path}>Change record · {title}</p>
          <p className="text-caption text-muted">The publisher&apos;s cell text, compared between the two retrievals; listed, never imported.</p>
        </div>
        <button type="button" className="analysis-button shrink-0" onClick={onClose}>Close rows</button>
      </header>
      {error && <p role="alert" className="px-3 py-2 text-label text-danger">{error}</p>}
      {!error && !page && <p role="status" className="px-3 py-2 text-label text-muted">Verifying and reading the change record…</p>}
      {page && !error && (
        <>
          {notes.map((note) => <p key={note} className="px-3 pt-2 text-caption text-fg-secondary">{note}.</p>)}
          {page.total === 0 ? (
            <p className="px-3 py-2 text-label text-muted">This change record lists no rows.</p>
          ) : (
            <div tabIndex={0} role="region" aria-label={`Rows of change record ${title}`} aria-busy={loading} className={`focus-ring max-h-80 overflow-auto transition-opacity ${loading ? 'opacity-60' : ''}`}>
              <table className="min-w-full border-collapse text-left text-label">
                <thead className="sticky top-0 bg-canvas">
                  <tr>
                    {['Change', 'Record', 'Field', 'Before', 'After'].map((column) => <th key={column} scope="col" className="border-b border-line px-2 py-1 font-semibold text-fg-secondary">{column}</th>)}
                  </tr>
                </thead>
                <tbody>
                  {page.rows.map((row, index) => (
                    <tr key={page.offset + index} className="odd:bg-surface/40 align-top">
                      <td className="whitespace-nowrap border-b border-line px-2 py-1 text-fg">{KIND_LABEL[row.change]}</td>
                      <td className="border-b border-line px-2 py-1 font-mono text-fg-secondary"><Cell value={recordLabel(row)} none="no publisher key" /></td>
                      <td className="border-b border-line px-2 py-1 text-fg-secondary"><Cell value={row.field} none="the whole record" /></td>
                      <td className="max-w-[240px] break-words border-b border-line px-2 py-1 text-fg-secondary"><Cell value={row.before} none="not in the earlier retrieval" /></td>
                      <td className="max-w-[240px] break-words border-b border-line px-2 py-1 text-fg-secondary"><Cell value={row.after} none="not in the later retrieval" /></td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
          {page.total > 0 && (
            <footer className="flex items-center justify-between gap-3 px-3 py-2 text-caption text-muted">
              <span role="status" className="viz-num">{loading ? 'Reading rows…' : `Rows ${(page.offset + 1).toLocaleString('en-US')}–${(page.offset + page.rows.length).toLocaleString('en-US')} of ${page.total.toLocaleString('en-US')}`}</span>
              {page.total > CHANGE_ROWS_PAGE && (
                <span className="flex gap-2">
                  <button type="button" className="analysis-button" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - CHANGE_ROWS_PAGE))}>Previous</button>
                  <button type="button" className="analysis-button" disabled={offset + CHANGE_ROWS_PAGE >= page.total} onClick={() => setOffset(offset + CHANGE_ROWS_PAGE)}>Next</button>
                </span>
              )}
            </footer>
          )}
        </>
      )}
    </section>
  );
}
