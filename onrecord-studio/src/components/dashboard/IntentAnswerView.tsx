import type { EvidenceBasis, IntentAnswer, IntentCitation, IntentOption, IntentRow, IntentStatus } from '../../types';
import { cn } from '../../utils/cn';

const BASIS_LABEL: Record<EvidenceBasis, string> = {
  STORED_FINDING: 'Stored finding',
  TYPED_RELATIONSHIP: 'Typed relationship',
  CO_OCCURRENCE_ONLY: 'Co-occurrence only',
  FULL_TEXT_TERM_MATCH: 'Text match',
  STATUTORY_SOURCED_DETERMINATION: 'Sourced determination',
  STATUTORY_ACTIVITY_PROXY: 'Activity stand-in',
  STATUTORY_PROVISIONAL_OBSERVATION: 'Still running',
};

/** Only a stated relationship or a sourced date is a measurement; the rest are signals or proxies. */
const BASIS_IS_MEASURED = new Set<EvidenceBasis>(['TYPED_RELATIONSHIP', 'STATUTORY_SOURCED_DETERMINATION']);

const STATUS_LABEL: Record<IntentStatus, string> = {
  ANSWERED: 'Answered',
  NO_MATCH: 'No match',
  NOT_UNDERSTOOD: 'Not understood',
  NEEDS_SUBJECT: 'Needs a name',
  AMBIGUOUS: 'More than one fit',
  UNAVAILABLE: 'Not searched',
};

function citationLocation(citation: IntentCitation): string {
  const parts = [citation.portalKey];
  if (citation.documentId !== null) parts.push(`document ${citation.documentId}`);
  if (citation.characterStart !== null && citation.characterEnd !== null) {
    parts.push(`characters ${citation.characterStart}-${citation.characterEnd}`);
  }
  return parts.join(' · ');
}

function Citation({ citation }: { citation: IntentCitation }) {
  return (
    <li className="mt-1.5">
      <div className="text-label font-medium text-fg-secondary">{citationLocation(citation)}</div>
      {citation.label && <div className="text-label text-muted">{citation.label}</div>}
      {citation.quote && (
        <blockquote className="mt-0.5 border-l-2 border-line-strong pl-2 text-label italic text-fg-secondary">
          {citation.quote}
        </blockquote>
      )}
    </li>
  );
}

function Row({ row }: { row: IntentRow }) {
  const measured = BASIS_IS_MEASURED.has(row.basis);
  return (
    <article className="rounded-xl border border-line px-3 py-2.5" data-basis={row.basis}>
      <div className="flex flex-wrap items-center gap-1.5">
        <span
          title={row.basisStatement}
          className={cn(
            'rounded-full border px-1.5 py-0.5 text-label font-semibold',
            measured ? 'border-line-strong text-fg' : 'border-dashed border-line-strong text-muted',
          )}
        >
          {BASIS_LABEL[row.basis]}
        </span>
        {row.relationshipKind && (
          <span className="rounded-full border border-line-strong px-1.5 py-0.5 text-label font-semibold text-fg-secondary">
            {row.relationshipKind}
          </span>
        )}
        <span className="text-label text-muted">
          {row.confidenceBasisPoints === null
            ? 'Confidence not assessed'
            : `Confidence ${(row.confidenceBasisPoints / 100).toFixed(0)}%`}
        </span>
      </div>
      <p className="mt-1.5 text-label font-medium text-fg">{row.headline}</p>
      {row.detail && <p className="mt-0.5 text-label text-muted">{row.detail}</p>}
      <p className="mt-1 text-label text-muted">{row.basisStatement}</p>
      {row.citations.length > 0 ? (
        <ul className="mt-1" aria-label="Citations">
          {row.citations.map((citation, index) => (
            <Citation key={`${citation.portalKey}-${citation.documentId}-${citation.characterStart}-${index}`} citation={citation} />
          ))}
          {row.omittedCitationCount > 0 && (
            <li className="mt-1.5 text-label text-muted">{row.omittedCitationCount} more citation(s) not shown.</li>
          )}
        </ul>
      ) : (
        <p className="mt-1 text-label text-muted">
          Citation unavailable: {row.citationsUnavailableReason ?? 'no reason was recorded.'}
        </p>
      )}
    </article>
  );
}

interface IntentAnswerViewProps {
  answer: IntentAnswer;
  onChooseOption: (option: IntentOption) => void;
}

export default function IntentAnswerView({ answer, onChooseOption }: IntentAnswerViewProps) {
  return (
    <div className="space-y-2.5 p-2">
      <div className="px-2 pt-1">
        <div className="flex items-center gap-2">
          <span className="rounded-full border border-line-strong px-1.5 py-0.5 text-label font-semibold text-fg-secondary">
            {STATUS_LABEL[answer.status]}
          </span>
          <span className="truncate text-label text-muted">{answer.utterance}</span>
        </div>
        <p className="mt-1.5 text-label text-fg">{answer.summary}</p>
      </div>
      {answer.rows.map((row) => (
        <Row key={row.id} row={row} />
      ))}
      {answer.truncated && (
        <p className="px-2 text-label text-muted">More rows matched than the limit of {answer.rowLimit}; only the first are shown.</p>
      )}
      {answer.notes.length > 0 && (
        <ul className="space-y-1 px-2" aria-label="Limits of this answer">
          {answer.notes.map((note) => (
            <li key={note} className="text-label text-muted">{note}</li>
          ))}
        </ul>
      )}
      {answer.options.length > 0 && (
        <ul className="space-y-1" aria-label="Questions you can ask">
          {answer.options.map((option) => (
            <li key={`${option.label}-${option.example}`}>
              <button
                type="button"
                onClick={() => onChooseOption(option)}
                className="command-row focus-ring w-full rounded-xl px-3 py-2 text-left"
              >
                <span className="block text-label font-medium text-fg">{option.label}</span>
                <span className="block text-label text-muted">{option.description}</span>
                <span className="mt-0.5 block text-label text-fg-secondary">Try: {option.example}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
