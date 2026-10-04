import type { CitationAnchorState, FindingCitation, FindingCitationAnchor } from '../../types/findings';

export type CitationAnchorTone = 'intact' | 'drifted' | 'stale' | 'unchecked';

export interface CitationAnchorText {
  tone: CitationAnchorTone;
  label: string;
  detail: string;
}

/**
 * The resolution for one citation, or `null` when the answer was read for a different version of
 * this citation (the finding was re-recorded in between) and so says nothing about this one.
 */
export function anchorForCitation(
  anchors: FindingCitationAnchor[],
  citation: FindingCitation,
  index: number,
): CitationAnchorState | null {
  const match = anchors.find((entry) => entry.citationIndex === index);
  if (!match) return null;
  const same = match.documentId === citation.documentId
    && match.characterStart === citation.characterStart
    && match.characterEnd === citation.characterEnd;
  return same ? match.anchor : null;
}

/** Plain wording for where a cited passage stands. `null` when there is nothing to add. */
export function describeCitationAnchor(anchor: CitationAnchorState): CitationAnchorText | null {
  if (anchor.kind === 'UNANCHORED') {
    if (anchor.reason === 'NO_RANGE') return null;
    return {
      tone: 'unchecked',
      label: 'Position not verifiable',
      detail: 'This citation was not anchored to the document text, so whether its range still holds the quoted words cannot be checked.',
    };
  }
  const resolution = anchor.resolution;
  switch (resolution.state) {
    case 'INTACT':
      return {
        tone: 'intact',
        label: 'Passage in place',
        detail: resolution.projection === 'UNCHANGED'
          ? 'The document text is unchanged since this was cited.'
          : resolution.projection === 'CHANGED'
            ? 'The document text was re-extracted since this was cited, and the cited words are still at this position.'
            : 'The cited words are at this position. Which version of the text was cited was not recorded.',
      };
    case 'DRIFTED':
      return {
        tone: 'drifted',
        label: 'Passage moved',
        detail: `The document text changed since this was cited. The cited words were found once, now at characters ${resolution.start}–${resolution.end}. The citation still records its original range.`,
      };
    case 'STALE':
      return {
        tone: 'stale',
        label: resolution.reason === 'TEXT_UNAVAILABLE' ? 'Passage cannot be checked' : 'Passage not found',
        detail: resolution.reason === 'MISSING'
          ? 'The cited words are no longer in the document text. The range is where they were; it is not marked in the document.'
          : resolution.reason === 'AMBIGUOUS'
            ? 'The cited words now appear more than once, so which one was cited cannot be told. Nothing is marked in the document.'
            : 'The document has no extracted text to check the citation against.',
      };
  }
}

/**
 * The citation as the document should be opened at: a moved passage at its new position, a lost
 * one with no position, so the viewer never marks words the citation did not cite.
 */
export function locateCitation(citation: FindingCitation, anchor: CitationAnchorState | null): FindingCitation {
  if (anchor?.kind !== 'ANCHORED') return citation;
  const resolution = anchor.resolution;
  if (resolution.state === 'STALE') return { ...citation, characterStart: null, characterEnd: null };
  return { ...citation, characterStart: resolution.start, characterEnd: resolution.end };
}
