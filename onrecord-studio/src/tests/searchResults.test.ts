import { describe, expect, it } from 'vitest';
import { formatMatchCount, matchBasis, SEARCH_RESULT_LIMIT } from '../utils/searchResults';

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
