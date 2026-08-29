import { describe, expect, it } from 'vitest';
import {
  readContrastPreference,
  readMotionPreference,
  resolveContrast,
  resolveMotion,
} from '../utils/accessibility';

describe('motion and contrast preferences', () => {
  it('defaults to the system setting and rejects anything else persisted', () => {
    expect(readMotionPreference({ getItem: () => 'reduce' })).toBe('reduce');
    expect(readMotionPreference({ getItem: () => 'full' })).toBe('full');
    expect(readMotionPreference({ getItem: () => 'off' })).toBe('system');
    expect(readMotionPreference({ getItem: () => null })).toBe('system');
    expect(readContrastPreference({ getItem: () => 'more' })).toBe('more');
    expect(readContrastPreference({ getItem: () => 'standard' })).toBe('standard');
    expect(readContrastPreference({ getItem: () => 'high' })).toBe('system');
    expect(readContrastPreference({ getItem: () => null })).toBe('system');
  });

  it('follows the operating system only while the preference is system', () => {
    expect(resolveMotion('system', true)).toBe('reduce');
    expect(resolveMotion('system', false)).toBe('full');
    expect(resolveContrast('system', true)).toBe('more');
    expect(resolveContrast('system', false)).toBe('standard');
  });

  // The override has to work in both directions. A reader whose OS globally asks for reduced
  // motion or higher contrast, but who wants this workspace to render otherwise, cannot say so
  // through the media query at all; a one-way toggle would leave them with no control.
  it('lets an explicit choice contradict the operating system in either direction', () => {
    expect(resolveMotion('full', true)).toBe('full');
    expect(resolveMotion('reduce', false)).toBe('reduce');
    expect(resolveContrast('standard', true)).toBe('standard');
    expect(resolveContrast('more', false)).toBe('more');
  });
});
