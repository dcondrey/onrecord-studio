import type { ResearchCitation, ResearchEvidence, ResearchReport } from "../../types";
import Time from "../ui/Time";
import { useAppStore } from "../../store/useAppStore";

export function Citations({
  citations,
  report,
}: {
  citations: ResearchCitation[];
  report: ResearchReport;
}) {
  return (
    <div className="space-y-2">
      {citations.map((citation, index) => (
        <blockquote
          key={index}
          className="border-l-2 border-accent/30 pl-3 text-label leading-relaxed text-fg-secondary"
        >
          <CitationContent
            citation={citation}
            evidence={report.evidence.find((e) => e.id === citation.evidenceId)}
          />
          <EvidenceLinks ids={[citation.evidenceId]} report={report} />
        </blockquote>
      ))}
    </div>
  );
}
type QueryResult = { columns: string[]; rows: unknown[][] };

/** A stored query result, or null when the text is not one. */
function parseQueryResult(text: string): QueryResult | null {
  try {
    const data: { columns?: unknown; rows?: unknown } = JSON.parse(text);
    if (
      Array.isArray(data.columns) &&
      data.columns.every((c) => typeof c === "string") &&
      Array.isArray(data.rows) &&
      data.rows.every(Array.isArray)
    )
      return { columns: data.columns, rows: data.rows };
  } catch {
    // Not JSON: shown verbatim by the caller.
  }
  return null;
}

function QueryTable({ text }: { text: string }) {
  const table = parseQueryResult(text);
  if (!table) {
    return (
      <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words">
        {text}
      </pre>
    );
  }
  return <ResultTable {...table} />;
}

function ResultTable({ columns, rows }: QueryResult) {
  return (
      <div className="max-h-72 max-w-full overflow-auto">
        <table className="w-full text-left text-label leading-4">
          <thead>
            <tr>
              {columns.map((column, i) => (
                <th
                  key={i}
                  className="whitespace-nowrap border-b border-line-strong px-2 py-2 font-semibold text-fg"
                >
                  {column.replace(/_/g, " ")}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, i) => (
              <tr key={i}>
                {row.map((value, j) => (
                  <td
                    key={j}
                    className="whitespace-nowrap border-b border-line px-2 py-2 font-mono"
                  >
                    {value === null ? (
                      <span className="text-warning">Not established</span>
                    ) : typeof value === "object" ? (
                      JSON.stringify(value)
                    ) : (
                      String(value)
                    )}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
  );
}
function CitationContent({
  citation,
  evidence,
}: {
  citation: ResearchCitation;
  evidence: ResearchEvidence | undefined;
}) {
  // A query citation quotes one result row; it is shown under the result's own headers.
  const row = evidence?.sql ? parseQuotedRow(citation.quote) : null;
  const result = row ? parseQueryResult(evidence!.text) : null;
  if (row && result && row.length === result.columns.length)
    return <ResultTable columns={result.columns} rows={[row]} />;
  return <p>“{citation.quote}”</p>;
}

function parseQuotedRow(quote: string): unknown[] | null {
  try {
    const value: unknown = JSON.parse(quote);
    return Array.isArray(value) ? value : null;
  } catch {
    return null;
  }
}

export function EvidenceLinks({
  ids,
  report,
}: {
  ids: string[];
  report: ResearchReport;
}) {
  return (
    <div className="mt-2 flex flex-wrap gap-2">
      {ids.map((id) => {
        const evidence = report.evidence.find((item) => item.id === id);
        if (!evidence) return null;
        const links = [...(evidence.sourceReferences ?? []).map((source) => ({ url: source.url, label: <SnapshotLabel snapshot={source.snapshot} /> })), ...(evidence.sourceUrl ? [{ url: evidence.sourceUrl, label: <>Open original source</> }] : [])].filter((source, index, all) => /^https?:\/\//.test(source.url) && all.findIndex((item) => item.url === source.url) === index);
        return (
          <div key={id} className="min-w-0 w-full text-label leading-4 text-muted">
          <div className="mb-2 flex flex-wrap gap-3">{links.map((source) => <a key={source.url} href={source.url} target="_blank" rel="noreferrer" className="text-accent underline underline-offset-2">{source.label}</a>)}</div>
          <details className="min-w-0 text-label leading-4 text-muted">
            <summary className="text-accent">
              {evidence.label}
            </summary>
            <div className="mt-2 space-y-2 rounded-lg border border-line-strong p-3">
              <p>{evidence.limitations}</p>
              {evidence.sql ? (
                <QueryTable text={evidence.text} />
              ) : (
                <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words">
                  {evidence.text}
                </pre>
              )}
              {evidence.sql && (
                <pre className="max-h-48 overflow-auto whitespace-pre-wrap">
                  {evidence.sql}
                </pre>
              )}
              <p className="break-all">
                Evidence SHA-256: {evidence.sourceSha256 || "Not recorded"}
              </p>
              {(evidence.sourceReferences ?? []).map((source) => (
                <p key={source.snapshot} className="break-all">
                  Source SHA-256 for snapshot {source.snapshot}: {source.sha256}
                </p>
              ))}
              {evidence.portalKey && evidence.requestId !== null && (
                <button
                  type="button"
                  className="analysis-button ml-2"
                  onClick={() => {
                    const state = useAppStore.getState();
                    state.setActivePortal(evidence.portalKey);
                    state.setSearchFilters("", null, null, null);
                    state.setSelectedRequest(
                      evidence.requestId,
                      evidence.portalKey,
                    );
                    state.setWorkspaceView("evidence");
                  }}
                >
                  Review local record
                </button>
              )}
            </div>
          </details>
          </div>
        );
      })}
    </div>
  );
}

/** A snapshot id ("snap-2026-09-30") names its date; the reader needs the date, not the id. */
function SnapshotLabel({ snapshot }: { snapshot: string }) {
  const date = /\d{4}-\d{2}-\d{2}/.exec(snapshot)?.[0];
  return <span title={`Snapshot ${snapshot}`}>Original source{date ? <> · <Time value={date} /></> : null}</span>;
}
