import { describe, expect, it } from 'vitest';
import { readThemePreference, resolveTheme } from '../utils/theme';

describe('theme resolution', () => {
  it('accepts only persisted theme preferences', () => {
    expect(readThemePreference({ getItem: () => 'auto' })).toBe('auto');
    expect(readThemePreference({ getItem: () => 'dark' })).toBe('dark');
    expect(readThemePreference({ getItem: () => 'invalid' })).toBe('auto');
    expect(readThemePreference({ getItem: () => 'light' })).toBe('light');
    expect(readThemePreference({ getItem: () => null })).toBe('auto');
  });

  it('resolves auto from the current system preference', () => {
    expect(resolveTheme('auto', true)).toBe('dark');
    expect(resolveTheme('auto', false)).toBe('light');
  });

  it('does not let the system override an explicit preference', () => {
    expect(resolveTheme('light', true)).toBe('light');
    expect(resolveTheme('dark', false)).toBe('dark');
  });
});
