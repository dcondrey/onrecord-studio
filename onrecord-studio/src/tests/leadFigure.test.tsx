// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import CalendarHeat from '../components/charts/figures/CalendarHeat';
import LeadFigure from '../components/charts/figures/LeadFigure';
import type { LeadFigure as LeadFigureData } from '../types';

afterEach(cleanup);

const sources = { tables: ['pd_calls_for_service_2025'] };
const base = { claim: 'descriptive' as const, caption: null, notes: [] };

const KINDS: LeadFigureData[] = [
  { ...base, kind: 'series', measure: 'Series', points: [{ key: '2024', label: 'FY 2024', partial: false }, { key: '2025', label: 'FY 2025', partial: false }, { key: '2026', label: 'FY 2026', partial: true }],
    panels: [{ key: 'beds', label: 'Beds', unit: 'count', values: [{ state: 'value', value: 10 }, { state: 'excluded', reason: 'fewer than 30 reports filed' }, { state: 'value', value: 0 }], context: null }] },
  { ...base, kind: 'stacked', measure: 'Stacked', unit: 'count', normalise: true, segments: [{ key: 'arrest', label: 'Arrest', tone: 'lead' }, { key: 'report', label: 'Report', tone: 'ord-4' }],
    rows: [{ key: 'r', group: 'SLEEPER', label: '2025', partial: false, values: [3, 7], excluded: [{ key: 'blank', label: 'no disposition', count: 2 }], overlay: { label: 'Crisis Response Team (also counted under its outcome)', value: 1 }, note: null }] },
  { ...base, kind: 'paired', claim: 'statistical', measure: 'Paired', unit: 'percent', display: 'dots', subjectLabel: 'Perceived as unhoused', comparisonLabel: 'Everyone else', band: null,
    rows: [{ key: 'a', label: 'Arrest', subject: { state: 'value', value: 0.2 }, comparison: { state: 'value', value: 0.1 }, subjectBasis: '2 of 10', comparisonBasis: '1 of 10' }] },
  { ...base, kind: 'paired', claim: 'statistical', measure: 'Bullets', unit: 'currency', display: 'bullet', subjectLabel: 'Actual', comparisonLabel: 'Adopted',
    band: { percent: 10, minimum: 1_000_000, label: 'Lead threshold (10% and $1M below): app yardstick, not a budget rule' },
    rows: [{ key: '2025', label: 'FY 2025', subject: { state: 'not-recorded', reason: 'no rows in the actuals table; not spending of zero' }, comparison: { state: 'value', value: 5_000_000 }, subjectBasis: null, comparisonBasis: null }] },
  { ...base, kind: 'bins', measure: 'Bins', unit: 'hours', bins: [{ key: 'a', label: 'under 3 h', from: 0, to: 3, count: 4 }, { key: 'b', label: '3 to 72 h', from: 3, to: 72, count: 9 }, { key: 'c', label: '72 h or more', from: 72, to: null, count: 1 }],
    before: { key: 'before', label: 'completed before the notice was logged', count: 2 }, median: 30, reference: { at: 72, label: '72 h reference (app yardstick)' }, denominator: [] },
  { ...base, kind: 'ranked', measure: 'Ranked', unit: 'count', excluded: [{ key: 'no_beat', label: 'no beat recorded', count: 5 }],
    rows: [{ key: '521', label: 'Beat 521', value: { state: 'value', value: 40 }, subject: true, detail: null }, { key: '522', label: 'Beat 522', value: { state: 'not-recorded', reason: 'no count' }, subject: false, detail: null }] },
];

describe('CalendarHeat', () => {
  it('fills a covered day with no record as the lightest step and leaves a day outside coverage unfilled', () => {
    const daily = { counts: new Map([['2025-03-02', 3], ['2025-03-03', 9]]), first: '2025-03-01', last: '2025-03-04', years: new Set([2025]), undated: 2 };
    const { container } = render(<CalendarHeat title="Abatements by day" noun="abatements" daily={daily} sources={sources} size="full" />);
    const filled = [...container.querySelectorAll('svg[aria-roledescription="calendar"] rect')].filter((rect) => rect.getAttribute('style')?.includes('--viz-seq-'));
    // Four covered days, the zeros on 1 and 4 March among them; every other day of 2025 is unfilled.
    expect(filled).toHaveLength(4);
    expect(filled.filter((rect) => rect.getAttribute('style')?.includes('--viz-seq-1'))).toHaveLength(2);
    expect(container.querySelectorAll('[data-absence="not-covered"]')).toHaveLength(365 - 4);
    expect(screen.getByText(/Busiest day: Mon Mar 3, 2025, 9/)).toBeTruthy();
    expect(screen.getByText('2 abatements have no readable date and are on no day.')).toBeTruthy();
  });
});

