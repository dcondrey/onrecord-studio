import { describe, expect, it } from 'vitest';
import { applyMerge, mergeCandidates } from '../utils/caseworkMerge';
import type { CaseworkCase, CaseworkCitation } from '../types/casework';

const cite = (id: string) => ({ id, label: id, quote: '' }) as unknown as CaseworkCitation;
const question = (id: string, text: string, citationIds: string[] = []) => ({ id, question: text, hypothesis: '', nextTest: '', disposition: '', owner: '', citationIds, counterCitationIds: [] });
const make = (overrides: Partial<CaseworkCase>): CaseworkCase =>
  ({ id: 'c', title: 'T', owner: '', disposition: '', notes: '', questions: [], requests: [], decisions: [], commitments: [], evidence: [], ...overrides }) as unknown as CaseworkCase;

describe('case merge', () => {
  const local = make({ notes: 'mine', questions: [question('q1', 'old'), question('q2', 'only mine')], evidence: [cite('e1')] });
  const incoming = make({ notes: 'theirs', questions: [question('q1', 'new'), question('q3', 'theirs', ['e9'])], evidence: [cite('e1'), cite('e9')] });

  it('lists additions, changes and removals, and applies only what was chosen', () => {
    const found = mergeCandidates(local, incoming);
    expect(found.map((c) => `${c.key}:${c.change}`).sort()).toEqual(
      ['evidence:e9:added', 'field:notes:changed', 'questions:q1:changed', 'questions:q2:removed', 'questions:q3:added'].sort(),
    );
    const merged = applyMerge(local, incoming, new Set(['questions:q1', 'questions:q3']));
    expect(merged.notes).toBe('mine');
    expect(merged.questions.map((q) => [q.id, q.question])).toEqual([['q1', 'new'], ['q2', 'only mine'], ['q3', 'theirs']]);
    expect(applyMerge(local, incoming, new Set())).toEqual(local);
  });

  it('keeps a removed citation that a surviving item still cites', () => {
    const cited = make({ questions: [question('q1', 'cites', ['e1'])], evidence: [cite('e1')] });
    const without = make({});
    expect(mergeCandidates(cited, without).map((c) => c.key)).toEqual(['questions:q1', 'evidence:e1']);
    expect(applyMerge(cited, without, new Set(['evidence:e1'])).evidence.map((e) => e.id)).toEqual(['e1']);
    expect(applyMerge(cited, without, new Set(['evidence:e1', 'questions:q1'])).evidence).toEqual([]);
  });

  it('brings across the evidence an adopted item cites, and honours a chosen removal', () => {
    const merged = applyMerge(local, incoming, new Set(['questions:q3', 'questions:q2']));
    expect(merged.evidence.map((e) => e.id)).toEqual(['e1', 'e9']);
    expect(merged.questions.map((q) => q.id)).toEqual(['q1', 'q3']);
  });
});
