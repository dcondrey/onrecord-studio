export const THEME_STORAGE_KEY = 'onrecord-theme';

export type ThemePreference = 'light' | 'dark' | 'auto';
export type ResolvedTheme = Exclude<ThemePreference, 'auto'>;

export function readThemePreference(storage?: Pick<Storage, 'getItem'>): ThemePreference {
  const availableStorage = storage
    ?? (typeof window === 'undefined' ? undefined : window.localStorage);
  const savedTheme = availableStorage?.getItem(THEME_STORAGE_KEY);
  return savedTheme === 'dark' || savedTheme === 'auto' ? savedTheme : 'light';
}

export function resolveTheme(
  preference: ThemePreference,
  prefersDark: boolean,
): ResolvedTheme {
  return preference === 'auto' ? (prefersDark ? 'dark' : 'light') : preference;
}
