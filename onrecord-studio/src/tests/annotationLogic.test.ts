import { describe, expect, it } from 'vitest';
import { agreementReport, annotationCitation, findPassageInText } from '../components/documents/annotationLogic';
import type { ResolvedAnnotation } from '../types';

const mark = (id: string, author: string | null, start: number, length: number, tag: string | null, patch: Partial<ResolvedAnnotation> = {}): ResolvedAnnotation => ({
  id,
  portalKey: 'sd',
  documentId: 7,
  kind: 'ISSUE',
  characterOffset: start,
  quote: 'q'.repeat(length),
  prefix: '',
  suffix: '',
  textSha256: 'a'.repeat(64),
  contextUnanchored: false,
  note: '',
  tag,
  author,
  status: 'OPEN',
  findingId: null,
  importedFromKey: null,
  createdAt: '2026-01-01T00:00:00Z',
  anchor: { state: 'ANCHORED', sourceOffset: start, sourceLength: length, reason: null },
  ...patch,
});

describe('agreementReport', () => {
  it('pairs only different authors over overlapping ranges, and says what it left out', () => {
    const report = agreementReport([
      mark('a', 'Maria', 0, 10, 'DELAY'),
      mark('b', 'Jon', 5, 10, 'DELAY'),
      mark('c', 'Jon', 8, 4, 'FEE'),
      mark('touching', 'Jon', 10, 5, 'DELAY'),
      mark('same-author', 'Maria', 2, 3, 'FEE'),
      mark('orphan', 'Jon', 0, 10, 'DELAY', { anchor: { state: 'ORPHANED', sourceOffset: null, sourceLength: null, reason: 'gone' } }),
      mark('nobody', null, 0, 10, 'DELAY'),
    ]);
    const verdicts = report.pairs.map((pair) => `${pair.first.id}-${pair.second.id}:${pair.verdict}`);
    // a [0,10) vs b [5,15) overlap and agree; a vs c [8,12) disagree; a vs touching [10,15) do not overlap.
    expect(verdicts).toEqual(['a-b:AGREE', 'a-c:DISAGREE']);
    expect(report.left).toEqual({ orphaned: 1, unattributed: 1, noPassage: 0 });
  });

  it('does not call a coded and an uncoded mark agreement', () => {
    const report = agreementReport([mark('a', 'Maria', 0, 5, 'DELAY'), mark('b', 'Jon', 1, 2, null), mark('c', 'Lee', 1, 2, null)]);
    expect(report.pairs.map((pair) => pair.verdict)).toEqual(['DISAGREE', 'DISAGREE', 'UNCODED']);
  });
});

describe('annotationCitation', () => {
  it('names the passage, the text it was found in, and the limit of the offset', () => {
    const text = annotationCitation(mark('a', 'Maria', 120, 6, 'DELAY', { quote: 'amend!' }), {
      fileName: 'contract.pdf',
      currentTextSha256: 'b'.repeat(64),
      appVersion: '1.2.3',
    });
    expect(text).toContain('Portal: sd');
    expect(text).toContain('Document id: 7 (contract.pdf)');
    expect(text).toContain('Character offset: 120, 6 characters');
    expect(text).toContain('Quote: "amend!"');
    expect(text).toContain(`Document text SHA-256: ${'b'.repeat(64)}`);
    expect(text).toContain('version 1.2.3');
    expect(text).toContain('extracted copy');
  });

  it('cites an orphan as recorded rather than at a position it no longer has', () => {
    const text = annotationCitation(
      mark('a', 'Maria', 120, 6, 'DELAY', { anchor: { state: 'ORPHANED', sourceOffset: null, sourceLength: null, reason: 'gone' }, textSha256: null }),
      { currentTextSha256: 'b'.repeat(64), appVersion: '1.0.0' },
    );
    expect(text).toContain('not found in the current text');
    expect(text).toContain('Document text SHA-256: not recorded');
  });
});

describe('findPassageInText', () => {
  it('chooses the repeated passage nearest where it was recorded and never one that is absent', () => {
    const text = 'fee. ... fee. ... fee.';
    const passage = { quote: 'fee', prefix: '', suffix: '' };
    expect(findPassageInText(text, passage, 0)).toBe(0);
    expect(findPassageInText(text, passage, 100)).toBe(18);
    expect(findPassageInText(text, { quote: 'absent', prefix: '', suffix: '' }, 0)).toBeNull();
  });
});
