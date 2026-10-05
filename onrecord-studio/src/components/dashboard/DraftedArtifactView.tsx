import { useCallback, useEffect, useState } from "react";
import type {
  DraftedArtifact,
  DraftEvidenceBasis,
  FindingConfidence,
} from "../../types";

const BASIS_LABEL: Record<DraftEvidenceBasis, string> = {
  PATTERN_OBSERVATION: "Pattern observation (not a finding of violation)",
  COUNTED_POLICY_COMPARISON:
    "Counted comparison with the agency's own stated commitment",
  COUNTED_STATUTORY_DEADLINE:
    "Counted against the statutory determination period",
  DEADLINE_PASSED_WITHOUT_RECORDED_RESPONSE:
    "Computed deadline passed with no recorded response (not a finding about the agency's conduct)",
};

export function confidenceLabel(confidence: FindingConfidence): string {
  if (confidence.kind === "UNASSESSED") return "not assessed";
  return `${(confidence.basisPoints / 100).toFixed(2)}% (measured)`;
}

/** The text that leaves the app: the edited body, then the disclosure the body cannot remove. */
export function outgoingText(body: string, disclosure: string | null): string {
  return disclosure === null
    ? body
    : `${body.trimEnd()}\n\n---\nDisclosure: ${disclosure}\n`;
}

/**
 * Editable draft with its evidence basis, confidence, citations and unknown facts. Copy and
 * download use the edited text. Nothing here transmits the draft.
 */
export default function DraftedArtifactView({
  draft,
}: {
  draft: DraftedArtifact;
}) {
  const [editedText, setEditedText] = useState(draft.generatedText);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setEditedText(draft.generatedText);
    setError(null);
  }, [draft]);

  const copyToClipboard = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(
        outgoingText(editedText, draft.disclosure),
      );
    } catch (reason) {
      setError(`Copy to clipboard failed: ${String(reason)}`);
    }
  }, [draft.disclosure, editedText]);

  const download = useCallback(() => {
    const blob = new Blob([outgoingText(editedText, draft.disclosure)], {
      type: "text/plain;charset=utf-8",
    });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement("a");
    anchor.href = url;
    anchor.download = `${draft.kind.toLowerCase()}-${draft.subjectFindingId.replace(/[^A-Za-z0-9_-]+/g, "-")}.txt`;
    document.body.appendChild(anchor);
    anchor.click();
    document.body.removeChild(anchor);
    URL.revokeObjectURL(url);
  }, [draft, editedText]);

  return (
    <div className="mt-3 flex flex-col gap-2">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <div className="text-caption uppercase tracking-wider text-muted">
          {draft.kind === "FOLLOWUP_REQUEST"
            ? "Draft records request"
            : "Draft appeal or complaint"}
          {" · "}not sent{" · "}template {draft.templateId}
          {" · "}jurisdiction {draft.jurisdiction}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={copyToClipboard}
            className="focus-ring rounded-md border border-control px-2 py-1 text-caption font-medium text-fg-secondary transition hover:border-accent/30"
          >
            Copy
          </button>
          <button
            type="button"
            onClick={download}
            className="focus-ring rounded-md border border-control px-2 py-1 text-caption font-medium text-fg-secondary transition hover:border-accent/30"
          >
            Download .txt
          </button>
        </div>
      </div>
      {error && (
        <div
          role="alert"
          className="rounded-md border border-danger/30 bg-danger/5 px-2 py-1.5 text-caption text-danger"
        >
          {error}
        </div>
      )}
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5 text-caption text-muted">
        <dt>Evidence basis</dt>
        <dd className="text-fg-secondary">{BASIS_LABEL[draft.evidenceBasis]}</dd>
        <dt>Confidence</dt>
        <dd className="text-fg-secondary">{confidenceLabel(draft.confidence)}</dd>
      </dl>
      {draft.unknownFacts.length > 0 && (
        <div className="rounded-md border border-warning/30 bg-warning/5 px-2 py-1.5 text-caption text-warning">
          <div className="font-semibold">
            Unknown to the app, fill before sending
          </div>
          <ul className="ml-4 list-disc">
            {draft.unknownFacts.map((fact) => (
              <li key={fact}>{fact}</li>
            ))}
          </ul>
        </div>
      )}
      <textarea
        value={editedText}
        onChange={(event) => setEditedText(event.target.value)}
        aria-label="Drafted artifact text (editable)"
        className="min-h-[220px] w-full resize-y rounded-md border border-control bg-canvas/70 p-3 font-mono text-label leading-relaxed text-fg focus:border-accent/30 focus:outline-none focus:ring-1 focus:ring-accent/30"
      />
      {draft.disclosure !== null && (
        <p
          data-testid="draft-disclosure"
          className="rounded-md border border-line bg-surface/70 px-3 py-2 font-mono text-label leading-relaxed text-fg-secondary"
        >
          Disclosure: {draft.disclosure}
          <span className="ml-2 font-sans text-caption italic text-muted">
            Added to every copy and download; not editable.
          </span>
        </p>
      )}
      {draft.citations.length > 0 && (
        <details className="text-caption text-muted">
          <summary className="">
            Citations ({draft.citations.length})
          </summary>
          <ul className="ml-4 mt-1 list-disc space-y-1">
            {draft.citations.map((cite, index) => (
              <li key={`${cite.portalKey}-${cite.documentId}-${index}`}>
                document {cite.documentId} on {cite.portalKey}
                {cite.characterStart != null && cite.characterEnd != null
                  ? `, characters ${cite.characterStart}-${cite.characterEnd}`
                  : ", character range not recorded"}
                {cite.quote ? `: "${cite.quote}"` : ""}
              </li>
            ))}
          </ul>
        </details>
      )}
      {draft.citedAuthorities.length > 0 && (
        <details className="text-caption text-muted">
          <summary className="">
            Cited authorities ({draft.citedAuthorities.length})
          </summary>
          <ul className="ml-4 mt-1 list-disc space-y-0.5">
            {draft.citedAuthorities.map((authority, index) => (
              <li key={`${authority.statute}-${index}`}>{authority.statute}</li>
            ))}
          </ul>
        </details>
      )}
      <p className="text-caption italic text-muted">
        The app produces drafts only; nothing is sent. Review and edit before
        submitting.
      </p>
    </div>
  );
}
