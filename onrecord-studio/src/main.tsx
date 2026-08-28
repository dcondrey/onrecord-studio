import { createRoot } from 'react-dom/client';
import App from './App';
import './index.css';
import { readThemePreference, resolveTheme } from './utils/theme';

const initialTheme = resolveTheme(
  readThemePreference(),
  window.matchMedia('(prefers-color-scheme: dark)').matches,
);
document.documentElement.dataset.theme = initialTheme;

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Application root element is missing');
}

createRoot(rootElement).render(<App />);
