import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';
import { cn } from '../utils/cn';

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
const TEXT = ['fg', 'fg-secondary', 'muted', 'accent', 'lead', 'highlight', 'success', 'warning', 'danger'];
const SURFACES = ['bg', 'panel', 'canvas', 'surface', 'raised'];

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

/** `top` at `alpha` over `bottom`, composited per 8-bit channel as the browser paints it. */
function over(top: string, alpha: number, bottom: string) {
  const [t, b] = [top, bottom].map((channels) => channels.split(' ').map(Number));
  return t.map((value, index) => Math.round(value * alpha + b[index] * (1 - alpha))).join(' ');
}

/** A declaration this file's channel parser skips: `--name: <number>` or `--name: rgb(r g b / a)`. */
function declared(selector: string, name: string) {
  const start = css.indexOf(`${selector} {`);
  const match = css.slice(start, css.indexOf('}', start)).match(new RegExp(`--${name}:\\s*(?:rgb\\((\\d+ \\d+ \\d+) / )?([\\d.]+)\\)?;`));
  if (start < 0 || !match) throw new Error(`missing --${name} in ${selector}`);
  return { channels: match[1], alpha: Number(match[2]) };
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
      for (const [text, fill] of [['on-accent', 'accent'], ['on-lead', 'lead-mark'], ['on-danger', 'danger']]) {
        const ratio = contrast(tokens[text], tokens[fill]);
        if (ratio < 4.5) failures.push(`${mode}: ${text} on ${fill} ${ratio.toFixed(2)}:1 < 4.5`);
      }
      for (const mark of ['lead-mark', 'success-mark', 'danger-mark', 'warning-mark']) {
        for (const surface of ['bg', 'panel']) {
          const ratio = contrast(tokens[mark], tokens[surface]);
          if (ratio < 3) failures.push(`${mode}: ${mark} on ${surface} ${ratio.toFixed(2)}:1 < 3`);
        }
      }
      for (const surface of SURFACES) {
        const ratio = contrast(tokens.control, tokens[surface]);
        if (ratio < 3) failures.push(`${mode}: control border on ${surface} ${ratio.toFixed(2)}:1 < 3`);
      }
    }
    expect(failures).toEqual([]);
  });

  // A tone's text on its own /10 tint is the badge and selected-row treatment. It sits on every
  // opaque surface, on glass over those surfaces (popovers and menus), and on glass over the scrim
  // over them (dialogs, the darkest backdrop glass gets). The sidebar's count badge stacks a second
  // accent/10 on the selected row. Increased contrast makes glass opaque, so it has no glass rows.
  it('keep tone text readable on its own tint, on glass and stacked in the sidebar', () => {
    const failures: string[] = [];
    for (const [mode, tokens] of Object.entries(MODES)) {
      const selector = mode.startsWith('dark') ? ':root[data-theme="dark"]' : ':root';
      const glass = declared(selector, 'glass-alpha').alpha;
      const scrim = declared(selector, 'scrim');
      const backdrops: [string, string][] = SURFACES.flatMap((surface) => {
        const opaque: [string, string] = [surface, tokens[surface]];
        if (mode.includes('increased')) return [opaque];
        return [
          opaque,
          [`glass over ${surface}`, over(tokens.panel, glass, tokens[surface])],
          [`dialog glass over scrim over ${surface}`, over(tokens.panel, glass, over(scrim.channels!, scrim.alpha, tokens[surface]))],
        ];
      });
      const pairs: [string, string, string][] = [
        ...['accent', 'lead', 'success', 'warning', 'danger'].flatMap((tone) =>
          backdrops.map(([name, backdrop]): [string, string, string] => [`${tone} on ${tone}/10 on ${name}`, tokens[tone], over(tokens[tone], 0.1, backdrop)])),
        ['accent count badge on the selected sidebar row', tokens.accent, over(tokens.accent, 0.1, over(tokens.accent, 0.1, tokens.surface))],
      ];
      for (const [pair, text, background] of pairs) {
        const ratio = contrast(text, background);
        if (ratio < 4.5) failures.push(`${mode}: ${pair} ${ratio.toFixed(2)}:1 < 4.5`);
      }
    }
    expect(failures).toEqual([]);
  });

  // `control` is the 3:1 border step. Generated as a text colour too, `text-control` (the control
  // type size) painted labels in it, and cn() read it as a colour and dropped the tone beside it.
  it('keep text-control a type size, never a text colour', async () => {
    const { default: resolveConfig } = await import('tailwindcss/resolveConfig');
    const { default: config } = await import(/* @vite-ignore */ pathToFileURL(join(__dirname, '..', '..', 'tailwind.config.js')).href);
    expect(Object.keys(resolveConfig(config).theme.textColor)).not.toContain('control');
    expect(cn('text-success text-caption', 'text-control')).toBe('text-success text-control');
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
