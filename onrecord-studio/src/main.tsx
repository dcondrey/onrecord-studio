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

// WebKit's default menu offers browser actions (Reload, Back) that mean nothing in a desktop app.
// It stays where it does real work: editing fields and copying a selection. Development builds
// keep it for Inspect Element.
if (!import.meta.env.DEV) {
  document.addEventListener('contextmenu', (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const editable = target?.closest('input, textarea, [contenteditable="true"]');
    const selection = window.getSelection();
    if (!editable && (!selection || selection.isCollapsed)) event.preventDefault();
  });
}

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Application root element is missing');
}

createRoot(rootElement).render(<App />);
