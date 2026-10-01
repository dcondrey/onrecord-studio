import { describe, expect, it } from 'vitest';
import { formatMatchCount, matchBasis, SEARCH_RESULT_LIMIT, searchQueryId, searchRecordText } from '../utils/searchResults';

describe('formatMatchCount', () => {
  it('reports an exact total below the cap and a lower bound at it', () => {
    expect(formatMatchCount(SEARCH_RESULT_LIMIT - 1)).toBe('499');
    expect(formatMatchCount(SEARCH_RESULT_LIMIT)).toBe('500+');
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
  it('says when the result list was limited and never claims completeness', () => {
    const base = { ranAt: '2026-09-30 20:00 UTC', query: 'sweep camp', filters: [{ label: 'Status', value: 'open' }], sourceCount: 3 };
    const limited = searchRecordText({ ...base, resultCount: SEARCH_RESULT_LIMIT });
    expect(limited).toContain('500+ (limited to the first 500; more may exist)');
    expect(limited).toContain('Status: open');
    expect(searchRecordText({ ...base, resultCount: 12 })).toContain('Records returned: 12\n');
    expect(limited).toContain('request metadata can still change');
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
    expect(searchRecordText({ ...base, corpus: null })).toContain('Corpus state: could not be read.');
  });
});
