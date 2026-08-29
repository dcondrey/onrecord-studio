/**
 * Motion and contrast preferences, resolved the same way the theme is.
 *
 * The OS query is the default, not the whole answer. A reader who needs reduced motion or higher
 * contrast in this application specifically, on a shared machine, or on an OS whose global setting
 * they do not want to change, has no way to say so through `prefers-reduced-motion` alone. So each
 * preference carries an explicit override in both directions and `system` defers to the query.
 */

export const MOTION_STORAGE_KEY = 'onrecord-motion';
export const CONTRAST_STORAGE_KEY = 'onrecord-contrast';

export type MotionPreference = 'system' | 'reduce' | 'full';
export type ResolvedMotion = 'reduce' | 'full';
export type ContrastPreference = 'system' | 'more' | 'standard';
export type ResolvedContrast = 'more' | 'standard';

type ReadableStorage = Pick<Storage, 'getItem'>;

function read(key: string, allowed: readonly string[], storage?: ReadableStorage) {
  const availableStorage =
    storage ?? (typeof window === 'undefined' ? undefined : window.localStorage);
  const saved = availableStorage?.getItem(key);
  return saved !== null && saved !== undefined && allowed.includes(saved) ? saved : 'system';
}

export function readMotionPreference(storage?: ReadableStorage): MotionPreference {
  return read(MOTION_STORAGE_KEY, ['reduce', 'full'], storage) as MotionPreference;
}

export function readContrastPreference(storage?: ReadableStorage): ContrastPreference {
  return read(CONTRAST_STORAGE_KEY, ['more', 'standard'], storage) as ContrastPreference;
}

export function resolveMotion(
  preference: MotionPreference,
  systemPrefersReduced: boolean,
): ResolvedMotion {
  if (preference === 'system') return systemPrefersReduced ? 'reduce' : 'full';
  return preference;
}

export function resolveContrast(
  preference: ContrastPreference,
  systemPrefersMore: boolean,
): ResolvedContrast {
  if (preference === 'system') return systemPrefersMore ? 'more' : 'standard';
  return preference;
}

/**
 * Reads both queries defensively.
 *
 * `prefers-contrast` is not implemented in every engine this app is rendered by, and a
 * `matchMedia` call for an unknown feature returns `matches: false` rather than throwing. Treating
 * that as "the reader does not want more contrast" is correct: it is the absence of a signal, and
 * the explicit override exists precisely so it is not the only way to ask.
 */
export function systemAccessibilityPreferences() {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return { prefersReducedMotion: false, prefersMoreContrast: false };
  }
  return {
    prefersReducedMotion: window.matchMedia('(prefers-reduced-motion: reduce)').matches,
    prefersMoreContrast: window.matchMedia('(prefers-contrast: more)').matches,
  };
}
