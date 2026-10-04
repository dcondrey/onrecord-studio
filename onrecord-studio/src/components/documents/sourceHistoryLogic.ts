import type { DocumentAcquisition, DocumentSourceHistory, SourceCheck } from '../../types';

export type SourceHistoryEntry =
  | { kind: 'check'; at: string; check: SourceCheck }
  | { kind: 'acquisition'; at: string; acquisition: DocumentAcquisition };

function instant(stamp: string): number {
  const parsed = Date.parse(stamp);
  return Number.isNaN(parsed) ? Number.NEGATIVE_INFINITY : parsed;
}

/**
 * Checks and acquisitions as one newest-first list. A fetch a check made is the check's own
 * evidence and is shown with it, not again as a separate, unexplained fetch.
 */
export function sourceHistoryEntries(history: DocumentSourceHistory): SourceHistoryEntry[] {
  const entries: SourceHistoryEntry[] = [
    ...history.checks.map((check) => ({ kind: 'check' as const, at: check.checkedAt, check })),
    ...history.acquisitions
      .filter((acquisition) => acquisition.sourceCheckId === null)
      .map((acquisition) => ({ kind: 'acquisition' as const, at: acquisition.retrievedAt, acquisition })),
  ];
  return entries.sort((left, right) => instant(right.at) - instant(left.at));
}

export function shortDigest(sha256: string): string {
  return sha256.slice(0, 12);
}

/** What one check observed, in words that claim nothing beyond it. */
export function describeCheck(check: SourceCheck): { title: string; detail: string } {
  switch (check.outcome) {
    case 'SAME_BYTES':
      return {
        title: 'Same bytes',
        detail: `The address served a file whose SHA-256 matches the collected copy (${shortDigest(check.collectedSha256)}…).`,
      };
    case 'DIFFERENT_BYTES':
      return {
        title: 'Different bytes',
        detail: `The address served a file whose SHA-256 (${shortDigest(check.observedSha256 ?? '')}…) differs from the collected copy (${shortDigest(check.collectedSha256)}…). This compares digests only: a replaced file and an error page served in its place both differ. ${check.observedCopyRetained ? 'A copy of what was served is kept.' : 'What was served was not kept.'}`,
      };
    case 'GONE':
      return {
        title: `Not found (HTTP ${check.httpStatus ?? ''})`.trim(),
        detail: 'The address answered that nothing is there now. It does not say when or why the record was removed, or whether it moved.',
      };
    case 'UNREACHABLE':
      return {
        title: 'No answer',
        detail: `This check got no answer about the record${check.detail ? ` (${check.detail})` : ''}. It says nothing about whether the record is still there.`,
      };
  }
}

export function describeAcquisition(acquisition: DocumentAcquisition): { title: string; detail: string } {
  const changed = acquisition.previousRawFileSha256 !== null && acquisition.previousRawFileSha256.toLowerCase() !== acquisition.rawFileSha256.toLowerCase();
  const first = acquisition.previousRawFileSha256 === null;
  return {
    title: acquisition.syncRunId ? 'Collected' : 'Fetched',
    detail: `HTTP ${acquisition.httpStatus}, ${acquisition.fileSize.toLocaleString()} bytes, SHA-256 ${shortDigest(acquisition.rawFileSha256)}…${first ? '. The first recorded fetch of this document.' : changed ? `. Differs from the copy held before this fetch (${shortDigest(acquisition.previousRawFileSha256 ?? '')}…).` : '. Matches the copy held before this fetch.'}`,
  };
}

const TEXT_PREVIEW_CHARACTERS = 200_000;

/** How the kept bytes can be shown here: as PDF pages, as text, or not at all. */
export function keptCopyKind(bytes: Uint8Array): 'pdf' | 'text' | 'binary' {
  const head = bytes.subarray(0, 5);
  if (head.length === 5 && String.fromCharCode(...head) === '%PDF-') return 'pdf';
  return bytes.subarray(0, 4096).includes(0) ? 'binary' : 'text';
}

export function keptCopyText(bytes: Uint8Array): { text: string; truncated: boolean } {
  // Four UTF-8 bytes at most per character, so this window always holds the preview.
  const window = bytes.subarray(0, TEXT_PREVIEW_CHARACTERS * 4);
  const text = new TextDecoder('utf-8', { fatal: false }).decode(window);
  if (text.length <= TEXT_PREVIEW_CHARACTERS) return { text, truncated: window.length < bytes.length };
  // Never end on half of a surrogate pair.
  const end = /[\uD800-\uDBFF]/.test(text[TEXT_PREVIEW_CHARACTERS - 1] ?? '') ? TEXT_PREVIEW_CHARACTERS - 1 : TEXT_PREVIEW_CHARACTERS;
  return { text: text.slice(0, end), truncated: true };
}
