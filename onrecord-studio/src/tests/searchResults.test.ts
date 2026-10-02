import { describe, expect, it } from 'vitest';
import { formatMatchCount, matchBasis, searchQueryId, searchRecordText } from '../utils/searchResults';

describe('formatMatchCount', () => {
  it('states the exact total at any size and never a lower bound', () => {
    expect(formatMatchCount(499)).toBe('499');
    expect(formatMatchCount(500)).toBe('500');
    expect(formatMatchCount(12431)).toBe('12,431');
  });
});

describe('matchBasis', () => {
  const result = { title: 'Budget amendment 12', descriptionPreview: 'Requests contract records' };
  it('says where a term is visible and never calls a hidden match absent', () => {
    expect(matchBasis(result, 'amendment')).toBe('title');
    expect(matchBasis(result, 'contract RECORDS')).toBe('description');
    expect(matchBasis(result, 'vendor')).toBe('unshown');
    expect(matchBasis(result, '   ')).toBeNull();
  });
});

describe('searchRecordText', () => {
  it('records the exact match count, not the number of rows that were loaded', () => {
    const base = { ranAt: '2026-09-30 20:00 UTC', query: 'sweep camp', filters: [{ label: 'Status', value: 'open' }], sourceCount: 3 };
    const text = searchRecordText({ ...base, resultCount: 12431 });
    expect(text).toContain('Matches: 12,431 (exact count when the search ran)');
    expect(text).not.toMatch(/limited|more may exist|\+/);
    expect(text).toContain('Status: open');
    expect(text).toContain('request metadata can still change');
  });
});

describe('searchQueryId', () => {
  it('names the same query the same way regardless of spacing or filter order, and differs when the query does', async () => {
    const a = await searchQueryId('sweep   camp', [{ label: 'Status', value: 'open' }, { label: 'Source', value: 'sd' }]);
    const b = await searchQueryId(' sweep camp ', [{ label: 'Source', value: 'sd' }, { label: 'Status', value: 'open' }]);
    const c = await searchQueryId('sweep camp', [{ label: 'Status', value: 'closed' }]);
    expect(a).toMatch(/^[0-9a-f]{12}$/);
    expect(a).toBe(b);
    expect(c).not.toBe(a);
  });

  it('puts the query ID and corpus state in the record, and says when the corpus could not be read', () => {
    const base = { ranAt: 'x', query: 'q', filters: [], sourceCount: 1, resultCount: 3 };
    const full = searchRecordText({ ...base, queryId: 'abc123abc123', corpus: { digest: 'f'.repeat(64), documentCount: 1200 } });
    expect(full).toContain('Query ID: abc123abc123');
    expect(full).toContain('sha256:ffffffffffffffff (1,200 documents)');
    expect(searchRecordText({ ...base, corpus: null })).toContain('Corpus state when copied: could not be read.');
  });
});
