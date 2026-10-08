import { describe, expect, it } from 'vitest';
import { cn } from '../utils/cn';

describe('cn', () => {
  it('treats the custom type scale as font sizes, not colours', () => {
    expect(cn('text-caption text-danger')).toBe('text-caption text-danger');
    expect(cn('text-caption', 'text-body')).toBe('text-body');
    expect(cn('text-muted', 'text-fg')).toBe('text-fg');
  });
});
