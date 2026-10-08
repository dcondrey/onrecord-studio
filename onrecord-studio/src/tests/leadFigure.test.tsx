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
