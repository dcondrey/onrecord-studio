import { createRoot } from 'react-dom/client';
import App from './App';
import './index.css';

// TODO(theme): Centralize initial theme resolution with the Zustand store so
// the first paint and hydrated application state cannot diverge.
const savedTheme = window.localStorage.getItem('onrecord-theme');
const initialTheme = savedTheme === 'dark'
  || (savedTheme === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches)
  ? 'dark'
  : 'light';
document.documentElement.dataset.theme = initialTheme;

const rootElement = document.getElementById('root');

if (!rootElement) {
  throw new Error('Application root element is missing');
}

createRoot(rootElement).render(<App />);
