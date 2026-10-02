import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { InferenceReport } from '../../types';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';

/**
 * Renders the ranked candidate identities for a specific redaction span the user selected in the
 * document viewer. Candidates and evidence come from `infer_redaction`, which uses the semantic
 * path when the fastembed model is loaded and falls back to lexical matching otherwise. The
 * confidence ceiling is bounded — no candidate exceeds the engine's ceiling — and the panel
 * repeats that framing above the list so a user never mistakes an inference for a decoded value.
 */
export default function RedactionInferencePanel({
  documentId,
  portalKey,
  spanStart,
  spanEnd,
  onClose,
}: {
  documentId: number;
  portalKey: string;
  spanStart: number;
  spanEnd: number;
  onClose?: () => void;
}) {
  const [report, setReport] = useState<InferenceReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const gate = useLatestRequestGate();

  useEffect(() => {
    const token = gate.begin();
    setIsLoading(true);
    setError(null);
    setReport(null);
    invokeCommand('infer_redaction', { documentId, portalKey, spanStart, spanEnd })
      .then((result) => {
        if (gate.isCurrent(token)) {
          setReport(result);
          setIsLoading(false);
        }
      })
      .catch((reason) => {
        if (gate.isCurrent(token)) {
          setError(String(reason));
          setIsLoading(false);
        }
      });
  }, [documentId, portalKey, spanStart, spanEnd, gate]);

  return (
    <section className="rounded-lg border border-line bg-canvas/70 p-4" aria-label="Inferred identity for this redaction">
      <header className="mb-3 flex items-baseline justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-fg">Inferred identity for this redaction</h3>
          <p className="mt-1 text-label leading-4 text-muted">
            Candidates below are inferred from the words around the redaction and from other released documents in
            this corpus that share the same context. This is inference, not decryption. Treat each candidate as a
            lead to verify.
          </p>
        </div>
        {onClose && (
          <button type="button" onClick={onClose} className="text-label leading-4 text-muted underline">
            Close
          </button>
        )}
      </header>

      {isLoading && <p className="text-label leading-4 text-muted">Reading corpus for corroborating context…</p>}
      {error && <p className="text-label leading-4 text-danger">{error}</p>}

      {report && !isLoading && (
        <>
          <p className="mb-3 text-label leading-4 text-muted">
            {report.semanticRankingUsed
              ? 'Ranked using the local semantic model.'
              : 'Ranked using lexical corroboration (semantic model not initialized).'}
          </p>

          {report.contextSnippet && (
            <blockquote className="mb-3 rounded border border-line bg-surface/70 p-3 text-label leading-4 text-fg">
              …{report.contextSnippet}…
            </blockquote>
          )}

          {report.candidates.length === 0 ? (
            <p className="text-label leading-4 text-muted">
              No corroborating context found for this redaction. Nothing to infer.
            </p>
          ) : (
            <ol className="space-y-3">
              {report.candidates.map((candidate, index) => (
                <li key={`${candidate.value}-${index}`} className="rounded border border-line p-3">
                  <div className="flex items-baseline justify-between gap-2">
                    <div className="text-sm text-fg">
                      <span className="font-medium">{candidate.value}</span>
                      <span className="ml-2 text-label leading-4 uppercase tracking-wide text-muted">{candidate.kind}</span>
                    </div>
                    <div className="text-label leading-4 text-muted" aria-label={`likelihood ${candidate.likelihoodBasisPoints} basis points`}>
                      {(candidate.likelihoodBasisPoints / 100).toFixed(1)}%
                    </div>
                  </div>
                  <p className="mt-1 text-label leading-4 text-muted">
                    Supported by {candidate.supportingEvidence.length} corroborating document
                    {candidate.supportingEvidence.length === 1 ? '' : 's'}.
                  </p>
                  <ul className="mt-2 space-y-1 text-label leading-4 text-fg-secondary">
                    {candidate.supportingEvidence.slice(0, 3).map((evidence, evidenceIndex) => (
                      <li key={`${evidence.documentId}-${evidenceIndex}`} className="rounded bg-surface/70 p-2">
                        <div className="text-muted">
                          document {evidence.documentId} ({evidence.portalKey})
                        </div>
                        <div className="mt-1 italic">"{evidence.quote}"</div>
                      </li>
                    ))}
                    {candidate.supportingEvidence.length > 3 && (
                      <li className="text-muted">and {candidate.supportingEvidence.length - 3} more.</li>
                    )}
                  </ul>
                </li>
              ))}
            </ol>
          )}
        </>
      )}
    </section>
  );
}
