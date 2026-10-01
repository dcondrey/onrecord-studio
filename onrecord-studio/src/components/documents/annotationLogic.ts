import type { AnnotationKind, AnnotationStatus, ResolvedAnnotation } from '../../types';

export const KIND_LABELS: Record<AnnotationKind, string> = {
  NOTE: 'Note',
  HIGHLIGHT: 'Highlight',
  ISSUE: 'Issue code',
};

export const STATUS_LABELS: Record<AnnotationStatus, string> = {
  OPEN: 'Open',
  CONFIRMED: 'Confirmed',
  DISPUTED: 'Disputed',
};

export const OFFSET_STATEMENT = 'Offsets locate text in the extracted copy of the record that this application stored (personal data masked in quotes), not in the original file or its pages.';

export function isCoded(tag: string | null, codes: readonly { code: string }[]): boolean {
  return tag !== null && codes.some((entry) => entry.code === tag);
}

export interface CitationContext {
  fileName?: string | null;
  /** Digest of the document's stored text as it is now; null when it is not recorded. */
  currentTextSha256: string | null;
  appVersion: string;
}

/**
 * A citation that can be checked later: where the passage is, what it said, and which text it was
 * located in. A passage that cannot be found in the current text is cited as it was recorded and
 * says so.
 */
export function annotationCitation(annotation: ResolvedAnnotation, context: CitationContext): string {
  const located = annotation.anchor && annotation.anchor.state !== 'ORPHANED' ? annotation.anchor : null;
  const offset = located?.sourceOffset ?? annotation.characterOffset;
  const length = located?.sourceLength ?? null;
  const digest = located ? context.currentTextSha256 : annotation.textSha256;
  const lines = [
    'OnRecord Studio citation',
    `Portal: ${annotation.portalKey}`,
    `Document id: ${annotation.documentId}${context.fileName ? ` (${context.fileName})` : ''}`,
  ];
  if (annotation.quote) {
    lines.push(
      `Character offset: ${offset === null ? 'not recorded' : offset.toLocaleString('en-US')}${length === null ? '' : `, ${length.toLocaleString('en-US')} characters`}`,
      `Quote: "${annotation.quote.replace(/\s*\n\s*/g, ' ')}"`,
    );
    if (!located) lines.push('This passage was not found in the current text; the offset and digest are as recorded when it was marked.');
  }
  lines.push(
    `Document text SHA-256: ${digest ?? 'not recorded'}`,
    `Annotation: ${annotation.id}${annotation.author ? ` by ${annotation.author}` : ''}, ${STATUS_LABELS[annotation.status].toLowerCase()}${annotation.tag ? `, code ${annotation.tag}` : ''}`,
    `OnRecord Studio version ${context.appVersion}`,
    OFFSET_STATEMENT,
  );
  return lines.join('\n');
}

export type AgreementVerdict = 'AGREE' | 'DISAGREE' | 'UNCODED';

export interface AgreementPair {
  first: ResolvedAnnotation;
  second: ResolvedAnnotation;
  verdict: AgreementVerdict;
}

export interface AgreementReport {
  pairs: AgreementPair[];
  /** Marks left out because they could not be compared, by reason. */
  left: { orphaned: number; unattributed: number; noPassage: number };
}

function range(annotation: ResolvedAnnotation): { start: number; end: number } | null {
  const anchor = annotation.anchor;
  if (!anchor || anchor.state === 'ORPHANED' || anchor.sourceOffset === null || anchor.sourceLength === null) return null;
  return { start: anchor.sourceOffset, end: anchor.sourceOffset + anchor.sourceLength };
}

/**
 * Pairs of marks by different authors over overlapping passages. Ranges that only touch do not
 * overlap. A mark that is orphaned, has no passage, or has no author cannot be placed against
 * anyone's, so it is counted as left out rather than guessed at.
 */
export function agreementReport(annotations: readonly ResolvedAnnotation[]): AgreementReport {
  const left = { orphaned: 0, unattributed: 0, noPassage: 0 };
  const comparable: { annotation: ResolvedAnnotation; start: number; end: number }[] = [];
  for (const annotation of annotations) {
    if (annotation.quote === '') { left.noPassage += 1; continue; }
    if (annotation.anchor?.state === 'ORPHANED') { left.orphaned += 1; continue; }
    if (annotation.author === null) { left.unattributed += 1; continue; }
    const placed = range(annotation);
    if (!placed) { left.noPassage += 1; continue; }
    comparable.push({ annotation, ...placed });
  }
  const pairs: AgreementPair[] = [];
  for (let i = 0; i < comparable.length; i += 1) {
    for (let j = i + 1; j < comparable.length; j += 1) {
      const a = comparable[i];
      const b = comparable[j];
      if (a.annotation.author === b.annotation.author || a.start >= b.end || b.start >= a.end) continue;
      const codeA = a.annotation.tag;
      const codeB = b.annotation.tag;
      pairs.push({
        first: a.annotation,
        second: b.annotation,
        verdict: codeA === null && codeB === null ? 'UNCODED' : codeA === codeB ? 'AGREE' : 'DISAGREE',
      });
    }
  }
  return { pairs, left };
}

export function downloadText(fileName: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const link = document.createElement('a');
  link.href = url;
  link.download = fileName;
  link.click();
  URL.revokeObjectURL(url);
}

/** Where to scroll to a passage inside the text that is on screen: the occurrence nearest where it was recorded. */
export function findPassageInText(
  text: string,
  passage: { quote: string; prefix: string; suffix: string },
  expectedIndex: number,
): number | null {
  const nearest = (needle: string, skip: number): number | null => {
    let best: number | null = null;
    let from = 0;
    for (;;) {
      const at = text.indexOf(needle, from);
      if (at === -1) break;
      const candidate = at + skip;
      if (best === null || Math.abs(candidate - expectedIndex) < Math.abs(best - expectedIndex)) best = candidate;
      from = at + 1;
    }
    return best;
  };
  return nearest(`${passage.prefix}${passage.quote}${passage.suffix}`, passage.prefix.length) ?? nearest(passage.quote, 0);
}
