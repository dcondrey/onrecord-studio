// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';

vi.hoisted(() => {
  const entries = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => entries.get(key) ?? null,
      setItem: (key: string, value: string) => void entries.set(key, String(value)),
      removeItem: (key: string) => void entries.delete(key),
      clear: () => entries.clear(),
      key: (index: number) => [...entries.keys()][index] ?? null,
      get length() { return entries.size; },
    },
  });
});

import { deadlineLedgerHeadline, rankFeed } from '../components/dashboard/AttentionFeed';
import type { Finding, FindingKind, FindingSeverity } from '../types';

const finding = (id: string, severity: FindingSeverity, kind: FindingKind, dismissedAt: string | null = null): Finding => ({
  id, kind, severity, confidence: { kind: 'UNASSESSED' },
  subject: { portalKey: 'p', agencyId: null, requestId: null, documentId: null, entityId: null },
  headline: id, explanation: '', citations: [], discoveredAt: '2026-10-01T00:00:00Z', lastSeenAt: '2026-10-01T00:00:00Z', dismissedAt, sourceDetector: 'test',
});

describe('rankFeed', () => {
  it('ranks a counted violation above a pattern signal of the same severity and leaves out dismissed findings', () => {
    const items = rankFeed({
      findings: [finding('pattern', 'SERIOUS', 'BOILERPLATE_DENIAL'), finding('counted', 'SERIOUS', 'POLICY_VIOLATION_DETECTED'), finding('gone', 'CRITICAL', 'BOILERPLATE_DENIAL', '2026-10-01T01:00:00Z')],
      calendar: null,
      changes: [],
    }, [], () => {});
    expect(items.map((item) => item.key)).toEqual(['finding:counted', 'finding:pattern']);
  });

  it('adds one deadline ledger row only when a counted miss exists, outside the ranked items', () => {
    const profile = (agencyName: string, statutoryViolationCount: number, totalRequestsProcessed: number) => ({
      agencyId: 1, portalSubdomain: 'p', agencyName, totalRequestsProcessed, totalClosed: 0, validLatencySampleSize: 0, excludedInvalidLifetimeCount: 0,
      averageClosureDays: null, globalMeanClosureDays: null, globalStddevClosureDays: null, performanceZScore: null, complianceIndicator: 'Compliant' as const,
      anomalyCount: 0, statutoryViolationCount, statutoryViolationRate: 0, statutoryDeadlinePassedCount: 0, suspiciousFastClosureCount: 0, totalDocumentsReleased: 0, keywordFrequencyMap: {},
    });
    expect(deadlineLedgerHeadline([profile('Parks', 0, 12)])).toBeNull();
    expect(deadlineLedgerHeadline([profile('Police', 22, 268), profile('Library', 3, 40), profile('Parks', 0, 12)]))
      .toBe('2 departments missed the statutory deadline on 25 of 320 requests; Police accounts for 22.');
  });
});

