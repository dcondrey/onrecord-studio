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

import { rankFeed } from '../components/dashboard/AttentionFeed';
import type { AgencyPerformanceProfile, Finding, FindingKind, FindingSeverity } from '../types';

const finding = (id: string, severity: FindingSeverity, kind: FindingKind, dismissedAt: string | null = null): Finding => ({
  id, kind, severity, confidence: { kind: 'UNASSESSED' },
  subject: { portalKey: 'p', agencyId: null, requestId: null, documentId: null, entityId: null },
  headline: id, explanation: '', citations: [], discoveredAt: '2026-10-01T00:00:00Z', lastSeenAt: '2026-10-01T00:00:00Z', dismissedAt, sourceDetector: 'test',
});

const profile = (agencyId: number, violations: number) => ({ agencyId, portalSubdomain: 'p', agencyName: `Agency ${agencyId}`, statutoryViolationCount: violations } as AgencyPerformanceProfile);

describe('rankFeed', () => {
  it('ranks a counted violation above a pattern signal of the same severity and leaves out dismissed findings', () => {
    const items = rankFeed({
      findings: [finding('pattern', 'SERIOUS', 'BOILERPLATE_DENIAL'), finding('counted', 'SERIOUS', 'POLICY_VIOLATION_DETECTED'), finding('gone', 'CRITICAL', 'BOILERPLATE_DENIAL', '2026-10-01T01:00:00Z')],
      calendar: null,
      changes: [],
    }, [], () => {});
    expect(items.map((item) => item.key)).toEqual(['finding:counted', 'finding:pattern']);
  });

  it('lists only agencies with a counted violation', () => {
    const items = rankFeed({ findings: [], calendar: null, changes: [] }, [profile(1, 0), profile(2, 3)], () => {});
    expect(items.map((item) => item.key)).toEqual(['agency:p:2']);
  });
});
