import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import colors from 'tailwindcss/colors';
import { describe, expect, it } from 'vitest';

// The dark appearance paints panels with the raw Tailwind slate scale, so a text utility below
// 4.5:1 on those surfaces is unreadable no matter how the tokens are tuned. That is the whole
// basis of the App Store "Sufficient Contrast" claim, and a single `text-slate-500` re-introduced
// in a later component would quietly falsify it. Light appearance is covered by the
// `[data-theme="light"]` remaps in index.css, which repoint every accent family at a --viz-* token.
const DARK_SURFACES = ['#020617', '#0f172a', '#1e293b'];
const MINIMUM_RATIO = 4.5;

// A dark utility is correct on a light chip; those lines are excluded rather than exempted.
const LIGHT_CHIP = /bg-(white|slate-(50|100|200|300)|(amber|cyan|emerald|rose|indigo|red|fuchsia|purple)-(100|200|300|400|500))(?![/[0-9])/;

function channel(value: number) {
  const v = value / 255;
  return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
}

function luminance(hex: string) {
  const value = hex.replace('#', '');
  const [r, g, b] = [0, 2, 4].map((i) => channel(parseInt(value.slice(i, i + 2), 16)));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrast(foreground: string, background: string) {
  const [a, b] = [luminance(foreground), luminance(background)];
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

function sourceFiles(directory: string): string[] {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return entry.isFile() && path.endsWith('.tsx') ? [path] : [];
  });
}

describe('dark appearance text contrast', () => {
  it('keeps every text utility at or above 4.5:1 on the slate surfaces it renders on', () => {
    const palette = colors as unknown as Record<string, Record<string, string>>;
    const failures: string[] = [];

    for (const file of sourceFiles(join(__dirname, '..', 'components'))) {
      readFileSync(file, 'utf8').split('\n').forEach((line, index) => {
        if (LIGHT_CHIP.test(line)) return;
        for (const [, family, shade] of line.matchAll(/\btext-([a-z]+)-(\d{2,3})\b/g)) {
          const hex = palette[family]?.[shade];
          if (typeof hex !== 'string' || !hex.startsWith('#')) continue;
          const worst = Math.min(...DARK_SURFACES.map((surface) => contrast(hex, surface)));
          if (worst < MINIMUM_RATIO) {
            failures.push(`${file.split('/src/')[1]}:${index + 1} text-${family}-${shade} (${worst.toFixed(2)}:1)`);
          }
        }
      });
    }

    expect(failures).toEqual([]);
  });
});
