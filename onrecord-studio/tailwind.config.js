/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      // The app's two caption sizes. String values set font-size only, so line height is
      // inherited from the surrounding text unless a leading-* class says otherwise.
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
        'highlight': 'rgb(var(--highlight) / <alpha-value>)',
        'success': 'rgb(var(--success) / <alpha-value>)',
        'warning': 'rgb(var(--warning) / <alpha-value>)',
        'danger': 'rgb(var(--danger) / <alpha-value>)',
      },
      fontSize: {
        caption: '0.75rem',
        label: '0.8125rem',
      },
      fontFamily: {
        sans: ['-apple-system', 'BlinkMacSystemFont', '"Segoe UI"', 'Roboto', 'Helvetica', 'Arial', 'sans-serif'],
        mono: ['SFMono-Regular', 'Consolas', '"Liberation Mono"', 'Menlo', 'monospace'],
      },
    },
  },
  plugins: [],
}
