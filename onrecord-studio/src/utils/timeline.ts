import type { ClaimedRequest, Finding, TimelineEventKind, WatchChange } from '../types';

export type TimelineSource = 'REQUEST' | 'FINDING' | 'WATCH' | 'CASE';

/**
 * What a timeline date means. They are not interchangeable: an event date is when something
 * happened, the other two are when this app saw it. Mixing them silently would make "noticed
 * Tuesday" read as "happened Tuesday".
 */
export type DateMeaning = 'OCCURRED' | 'LAST_SEEN' | 'DETECTED' | 'DUE';

/** A case citation behind an item, with the verification rung it was saved at. */
export interface TimelineCitation {
  label: string;
  grade: string;
}

export interface TimelineItem {
  id: string;
  source: TimelineSource;
  at: string;
  dateMeaning: DateMeaning;
  title: string;
  detail: string | null;
  provenance: string;
  evidence: string | null;
  findingId: string | null;
  /** Only case items carry this. An empty list is a case item with no citation recorded. */
  citations?: TimelineCitation[];
  /** Lane ids this item belongs to; see `caseTimeline.ts`. */
  lanes?: string[];
}

export const DATE_MEANING_LABELS: Record<DateMeaning, string> = {
  OCCURRED: 'Event date',
  LAST_SEEN: 'Last seen',
  DETECTED: 'Noticed by this app',
  DUE: 'Due or modeled date',
};

export const SOURCE_LABELS: Record<TimelineSource, string> = {
  REQUEST: 'My requests',
  FINDING: 'Findings',
  WATCH: 'Watch changes',
  CASE: 'Case records',
};

export function requestItems(claims: ClaimedRequest[], kindLabels: Record<TimelineEventKind, string>): TimelineItem[] {
  return claims.flatMap((claim) =>
    claim.events.map((event) => ({
      id: `request:${claim.id}:${event.id}`,
      source: 'REQUEST' as const,
      at: event.occurredAt,
      dateMeaning: 'OCCURRED' as const,
      title: `${claim.reference}: ${kindLabels[event.kind]}`,
      detail: claim.title,
      provenance: event.basis === 'EMAIL_CONFIRMED' ? 'Confirmed by you from an email' : 'Entered by you',
      evidence: event.evidence?.matchedSentence ?? null,
      findingId: null,
    })),
  );
}

export function findingItems(findings: Finding[]): TimelineItem[] {
  return findings.map((finding) => ({
    id: `finding:${finding.id}`,
    source: 'FINDING' as const,
    at: finding.lastSeenAt,
    dateMeaning: 'LAST_SEEN' as const,
    title: finding.headline,
    detail: null,
    provenance: `Detector ${finding.sourceDetector}; first discovered ${finding.discoveredAt.slice(0, 10)}. A pattern signal, not a legal claim.`,
    evidence: finding.explanation,
    findingId: finding.id,
  }));
}

export function watchItems(changes: WatchChange[]): TimelineItem[] {
  return changes.map((change) => {
    const from = change.oldValue === null ? 'not present in the baseline' : change.oldValue;
    const to = change.newValue === null ? 'no longer present' : change.newValue;
    return {
      id: `watch:${change.id}`,
      source: 'WATCH' as const,
      at: change.detectedAt,
      dateMeaning: 'DETECTED' as const,
      title: `${change.changeKind.replace(/_/g, ' ').toLowerCase()}: ${change.subject}`,
      detail: `${from} → ${to}`,
      provenance: `Watch check, basis ${change.basis}`,
      evidence: null,
      findingId: null,
    };
  });
}

export interface TimelineFilter {
  sources: ReadonlySet<TimelineSource>;
  /** `YYYY-MM-DD`, inclusive; empty for no bound. */
  from: string;
  to: string;
  text: string;
  /** Keep only items in this lane; null shows every lane. */
  lane?: string | null;
}

export interface TimelineView {
  items: TimelineItem[];
  /** Items whose date could not be read. They are excluded from the list, never shown at a made-up date. */
  unreadableDates: number;
}

function bound(day: string, end: boolean): number | null {
  if (!day) return null;
  const parsed = Date.parse(`${day}T${end ? '23:59:59.999' : '00:00:00'}Z`);
  return Number.isFinite(parsed) ? parsed : null;
}

export function buildTimeline(all: TimelineItem[], filter: TimelineFilter): TimelineView {
  const lower = bound(filter.from, false);
  const upper = bound(filter.to, true);
  const needle = filter.text.trim().toLowerCase();
  let unreadableDates = 0;
  const dated: { item: TimelineItem; time: number }[] = [];
  for (const item of all) {
    if (!filter.sources.has(item.source)) continue;
    if (filter.lane && !(item.lanes ?? []).includes(filter.lane)) continue;
    const time = Date.parse(item.at);
    if (!Number.isFinite(time)) {
      unreadableDates += 1;
      continue;
    }
    if ((lower !== null && time < lower) || (upper !== null && time > upper)) continue;
    if (needle && !`${item.title} ${item.detail ?? ''} ${item.evidence ?? ''}`.toLowerCase().includes(needle)) continue;
    dated.push({ item, time });
  }
  dated.sort((a, b) => b.time - a.time || a.item.id.localeCompare(b.item.id));
  return { items: dated.map((entry) => entry.item), unreadableDates };
}

export function monthKey(at: string): string {
  return at.slice(0, 7);
}
