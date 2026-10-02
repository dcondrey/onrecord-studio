import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// Every UI colour is a semantic role defined once per mode in index.css. These checks are the
// basis of the App Store "Sufficient Contrast" label: each text role must stay readable on each
// surface in every mode, and a component may not bypass the roles with a raw palette utility.
const css = readFileSync(join(__dirname, '..', 'index.css'), 'utf8');

function block(selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  if (start < 0) throw new Error(`missing ${selector}`);
  const body = css.slice(start, css.indexOf('}', start));
  return Object.fromEntries([...body.matchAll(/--([\w-]+):\s*(\d+ \d+ \d+);/g)].map(([, name, value]) => [name, value]));
}

const light = block(':root');
const dark = { ...light, ...block(':root[data-theme="dark"]') };
const more = block(':root[data-contrast="more"]');
const MODES = {
  light,
  dark,
  'light, increased contrast': { ...light, ...more },
  'dark, increased contrast': { ...dark, ...more, ...block(':root[data-contrast="more"][data-theme="dark"]') },
};
const TEXT = ['fg', 'fg-secondary', 'muted', 'accent', 'highlight', 'success', 'warning', 'danger'];
const SURFACES = ['canvas', 'surface', 'raised'];

function luminance(channels: string) {
  const [r, g, b] = channels.split(' ').map((value) => {
    const v = Number(value) / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(a: string, b: string) {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return entry.isFile() && /\.tsx?$/.test(path) ? [path] : [];
  });
}

describe('colour roles', () => {
  it('keep every text role readable on every surface in every mode', () => {
    const failures: string[] = [];
    for (const [mode, tokens] of Object.entries(MODES)) {
      const increased = mode.includes('increased');
      for (const text of TEXT) {
        const minimum = increased && ['fg', 'fg-secondary', 'muted'].includes(text) ? 7 : 4.5;
        for (const surface of SURFACES) {
          const ratio = contrast(tokens[text], tokens[surface]);
          if (ratio < minimum) failures.push(`${mode}: ${text} on ${surface} ${ratio.toFixed(2)}:1 < ${minimum}`);
        }
      }
      for (const surface of SURFACES) {
        const ratio = contrast(tokens.control, tokens[surface]);
        if (ratio < 3) failures.push(`${mode}: control border on ${surface} ${ratio.toFixed(2)}:1 < 3`);
      }
    }
    expect(failures).toEqual([]);
  });

  it('are the only colours components use', () => {
    const raw = /\b(?:text|bg|border|ring|ring-offset|fill|stroke|from|via|to|divide|outline|placeholder|decoration|shadow|caret)-(?:slate|gray|zinc|neutral|stone|red|orange|amber|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3}\b/;
    const offenders = sourceFiles(join(__dirname, '..', 'components'))
      .flatMap((file) => readFileSync(file, 'utf8').split('\n').map((line, index) => [file, index, line] as const))
      .filter(([, , line]) => raw.test(line))
      .map(([file, index, line]) => `${file.split('/src/')[1]}:${index + 1} ${line.match(raw)?.[0]}`);
    expect(offenders).toEqual([]);
  });
});
