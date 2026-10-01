import { useMemo } from 'react';
import type { ResolvedAnnotation } from '../../types';
import { agreementReport, KIND_LABELS, type AgreementVerdict } from './annotationLogic';

const VERDICT: Record<AgreementVerdict, { label: string; tone: string }> = {
  AGREE: { label: 'Agree on code', tone: 'text-emerald-200' },
  DISAGREE: { label: 'Disagree on code', tone: 'text-rose-200' },
  UNCODED: { label: 'Overlap, neither coded', tone: 'text-slate-300' },
};

function describe(item: ResolvedAnnotation) {
  return `${item.author ?? 'unknown'}: ${item.tag ?? 'no code'} (${KIND_LABELS[item.kind].toLowerCase()})`;
}

/** Where two different authors marked overlapping passages of this document, and whether their codes match. */
export default function AnnotationAgreement({ annotations }: { annotations: readonly ResolvedAnnotation[] }) {
  const report = useMemo(() => agreementReport(annotations), [annotations]);
  const left = report.left.orphaned + report.left.unattributed;
  return (
    <details className="rounded border border-slate-800 p-2">
      <summary className="cursor-pointer font-semibold text-slate-200">Agreement between reviewers ({report.pairs.length})</summary>
      <div className="mt-2 space-y-1.5">
        <p className="text-slate-400">Compares marks by different authors whose passages overlap. A colleague's marks appear here after you import their packet. Agreement means the same code; it says nothing about whether the code is right.</p>
        {report.pairs.length === 0 && <p className="text-slate-400">No overlapping marks by different authors.</p>}
        <ul aria-label="Agreement pairs" className="space-y-1">
          {report.pairs.map((pair) => (
            <li key={`${pair.first.id}:${pair.second.id}`} className="rounded border border-slate-800 p-1.5">
              <p className={`font-semibold ${VERDICT[pair.verdict].tone}`}>{VERDICT[pair.verdict].label}</p>
              <p>{describe(pair.first)}</p>
              <p>{describe(pair.second)}</p>
              <blockquote className="border-l border-slate-600 pl-2 text-slate-400">{pair.first.quote}</blockquote>
            </li>
          ))}
        </ul>
        {left > 0 && (
          <p className="text-amber-200">
            {left} mark{left === 1 ? '' : 's'} left out: {report.left.orphaned} orphaned (no place to compare), {report.left.unattributed} with no recorded author.
          </p>
        )}
      </div>
    </details>
  );
}
