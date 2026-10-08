// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';

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
      get length() {
        return entries.size;
      },
    },
  });
});

const bridge = vi.hoisted(() => ({ pending: [] as ((value: unknown) => void)[] }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) =>
    command === 'list_topic_packs'
      ? new Promise((resolve) => bridge.pending.push(resolve))
      : Promise.resolve(null),
}));

import TopicPacks from '../components/dashboard/TopicPacks';
import { useAppStore } from '../store/useAppStore';
import { buildBriefingMarkdown, topicBriefingProject } from '../utils/briefing';
import type { Finding, TopicFindingMatch, TopicPackView, TopicPacksReport } from '../types';

function finding(id: string, confidence: Finding['confidence']): Finding {
  return {
    id,
    kind: 'HELD_BACK_RESPONSE',
    severity: 'NOTABLE',
    confidence,
    subject: { portalKey: 'city', agencyId: null, requestId: 4, documentId: 9, entityId: null },
    headline: `Headline ${id}`,
    explanation: 'Explanation.',
    citations: [{ portalKey: 'city', documentId: 9, characterStart: 5, characterEnd: 12, quote: 'SECRET QUOTE', label: null }],
    discoveredAt: '2026-09-01T00:00:00Z',
    lastSeenAt: '2026-09-01T00:00:00Z',
    dismissedAt: null,
    sourceDetector: 'test',
  };
}

function topic(findings: TopicFindingMatch[]): TopicPackView {
  return {
    id: 'abatement',
    title: 'Encampment abatement and sweeps',
    cues: ['encampment'],
    documentTotal: 0,
    documents: [],
    requestTotal: 0,
    requests: [],
    findingTotal: findings.length,
    findings,
  };
}

function report(findings: TopicFindingMatch[]): TopicPacksReport {
  return {
    topics: [topic(findings)],
    documentsScanned: 1,
    documentsPrefixOnly: 0,
    documentsFileNameOnly: 0,
    documentLimitReached: false,
    requestsScanned: 1,
    requestLimitReached: false,
    textPrefixCharacters: 500000,
  };
}

afterEach(() => {
  cleanup();
  bridge.pending = [];
});

describe('TopicPacks', () => {
  it('discards a load that a newer refresh superseded', async () => {
    render(<TopicPacks />);
    act(() => useAppStore.getState().requestFindingsRefresh());
    expect(bridge.pending).toHaveLength(2);
    await act(async () => bridge.pending[1](report([{ basis: 'PATTERN', anchorRequestId: 4, finding: finding('newer', { kind: 'UNASSESSED' }) }])));
    await act(async () => bridge.pending[0](report([{ basis: 'PATTERN', anchorRequestId: 4, finding: finding('older', { kind: 'UNASSESSED' }) }])));
    expect(screen.getByText('Headline newer')).toBeTruthy();
    expect(screen.queryByText('Headline older')).toBeNull();
  });

  it('shows basis always and confidence only when measured, including a measured zero', async () => {
    render(<TopicPacks />);
    await act(async () => bridge.pending[0](report([
      { basis: 'COUNTED', anchorRequestId: 4, finding: finding('a', { kind: 'MEASURED', basisPoints: 0 }) },
      { basis: 'PATTERN', anchorRequestId: 4, finding: finding('b', { kind: 'UNASSESSED' }) },
    ])));
    expect(screen.getByText('Counted violation')).toBeTruthy();
    expect(screen.getByText('Pattern signal, not a legal claim')).toBeTruthy();
    expect(screen.getByText('0% measured')).toBeTruthy();
    expect(screen.queryByText('Unassessed')).toBeNull();
  });
});

describe('TopicPacks reading depth', () => {
  it('states how deeply the scanned documents were read and withholds a basis split from a truncated page', async () => {
    render(<TopicPacks />);
    const truncated = { ...topic([{ basis: 'COUNTED', anchorRequestId: 4, finding: finding('a', { kind: 'UNASSESSED' }) }]), findingTotal: 3 };
    await act(async () => bridge.pending[0]({ ...report([]), topics: [truncated], documentsScanned: 10, documentsPrefixOnly: 3, documentsFileNameOnly: 2 }));
    expect(screen.getByRole('meter', { name: 'Reading depth of scanned documents' }).getAttribute('aria-valuetext'))
      .toBe('5 of 10 scanned documents were read in full; 3 to the first 500,000 characters only; 2 by file name only.');
    const pack = screen.getByRole('img', { name: /^Encampment abatement and sweeps:/ });
    expect(pack.getAttribute('aria-label')).toContain('first 1 of 3 findings loaded');
    expect(pack.textContent).not.toContain('1 counted');
  });

  it('claims nothing legal for a chart of tagged-document counts, even when a loaded finding is counted', async () => {
    render(<TopicPacks />);
    await act(async () => bridge.pending[0](report([{ basis: 'COUNTED', anchorRequestId: 4, finding: finding('a', { kind: 'UNASSESSED' }) }])));
    expect(screen.queryByText('Counted (legal claim)')).toBeNull();
    expect(screen.getByRole('img', { name: /^Encampment abatement and sweeps:/ }).textContent).toContain('1 counted');
  });

  it('never paints a zero counted crimson', async () => {
    render(<TopicPacks />);
    await act(async () => bridge.pending[0](report([{ basis: 'STATED', anchorRequestId: 4, finding: finding('a', { kind: 'UNASSESSED' }) }])));
    expect(screen.queryByText('Counted (legal claim)')).toBeNull();
    const pack = screen.getByRole('img', { name: /^Encampment abatement and sweeps:/ });
    expect(pack.textContent).toContain('0 counted');
    const crimson = [...pack.querySelectorAll('*')].filter((node) => (node.getAttribute('style') ?? '').includes('--viz-counted'));
    expect(crimson).toHaveLength(0);
  });
});

describe('topic briefing export', () => {
  it('lists basis and citation locators, leaves quotes out, and states confidence only when measured', () => {
    const markdown = buildBriefingMarkdown(topicBriefingProject(topic([
      { basis: 'PATTERN', anchorRequestId: 4, finding: finding('a', { kind: 'UNASSESSED' }) },
      { basis: 'COUNTED', anchorRequestId: 4, finding: finding('b', { kind: 'MEASURED', basisPoints: 7200 }) },
    ])), new Date('2026-09-30T00:00:00Z'));
    expect(markdown).toContain('Pattern signal, not a legal claim: Headline a');
    expect(markdown).toContain('Counted violation: Headline b');
    expect(markdown).toContain('- city / document 9 / characters 5–12');
    expect(markdown).toContain('Measured confidence: 72%');
    expect(markdown.match(/Measured confidence/g)).toHaveLength(1);
    expect(markdown).not.toContain('SECRET QUOTE');
    expect(markdown).toContain('city / request 4 / document 9');
  });

  it('leaves inferred-redaction findings out and says so', () => {
    const inferred = { ...finding('inferred', { kind: 'UNASSESSED' }), kind: 'REDACTION_CANDIDATE_INFERRED' as const };
    const markdown = buildBriefingMarkdown(topicBriefingProject(topic([
      { basis: 'PATTERN', anchorRequestId: 4, finding: inferred },
      { basis: 'PATTERN', anchorRequestId: 4, finding: finding('kept', { kind: 'UNASSESSED' }) },
    ])));
    expect(markdown).not.toContain('Headline inferred');
    expect(markdown).toContain('Headline kept');
    expect(markdown).toContain('1 inferred-redaction finding is left out');
  });
});
