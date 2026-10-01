import { describe, expect, it } from 'vitest';
import { buildTimeline, type TimelineItem, type TimelineSource } from '../utils/timeline';

const item = (id: string, source: TimelineSource, at: string, title = id): TimelineItem => ({
  id,
  source,
  at,
  dateMeaning: source === 'REQUEST' ? 'OCCURRED' : 'DETECTED',
  title,
  detail: null,
  provenance: '',
  evidence: null,
  findingId: null,
});
const everything = new Set<TimelineSource>(['REQUEST', 'FINDING', 'WATCH']);

describe('buildTimeline', () => {
  const all = [
    item('a', 'REQUEST', '2026-03-02T10:00:00Z'),
    item('b', 'FINDING', '2026-03-31T23:59:59Z'),
    item('c', 'WATCH', '2026-04-01T00:00:00Z'),
    item('bad', 'FINDING', 'not a date'),
  ];

  it('orders newest first, includes both bound days, and counts unreadable dates instead of placing them', () => {
    const view = buildTimeline(all, { sources: everything, from: '2026-03-02', to: '2026-03-31', text: '' });
    expect(view.items.map((entry) => entry.id)).toEqual(['b', 'a']);
    expect(view.unreadableDates).toBe(1);
    expect(buildTimeline(all, { sources: everything, from: '', to: '', text: '' }).items.map((entry) => entry.id)).toEqual(['c', 'b', 'a']);
  });

  it('narrows by source and text without losing the unreadable count', () => {
    const view = buildTimeline(all, { sources: new Set(['REQUEST']), from: '', to: '', text: 'A' });
    expect(view.items.map((entry) => entry.id)).toEqual(['a']);
    expect(view.unreadableDates).toBe(0);
  });
});
