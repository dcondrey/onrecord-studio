import { formatDateTime } from '../components/objects/labels';
import type { CountBasis, SeriesGap, StandingQueryRun } from '../types/standingSearch';

/** Exact and estimated counts are different claims; the label travels with every breakdown. */
export function describeCountBasis(basis: CountBasis): string {
  return basis.basis === 'EXACT'
    ? 'exact counts'
    : `estimates scaled from a sample of ${basis.sampleSize.toLocaleString('en-US')} matches`;
}

export function formatFacetCount(count: number, basis: CountBasis): string {
  return basis.basis === 'EXACT' ? count.toLocaleString('en-US') : `≈${count.toLocaleString('en-US')}`;
}

/** One line naming the corpus state the run saw and whether its answer moved. */
export function describeRun(run: StandingQueryRun): string {
  const corpus = `corpus sha256:${run.corpusDigest.slice(0, 16)} (${run.corpusDocumentCount.toLocaleString('en-US')} documents)`;
  const change =
    run.changedSincePrevious === null
      ? 'first run, nothing to compare'
      : run.changedSincePrevious
        ? `match set changed from ${run.previousMatchCount?.toLocaleString('en-US') ?? '?'} to ${run.result.matchCount.toLocaleString('en-US')}`
        : 'same match set as the previous run';
  return `Ran ${formatDateTime(run.ranAt)} against ${corpus}; ${run.result.matchCount.toLocaleString('en-US')} matching requests (exact); ${change}.`;
}

export function describeGap(gap: SeriesGap): string {
  const pad = (number: number) => String(number).padStart(gap.width, '0');
  const range = gap.fromNumber === gap.toNumber ? pad(gap.fromNumber) : `${pad(gap.fromNumber)} to ${pad(gap.toNumber)}`;
  return `No document in the corpus carries ${gap.label} ${gap.prefix}${range}, between ${gap.before.displayValue} (${gap.before.documentCount} documents) and ${gap.after.displayValue} (${gap.after.documentCount} documents) in a series of ${gap.seriesMemberCount}.`;
}
