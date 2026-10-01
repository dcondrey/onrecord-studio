import { describe, expect, it } from 'vitest';
import { formatMatchCount, matchBasis, SEARCH_RESULT_LIMIT, searchRecordText } from '../utils/searchResults';

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
    expect(limited).toContain('can change as sources synchronize');
  });
});
