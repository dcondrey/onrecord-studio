import type { ResearchCitation, ResearchEvidence, ResearchReport } from "../../types";
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
function QueryTable({ text }: { text: string }) {
  try {
    const data: { columns: unknown; rows: unknown } = JSON.parse(text);
    if (
      !Array.isArray(data.columns) ||
      !data.columns.every((c) => typeof c === "string") ||
      !Array.isArray(data.rows) ||
      !data.rows.every(Array.isArray)
    )
      throw new Error("Not tabular");
    const columns = data.columns as string[];
    const rows = data.rows as unknown[][];
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
  } catch {
    return (
      <pre className="max-h-60 overflow-auto whitespace-pre-wrap break-words">
        {text}
      </pre>
    );
  }
}
function CitationContent({
  citation,
  evidence,
}: {
  citation: ResearchCitation;
  evidence: ResearchEvidence | undefined;
}) {
  if (evidence?.sql) {
    try {
      const quote: unknown = JSON.parse(citation.quote);
      const result = JSON.parse(evidence.text);
      if (
        Array.isArray(quote) &&
        Array.isArray(result.columns) &&
        quote.length === result.columns.length
      )
        return (
          <QueryTable
            text={JSON.stringify({ columns: result.columns, rows: [quote] })}
          />
        );
    } catch {
      /* Display other exact citations verbatim. */
    }
  }
  return <p>“{citation.quote}”</p>;
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
        const links = [...(evidence.sourceReferences ?? []).map((source) => ({ url: source.url, label: `Original source · ${source.snapshot}` })), ...(evidence.sourceUrl ? [{ url: evidence.sourceUrl, label: "Open original source" }] : [])].filter((source, index, all) => /^https?:\/\//.test(source.url) && all.findIndex((item) => item.url === source.url) === index);
        return (
          <div key={id} className="min-w-0 w-full text-label leading-4 text-muted">
          <div className="mb-2 flex flex-wrap gap-3">{links.map((source) => <a key={source.url} href={source.url} target="_blank" rel="noreferrer" className="text-accent underline underline-offset-2">{source.label}</a>)}</div>
          <details className="min-w-0 text-label leading-4 text-muted">
            <summary className="cursor-pointer text-accent">
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
              {(evidence.sourceReferences ?? []).map(
                (source) =>
                  /^https?:\/\//.test(source.url) && (
                    <div key={source.snapshot} className="space-y-1 break-all">
                      <a
                        href={source.url}
                        target="_blank"
                        rel="noreferrer"
                        className="text-accent"
                      >
                        Open source for snapshot {source.snapshot}
                      </a>
                      <p>Source SHA-256: {source.sha256}</p>
                    </div>
                  ),
              )}
              {evidence.sourceUrl &&
                /^https?:\/\//.test(evidence.sourceUrl) && (
                  <a
                    href={evidence.sourceUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="text-accent"
                  >
                    Open original source
                  </a>
                )}
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
