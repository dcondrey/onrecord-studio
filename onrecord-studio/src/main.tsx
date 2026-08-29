import { createRoot } from 'react-dom/client';
import App from './App';
import './index.css';
import { readThemePreference, resolveTheme } from './utils/theme';
import {
  readContrastPreference,
  readMotionPreference,
  resolveContrast,
  resolveMotion,
  systemAccessibilityPreferences,
} from './utils/accessibility';

const initialTheme = resolveTheme(
  readThemePreference(),
  window.matchMedia('(prefers-color-scheme: dark)').matches,
);
document.documentElement.dataset.theme = initialTheme;

// Stamped before first paint alongside the theme. Applying them from an effect instead would let
// the opening animation run once for a reader who asked for reduced motion.
const system = systemAccessibilityPreferences();
document.documentElement.dataset.motion = resolveMotion(
  readMotionPreference(),
  system.prefersReducedMotion,
);
document.documentElement.dataset.contrast = resolveContrast(
  readContrastPreference(),
  system.prefersMoreContrast,
);

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Application root element is missing');
}

createRoot(rootElement).render(<App />);
