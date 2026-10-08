import { formatDate } from '../core/format';
import { parseDate } from '../core/scale';

export interface ChartSources {
  /** The tables the plotted rows were read from. Required: every chart names its source. */
  tables: string[];
  snapshot?: { name: string; id: string; sha256?: string };
  /** ISO date or timestamp the source was retrieved. */
  retrievedAt?: string;
  note?: string;
}

/** 'Source: table_a, table_b · Snapshot … · Retrieved …', printed under every chart. */
export function SourceCaption({ sources }: { sources: ChartSources }) {
  const retrieved = sources.retrievedAt ? parseDate(sources.retrievedAt) : Number.NaN;
  return (
    <p className="text-caption text-muted">
      <span>Source: </span>
      {sources.tables.map((table, index) => (
        <span key={table}>
          {index > 0 && ', '}
          <code className="font-mono text-code text-fg-secondary">{table}</code>
        </span>
      ))}
      {sources.snapshot && (
        <span>
          {' · Snapshot '}
          {sources.snapshot.name}
          {' '}
          <span className="font-mono text-code" title={sources.snapshot.id}>({sources.snapshot.id.slice(0, 8)})</span>
          {sources.snapshot.sha256 && (
            <span className="font-mono text-code" title={`SHA-256 ${sources.snapshot.sha256}`}>{` · SHA-256 ${sources.snapshot.sha256.slice(0, 12)}…`}</span>
          )}
        </span>
      )}
      {Number.isFinite(retrieved) && <span>{` · Retrieved ${formatDate(retrieved, 'day')}`}</span>}
      {sources.note && <span>{` · ${sources.note}`}</span>}
    </p>
  );
}
