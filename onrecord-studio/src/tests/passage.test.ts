import { describe, expect, it } from 'vitest';
import { clipPassage } from '../utils/passage';

// A cited passage is printed as evidence: it never ends on a fragment of a word, and any cut is
// marked, so the reader never takes a clipped span for the whole of what the record says.
describe('clipPassage', () => {
  it('drops a dangling sentence start and marks the cut, and leaves a whole passage alone', () => {
    expect(clipPassage('Notices were not located. T')).toBe('Notices were not located. …');
    expect(clipPassage('Payments total $1,486,200.00 for FY2025.')).toBe('Payments total $1,486,200.00 for FY2025.');
    expect(clipPassage('one two three four', 12)).toBe('one two…');
  });
});
