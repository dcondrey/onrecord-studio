import { useRef, useState } from "react";
import type { ResearchReport } from "../../types";
import {
  buildResearchMarkdown,
  researchCaseSnapshot,
} from "../../utils/researchExport";

function download(name: string, content: string, type: string) {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export default function ResearchExport({
  report,
  actionStates,
}: {
  report: ResearchReport;
  actionStates: Record<string, string>;
}) {
  const [packet, setPacket] = useState<{
    markdown: string;
    json: string;
    exportedAt: string;
  } | null>(null);
  const preview = useRef<HTMLTextAreaElement>(null);
  const prepare = () => {
    const exportedAt = new Date().toISOString();
    setPacket({
      markdown: buildResearchMarkdown(report, actionStates, exportedAt),
      json: JSON.stringify(researchCaseSnapshot(report, actionStates, exportedAt), null, 2),
      exportedAt,
    });
  };
  const filename = `${report.topic.replace(/[^a-z0-9_-]/gi, "-")}-${report.createdAt.slice(0, 10)}-case-packet`;
  return (
    <section aria-label="Export investigation case packet" className="rounded-xl border border-line-strong p-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h4 className="text-sm font-semibold text-fg">Take the case forward</h4>
          <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
            A portable packet with linked citations, exact query results, source hashes,
            request drafts, meeting actions, and your follow-through states.
          </p>
        </div>
        <button type="button" className="analysis-button" onClick={prepare}>
          {packet ? "Refresh case packet" : "Prepare case packet"}
        </button>
      </div>
      {packet && (
        <div className="mt-4 space-y-3">
          <p role="status" className="text-label leading-4 text-muted">
            Snapshot prepared {new Date(packet.exportedAt).toLocaleString()}. Refresh
            the packet to include later casework changes.
          </p>
          <label className="block text-label leading-4 text-fg-secondary">
            Review case packet
            <textarea
              ref={preview}
              readOnly
              value={packet.markdown}
              rows={12}
              className="analysis-input mt-2 w-full resize-y font-mono text-label leading-4"
            />
          </label>
          <div className="flex flex-wrap gap-2">
            <button type="button" className="analysis-button" onClick={() => {
              preview.current?.focus();
              preview.current?.select();
            }}>
              Select text to copy
            </button>
            <button type="button" className="analysis-button" onClick={() => download(`${filename}.md`, packet.markdown, "text/markdown;charset=utf-8")}>
              Download Markdown packet
            </button>
            <button type="button" className="analysis-button" onClick={() => download(`${filename}.json`, packet.json, "application/json;charset=utf-8")}>
              Download evidence bundle (JSON)
            </button>
            <button type="button" className="analysis-button" onClick={() => setPacket(null)}>
              Close packet preview
            </button>
          </div>
          <p className="text-label leading-4 text-muted">
            Markdown is ready for editorial review. JSON preserves the exact saved evidence,
            SQL, and provenance for independent checking. Exporting does not submit requests
            or mark findings verified.
          </p>
        </div>
      )}
    </section>
  );
}