describe('LeadFigure', () => {
  it('never paints a lead in the counted crimson or tags it as a legal claim, in any kind', () => {
    for (const figure of KINDS) {
      const { container } = render(<LeadFigure figure={figure} headline="A lead" size="full" sources={sources} />);
      expect(container.innerHTML).not.toContain('--viz-counted');
      expect(screen.queryByText('Counted (legal claim)')).toBeNull();
      cleanup();
    }
  });

  it('draws an absent value as its gap with the reason, and the table twin prints the reason and the records outside the measure', () => {
    const ranked = KINDS.find((figure) => figure.kind === 'ranked')!;
    const { container } = render(<LeadFigure figure={ranked} headline="A lead" size="full" sources={sources} />);
    expect(container.querySelector('[data-absence="not-recorded"]')).not.toBeNull();
    expect(container.querySelector('[data-absence="excluded"]')?.textContent).toContain('5 no beat recorded');
    fireEvent.click(screen.getByRole('button', { name: 'View as table' }));
    const table = within(screen.getByRole('table'));
    expect(table.getByText('No value recorded: no count')).toBeTruthy();
    expect(table.getByText('Outside this measure: 5 no beat recorded')).toBeTruthy();
  });
});

describe('LeadFigure geometry', () => {
  const find = <K extends LeadFigureData['kind']>(kind: K) => KINDS.find((figure) => figure.kind === kind) as Extract<LeadFigureData, { kind: K }>;

  it('draws each closed bin as wide as its hours, and the open-ended bin as a tile off the scale', () => {
    const { container } = render(<LeadFigure figure={find('bins')} headline="A lead" size="full" sources={sources} />);
    // Focus rings follow each mark's box: a closed bin's box is its hour span less a 1.5px gap.
    const widths = [...container.querySelectorAll('.viz-focus-ring')].map((ring) => Number(ring.getAttribute('width')) - 6 + 1.5);
    expect(widths).toHaveLength(3);
    expect(widths[0] / widths[1]).toBeCloseTo(3 / 69, 6);
    expect(container.querySelector('[data-open-bin]')?.textContent).toContain('1');
  });

  it('prints a count exactly below 10,000 and compactly from 10,000', () => {
    const ranked = { ...find('ranked'), excluded: [], rows: [
      { key: 'a', label: 'Beat 521', value: { state: 'value' as const, value: 10_000 }, subject: true, detail: null },
      { key: 'b', label: 'Beat 522', value: { state: 'value' as const, value: 9_999 }, subject: false, detail: null }] };
    const { container } = render(<LeadFigure figure={ranked} headline="A lead" size="full" sources={sources} />);
    const labels = [...container.querySelectorAll('text')].map((text) => text.textContent);
    expect(labels).toContain('10K');
    expect(labels).toContain('9,999');
  });

  it("gives an absent bullet subject a slot with the builder's reason, not a 'not recorded' word in the comparison's track", () => {
    const bullet = KINDS.find((figure) => figure.kind === 'paired' && figure.display === 'bullet')!;
    const { container } = render(<LeadFigure figure={bullet} headline="A lead" size="full" sources={sources} />);
    const slot = container.querySelector('[data-absence="not-recorded"]')!;
    expect(slot.querySelector('text')?.textContent).toMatch(/^no rows in the actuals/);
    expect([...container.querySelectorAll('text')].some((text) => text.textContent === 'not recorded')).toBe(false);
  });

  it('marks the focused year across the panels, and a card keeps the first panel and says how many more there are', () => {
    const series = find('series');
    const twoPanels = { ...series, panels: [...series.panels, { ...series.panels[0], key: 'other', label: 'Other beds' }] };
    const { container } = render(<LeadFigure figure={twoPanels} headline="A lead" size="full" sources={sources} />);
    const year = screen.getAllByRole('img').find((mark) => mark.getAttribute('aria-label')?.startsWith('FY 2024'))!;
    expect(container.querySelector('[data-crosshair]')).toBeNull();
    fireEvent.focus(year);
    expect(container.querySelector('[data-crosshair]')).not.toBeNull();
    fireEvent.blur(year);
    expect(container.querySelector('[data-crosshair]')).toBeNull();
    cleanup();
    render(<LeadFigure figure={twoPanels} headline={null} size="card" sources={sources} />);
    expect(screen.queryByText('Other beds')).toBeNull();
    expect(screen.getByText("+1 panel in the lead's dossier.")).toBeTruthy();
  });
});

describe('CalendarHeat on a card', () => {
  it('names every covered day and keeps the grid to one tab stop', () => {
    const daily = { counts: new Map([['2025-03-04', 12]]), first: '2025-03-01', last: '2025-03-10', years: new Set([2025]), undated: 0 };
    const { container } = render(<CalendarHeat title="Abatements by day" noun="abatements" daily={daily} sources={sources} size="card" />);
    expect(screen.getByRole('img', { name: 'Tue Mar 4, 2025: 12 abatements' })).toBeTruthy();
    expect(screen.getAllByRole('img', { name: /abatements$/ })).toHaveLength(10);
    expect(container.querySelectorAll('[tabindex="0"]')).toHaveLength(1);
  });
});
