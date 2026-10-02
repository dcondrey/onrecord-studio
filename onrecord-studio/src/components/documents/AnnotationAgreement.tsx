import { useMemo } from 'react';
import type { ResolvedAnnotation } from '../../types';
import { agreementReport, KIND_LABELS, type AgreementVerdict } from './annotationLogic';

const VERDICT: Record<AgreementVerdict, { label: string; tone: string }> = {
  AGREE: { label: 'Agree on code', tone: 'text-success' },
  DISAGREE: { label: 'Disagree on code', tone: 'text-danger' },
  UNCODED: { label: 'Overlap, neither coded', tone: 'text-fg-secondary' },
};

function describe(item: ResolvedAnnotation) {
  return `${item.author ?? 'unknown'}: ${item.tag ?? 'no code'} (${KIND_LABELS[item.kind].toLowerCase()})`;
}

/** Where two different authors marked overlapping passages of this document, and whether their codes match. */
export default function AnnotationAgreement({ annotations }: { annotations: readonly ResolvedAnnotation[] }) {
  const report = useMemo(() => agreementReport(annotations), [annotations]);
  const left = report.left.orphaned + report.left.unattributed;
  return (
    <details className="rounded border border-line p-2">
      <summary className="cursor-pointer font-semibold text-fg">Agreement between reviewers ({report.pairs.length})</summary>
      <div className="mt-2 space-y-1.5">
        <p className="text-muted">Compares marks by different authors whose passages overlap. A colleague's marks appear here after you import their packet. Agreement means the same code; it says nothing about whether the code is right.</p>
        {report.pairs.length === 0 && <p className="text-muted">No overlapping marks by different authors.</p>}
        <ul aria-label="Agreement pairs" className="space-y-1">
          {report.pairs.map((pair) => (
            <li key={`${pair.first.id}:${pair.second.id}`} className="rounded border border-line p-1.5">
              <p className={`font-semibold ${VERDICT[pair.verdict].tone}`}>{VERDICT[pair.verdict].label}</p>
              <p>{describe(pair.first)}</p>
              <p>{describe(pair.second)}</p>
              <blockquote className="border-l border-line-strong pl-2 text-muted">{pair.first.quote}</blockquote>
            </li>
          ))}
        </ul>
        {left > 0 && (
          <p className="text-warning">
            {left} mark{left === 1 ? '' : 's'} left out: {report.left.orphaned} orphaned (no place to compare), {report.left.unattributed} with no recorded author.
          </p>
        )}
      </div>
    </details>
  );
}
