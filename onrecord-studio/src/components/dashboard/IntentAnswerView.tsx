import type { EvidenceBasis, IntentAnswer, IntentCitation, IntentOption, IntentRow, IntentStatus } from '../../types';
import { useAppStore } from '../../store/useAppStore';
import Badge from '../ui/Badge';

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

const QUOTE_LIMIT = 220;

const SENTENCE_END = '.!?"”’)]';

/**
 * Cuts at the last word boundary inside the limit so a quote never ends mid-word. A stored
 * excerpt can itself stop just after starting a new sentence ("…were located. T"); that dangling
 * fragment is dropped and marked as cut.
 */
function trimQuote(quote: string): string {
  const trimmed = quote.trim();
  if (trimmed.length <= QUOTE_LIMIT) {
    const lastSpace = trimmed.lastIndexOf(' ');
    const dangling = lastSpace > 0 && SENTENCE_END.includes(trimmed[lastSpace - 1]) && trimmed.length - lastSpace - 1 <= 2;
    return dangling ? `${trimmed.slice(0, lastSpace)} …` : trimmed;
  }
  const cut = trimmed.slice(0, QUOTE_LIMIT);
  const boundary = cut.lastIndexOf(' ');
  return `${(boundary > 0 ? cut.slice(0, boundary) : cut).replace(/[\s.,;:]+$/, '')}…`;
}

function citationOffsets(citation: IntentCitation): string | undefined {
  const parts: string[] = [];
  if (citation.documentId !== null) parts.push(`Document ${citation.documentId}`);
  if (citation.characterStart !== null && citation.characterEnd !== null) {
    parts.push(`characters ${citation.characterStart}–${citation.characterEnd}`);
  }
  return parts.length > 0 ? parts.join(', ') : undefined;
}

function Citation({ citation, sourceName }: { citation: IntentCitation; sourceName: string }) {
  return (
    <li className="mt-2">
      <div className="text-caption font-medium text-fg-secondary" title={citationOffsets(citation)}>{sourceName}</div>
      {citation.label && <div className="text-caption text-muted">{citation.label}</div>}
      {citation.quote && (
        <blockquote className="mt-1 border-l-2 border-line-strong pl-2 text-body italic text-fg-secondary">
          {trimQuote(citation.quote)}
        </blockquote>
      )}
    </li>
  );
}

function Row({ row, sourceName, onOpen }: { row: IntentRow; sourceName: (portalKey: string) => string; onOpen: ((id: string) => void) | null }) {
  const measured = BASIS_IS_MEASURED.has(row.basis);
  return (
    <article className="px-3 py-3" data-basis={row.basis}>
      <div className="flex flex-wrap items-center gap-1.5">
        <Badge tone="neutral" dashed={!measured} title={row.basisStatement}>{BASIS_LABEL[row.basis]}</Badge>
        {row.relationshipKind && <Badge tone="neutral">{row.relationshipKind}</Badge>}
        <span className="text-caption tabular-nums text-muted">
          {row.confidenceBasisPoints === null
            ? 'Confidence not assessed'
            : `Confidence ${(row.confidenceBasisPoints / 100).toFixed(0)}%`}
        </span>
      </div>
      {onOpen ? (
        <button
          type="button"
          onClick={() => onOpen(row.id)}
          className="mt-1.5 block text-left text-control text-fg underline-offset-2 hover:underline"
        >
          {row.headline}
        </button>
      ) : (
        <p className="mt-1.5 text-control text-fg">{row.headline}</p>
      )}
      {row.detail && <p className="mt-0.5 text-body text-muted">{row.detail}</p>}
      {row.citations.length > 0 ? (
        <ul className="mt-1" aria-label="Citations">
          {row.citations.map((citation, index) => (
            <Citation key={`${citation.portalKey}-${citation.documentId}-${citation.characterStart}-${index}`} citation={citation} sourceName={sourceName(citation.portalKey)} />
          ))}
          {row.omittedCitationCount > 0 && (
            <li className="mt-2 text-caption tabular-nums text-muted">
              {row.omittedCitationCount} more citation{row.omittedCitationCount === 1 ? '' : 's'}
            </li>
          )}
        </ul>
      ) : (
        <p className="mt-1 text-caption text-muted">
          Citation unavailable: {row.citationsUnavailableReason ?? 'no reason was recorded.'}
        </p>
      )}
    </article>
  );
}

interface IntentAnswerViewProps {
  answer: IntentAnswer;
  onChooseOption: (option: IntentOption) => void;
  /** Called before a stored finding opens in the inspector, so the palette can close first. */
  onOpenFinding: () => void;
}

export default function IntentAnswerView({ answer, onChooseOption, onOpenFinding }: IntentAnswerViewProps) {
  const portals = useAppStore((state) => state.portals);
  const inspect = useAppStore((state) => state.inspect);
  const sourceName = (portalKey: string) =>
    portals.find((saved) => saved.registration.portalKey === portalKey)?.registration.displayName ?? portalKey;
  const openFinding = (id: string) => {
    onOpenFinding();
    // The palette returns focus to its opener on the next frame; the inspector takes it after that.
    window.requestAnimationFrame(() => inspect({ kind: 'finding', id }, 'view'));
  };
  return (
    <div className="space-y-2 p-2">
      <div className="px-2 pt-1">
        {answer.status !== 'ANSWERED' && <Badge tone="neutral" className="mb-2">{STATUS_LABEL[answer.status]}</Badge>}
        <p className="text-body text-fg">{answer.summary}</p>
      </div>
      {answer.rows.length > 0 && (
        <div className="divide-y divide-line-strong/70 rounded-lg border border-line-strong/70">
          {answer.rows.map((row) => (
            <Row key={row.id} row={row} sourceName={sourceName} onOpen={row.basis === 'STORED_FINDING' ? openFinding : null} />
          ))}
        </div>
      )}
      {answer.truncated && (
        <p className="px-2 text-caption text-muted">More rows matched than the limit of {answer.rowLimit}; only the first are shown.</p>
      )}
      {answer.notes.length > 0 && (
        <ul className="space-y-1 px-2" aria-label="Limits of this answer">
          {answer.notes.map((note) => (
            <li key={note} className="text-caption text-muted">{note}</li>
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
                className="command-row w-full rounded-lg px-3 py-2 text-left"
              >
                <span className="block text-control text-fg">{option.label}</span>
                <span className="mt-0.5 block text-caption text-muted">{option.description}</span>
                <span className="mt-0.5 block text-caption text-fg-secondary">Try: {option.example}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
