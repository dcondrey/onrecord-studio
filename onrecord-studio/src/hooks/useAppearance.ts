import { useEffect, useState } from 'react';
import { setTheme as setNativeTheme } from '@tauri-apps/api/app';
import { isTauri } from '@tauri-apps/api/core';
import { useAppStore } from '../store/useAppStore';
import { resolveTheme, THEME_STORAGE_KEY } from '../utils/theme';
import {
  CONTRAST_STORAGE_KEY,
  MOTION_STORAGE_KEY,
  resolveContrast,
  resolveMotion,
  systemAccessibilityPreferences,
} from '../utils/accessibility';

/**
 * Resolves theme, contrast and motion against the OS and writes them to `<html>`. It runs above the
 * licence gate, so sign-in and expiry screens follow the same appearance as the workspace.
 */
export function useAppearance() {
  const theme = useAppStore((state) => state.theme);
  const motion = useAppStore((state) => state.motion);
  const contrast = useAppStore((state) => state.contrast);
  const [systemTheme, setSystemTheme] = useState<'light' | 'dark'>(() =>
    window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
  );
  const [systemAccessibility, setSystemAccessibility] = useState(systemAccessibilityPreferences);

  const resolvedTheme = resolveTheme(theme, systemTheme === 'dark');
  const resolvedMotion = resolveMotion(motion, systemAccessibility.prefersReducedMotion);
  const resolvedContrast = resolveContrast(contrast, systemAccessibility.prefersMoreContrast);

  useEffect(() => {
    const appearance = window.matchMedia('(prefers-color-scheme: dark)');
    const updateSystemTheme = () => setSystemTheme(appearance.matches ? 'dark' : 'light');
    updateSystemTheme();
    appearance.addEventListener('change', updateSystemTheme);
    return () => appearance.removeEventListener('change', updateSystemTheme);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = resolvedTheme;
    document.documentElement.dataset.themePreference = theme;
    window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    if (isTauri()) void setNativeTheme(theme === 'auto' ? null : theme);
  }, [resolvedTheme, theme]);

  useEffect(() => {
    const motionQuery = window.matchMedia('(prefers-reduced-motion: reduce)');
    const contrastQuery = window.matchMedia('(prefers-contrast: more)');
    const update = () =>
      setSystemAccessibility({
        prefersReducedMotion: motionQuery.matches,
        prefersMoreContrast: contrastQuery.matches,
      });
    update();
    motionQuery.addEventListener('change', update);
    contrastQuery.addEventListener('change', update);
    return () => {
      motionQuery.removeEventListener('change', update);
      contrastQuery.removeEventListener('change', update);
    };
  }, []);

  useEffect(() => {
    document.documentElement.dataset.motion = resolvedMotion;
    document.documentElement.dataset.motionPreference = motion;
    document.documentElement.dataset.contrast = resolvedContrast;
    document.documentElement.dataset.contrastPreference = contrast;
    window.localStorage.setItem(MOTION_STORAGE_KEY, motion);
    window.localStorage.setItem(CONTRAST_STORAGE_KEY, contrast);
  }, [contrast, motion, resolvedContrast, resolvedMotion]);

  return resolvedTheme;
}
