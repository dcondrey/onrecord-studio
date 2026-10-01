import { describe, expect, it } from 'vitest';
import { formatMatchCount, SEARCH_RESULT_LIMIT } from '../utils/searchResults';

describe('formatMatchCount', () => {
  it('reports an exact total below the cap and a lower bound at it', () => {
    expect(formatMatchCount(SEARCH_RESULT_LIMIT - 1)).toBe('499');
    expect(formatMatchCount(SEARCH_RESULT_LIMIT)).toBe('500+');
  });
});
