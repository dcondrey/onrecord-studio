/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        'fg': 'rgb(var(--fg) / <alpha-value>)',
        'fg-secondary': 'rgb(var(--fg-secondary) / <alpha-value>)',
        'muted': 'rgb(var(--muted) / <alpha-value>)',
        'canvas': 'rgb(var(--canvas) / <alpha-value>)',
        'surface': 'rgb(var(--surface) / <alpha-value>)',
        'raised': 'rgb(var(--raised) / <alpha-value>)',
        'line': 'rgb(var(--line) / <alpha-value>)',
        'line-strong': 'rgb(var(--line-strong) / <alpha-value>)',
        'control': 'rgb(var(--control) / <alpha-value>)',
        'accent': 'rgb(var(--accent) / <alpha-value>)',
        'on-accent': 'rgb(var(--on-accent) / <alpha-value>)',
        'highlight': 'rgb(var(--highlight) / <alpha-value>)',
        'success': 'rgb(var(--success) / <alpha-value>)',
        'warning': 'rgb(var(--warning) / <alpha-value>)',
        'danger': 'rgb(var(--danger) / <alpha-value>)',
        'focus': 'rgb(var(--focus) / <alpha-value>)',
        'scrim': 'var(--scrim)',
        'hatch': 'var(--hatch)',
        'panel': 'var(--panel-surface)',
        'chrome': 'var(--header-surface)',
      },
      // The type scale. Each token carries its own line height (and weight/tracking where the role
      // fixes them), so a size never inherits a leading that is off the 4px grid. Keep these keys in
      // sync with the `font-size` class group in src/utils/cn.ts, or cn() will read them as colours.
      // `label` is a transitional alias of `body` until call sites are renamed.
      fontSize: {
        display: ['1.75rem', { lineHeight: '2.125rem', letterSpacing: '-0.022em', fontWeight: '600' }],
        title: ['1.25rem', { lineHeight: '1.75rem', letterSpacing: '-0.012em', fontWeight: '600' }],
        heading: ['0.9375rem', { lineHeight: '1.25rem', letterSpacing: '-0.006em', fontWeight: '600' }],
        body: ['0.8125rem', { lineHeight: '1.25rem' }],
        label: ['0.8125rem', { lineHeight: '1.25rem' }],
        control: ['0.8125rem', { lineHeight: '1rem', fontWeight: '500' }],
        caption: ['0.75rem', { lineHeight: '1rem' }],
        overline: ['0.6875rem', { lineHeight: '1rem', letterSpacing: '0.06em', fontWeight: '600' }],
        metric: ['1.5rem', { lineHeight: '1.75rem', letterSpacing: '-0.02em', fontWeight: '600' }],
        code: ['0.75rem', { lineHeight: '1rem' }],
      },
      fontFamily: {
        sans: ['-apple-system', 'BlinkMacSystemFont', '"SF Pro Text"', '"Segoe UI"', 'Roboto', '"Helvetica Neue"', 'Arial', 'sans-serif'],
        mono: ['ui-monospace', '"SF Mono"', 'SFMono-Regular', 'Menlo', 'Consolas', 'monospace'],
      },
      // Only 400/500/600 exist in the system; `font-bold` renders as semibold.
      fontWeight: {
        bold: '600',
      },
      boxShadow: {
        panel: 'var(--shadow-panel)',
        popover: 'var(--shadow-popover)',
        dialog: 'var(--shadow-dialog)',
      },
      maxWidth: {
        content: '1600px',
        prose: '45rem',
      },
      zIndex: {
        sticky: '10',
        header: '30',
        overlay: '40',
        popover: '50',
        sheet: '60',
        dialog: '70',
        palette: '80',
        toast: '90',
        gate: '100',
      },
    },
  },
  plugins: [],
}
