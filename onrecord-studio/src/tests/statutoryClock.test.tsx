// @vitest-environment jsdom
//
// The statutory clock's claim rule and its request-identity guard. Crimson (a counted span) comes
// only from a counted measure: the tracker's violationAsserted or a stored statutory_violations
// finding. A casework model, the evaluator's own isViolation, and a dismissed finding never paint
// counted. And a clock read for request A that resolves after B is selected never lands on B.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';

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

const engineReads = vi.hoisted(() => new Map<number, (value: unknown) => void>());

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: { requestId?: number }) => {
    if (command === 'get_portal_request_deadline' && args?.requestId !== undefined) {
      return new Promise((resolve) => engineReads.set(args.requestId as number, resolve));
    }
    if (command === 'get_request_statutory_extension') return Promise.resolve(null);
    if (command === 'query_audit_records_page') return Promise.reject(new Error('not under test'));
    return Promise.resolve([]);
  },
}));

import MainLayout from '../components/dashboard/MainLayout';
import ClockStrip, { caseworkClock, countedSpanStartDay, pastDeadlinePaint, portalClock, trackerClock } from '../components/charts/figures/ClockStrip';
import ClockStack from '../components/charts/figures/ClockStack';
import { KIND_LABELS } from '../components/tracker/MyRequestsPanel';
import { useAppStore } from '../store/useAppStore';
import type { ClaimedRequest, DeadlineAnalysis, LegalComplianceFinding, RequestDeadline, RequestFilingPlan, RequestSearchResult } from '../types';

function analysis(overrides: Partial<DeadlineAnalysis> = {}): DeadlineAnalysis {
  return {
    requestId: 1,
    statuteCitation: 'Gov. Code § 7922.535',
    createdAt: '2026-08-01T00:00:00Z',
    determinationAt: null,
    determinationSource: null,
    determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY',
    observedAt: '2026-09-10T00:00:00Z',
    initialDeadlineAt: '2026-08-11T00:00:00Z',
    effectiveDeadlineAt: '2026-08-11T00:00:00Z',
    calendarDaysElapsed: 40,
    allowedCalendarDays: 10,
    extensionStatus: 'NOT_INVOKED',
    extensionReason: null,
    isViolation: true,
    anomalyMarkers: [],
    evidenceSummary: '',
    ...overrides,
  };
}

const claim: ClaimedRequest = {
  id: 'c1', portalKey: null, reference: '26-0001', title: 'Logs', sourceUrl: null, jurisdiction: 'CA_CPRA', claimedAt: '2026-08-01T00:00:00Z',
  events: [{ id: 'e1', kind: 'SUBMITTED', occurredAt: '2026-08-01T00:00:00Z', basis: 'USER_ENTERED', evidence: null, extensionReason: null, expectedDeterminationAt: null, createdAt: '2026-08-01T00:00:00Z' }],
  suggestions: [], importedEmails: [],
};

function deadline(overrides: Partial<RequestDeadline>): RequestDeadline {
  return {
    claimId: 'c1', model: 'CALIFORNIA_CPRA', analysis: analysis(), countdown: { kind: 'DEADLINE_PASSED', days: 30 },
    determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY', violationAsserted: false, basisStatement: '', notes: [], pendingSuggestionCount: 0,
    ...overrides,
  };
}

function finding(reviewStatus: LegalComplianceFinding['reviewStatus']): LegalComplianceFinding {
  return {
    violationId: 'v1', requestId: 1, portalKey: 'p', matchedStatuteId: 's', matchedVersionId: 'v', mandateId: null, statuteCitation: 'Gov. Code § 7922.535',
    authorityTitle: '', mandateTitle: null, classification: '', calculatedVarianceMetric: null, varianceDays: 30, confidenceBasisPoints: 10_000,
    evidenceSummary: '', supportingDocumentIds: [], missingEvidencePhrases: [], reviewStatus, firstDetectedAt: '', evaluatedAt: '',
  };
}

const plan: RequestFilingPlan = {
  statuteCitation: 'Gov. Code § 7922.535', evaluatedAt: '2026-09-10', filedOn: '2026-08-01', determinationDueOn: '2026-08-11',
  calendarDaysElapsed: 40, allowedCalendarDays: 10, determinationBasis: 'OBSERVATION_WITHOUT_RECORDED_ACTIVITY',
  measuredTo: null, deadlinePassedWithoutRecordedResponse: true, followUps: [],
};

const pastSpans = (container: HTMLElement) => [...container.querySelectorAll('[data-past]')].map((node) => node.getAttribute('data-past'));

afterEach(() => {
  cleanup();
  engineReads.clear();
});

describe('statutory clock claim paint', () => {
  it('paints the past-deadline span counted only for a counted measure', () => {
    const asserted = render(<ClockStrip data={trackerClock(claim, deadline({ violationAsserted: true }), KIND_LABELS)} storageKey="t1" />);
    expect(pastSpans(asserted.container)).toEqual(['counted']);
    cleanup();

    // The evaluator's own isViolation is arithmetic, not a counted finding.
    const provisional = render(<ClockStrip data={trackerClock(claim, deadline({ violationAsserted: false }), KIND_LABELS)} storageKey="t2" />);
    expect(pastSpans(provisional.container)).toEqual(['uncounted']);
    expect(provisional.container.textContent).toContain('past the deadline (provisional)');
    cleanup();

    const engineOnly = portalClock({ record: null, extension: null, engine: analysis(), findings: [], observedAt: '2026-09-10T00:00:00Z' });
    expect(pastDeadlinePaint(engineOnly)).toBe('uncounted');
    const dismissed = portalClock({ record: null, extension: null, engine: analysis(), findings: [finding('DISMISSED')], observedAt: '2026-09-10T00:00:00Z' });
    expect(pastDeadlinePaint(dismissed)).toBe('dismissed');
    const stored = portalClock({ record: null, extension: null, engine: analysis(), findings: [finding('OPEN')], observedAt: '2026-09-10T00:00:00Z' });
    expect(pastDeadlinePaint(stored)).toBe('counted');
    // Re-evaluation writes RESOLVED_BY_NEW_EVIDENCE exactly when it no longer finds a violation.
    const resolvedData = portalClock({ record: null, extension: null, engine: analysis(), findings: [finding('RESOLVED_BY_NEW_EVIDENCE')], observedAt: '2026-09-10T00:00:00Z' });
    expect(pastDeadlinePaint(resolvedData)).toBe('resolved');
    expect(resolvedData.headline).not.toContain('counted, exceeded');
    const resolved = render(<ClockStrip data={resolvedData} storageKey="t3" />);
    expect(pastSpans(resolved.container)).toEqual(['resolved']);
    expect(resolved.container.textContent).toContain('resolved by new evidence: not counted');
    expect(resolved.container.querySelector('[data-flag]')).toBeNull();
    cleanup();
    // A withdrawn finding with a larger variance never wins over a live one.
    const mixed = portalClock({ record: null, extension: null, engine: analysis(), findings: [{ ...finding('RESOLVED_BY_NEW_EVIDENCE'), violationId: 'old', varianceDays: 40 }, finding('CONFIRMED')], observedAt: '2026-09-10T00:00:00Z' });
    expect(mixed.model !== 'casework' && mixed.counted?.reviewStatus).toBe('CONFIRMED');
  });

  it('stops crimson at the stored variance and draws later days as not counted', () => {
    // [x0, split, crimson end] from the rendered bar: inside path, then the counted path.
    const spans = (container: HTMLElement) => {
      const [inside, counted] = [...container.querySelectorAll('[data-clock-bar] > path')].map((node) => node.getAttribute('d') ?? '');
      const num = (d: string, re: RegExp) => Number(re.exec(d)![1]);
      return { x0: num(inside, /^M([\d.]+),/), split: num(counted, /^M([\d.]+),/) - 1, end: num(counted, /H([\d.]+)/) };
    };
    // Phase 1, still open: a 5-day variance measured on day 20 counts days 15-20; today is day 40.
    const open = { createdAt: '2026-08-01T00:00:00Z', closedAt: null, status: 'open' };
    const phase1 = render(<ClockStrip data={portalClock({ record: open, extension: null, engine: null, findings: [{ ...finding('OPEN'), varianceDays: 5, evaluatedAt: '2026-08-21T00:00:00Z' }], observedAt: '2026-09-10T00:00:00Z' })} storageKey="w1" />);
    expect(pastSpans(phase1.container)).toEqual(['counted', 'uncounted']);
    let s = spans(phase1.container);
    expect(s.end - s.split).toBeCloseTo(((s.split - s.x0) / 15) * 5, 1);
    cleanup();
    // Phase 2, no determination: deadline day 10, 20 days stored, observed on day 40.
    const phase2 = render(<ClockStrip data={portalClock({ record: open, extension: null, engine: analysis(), findings: [{ ...finding('OPEN'), varianceDays: 20 }], observedAt: '2026-09-10T00:00:00Z' })} storageKey="w2" />);
    expect(pastSpans(phase2.container)).toEqual(['counted', 'uncounted']);
    s = spans(phase2.container);
    expect(s.end - s.split).toBeCloseTo(((s.split - s.x0) / 10) * 20, 1);
    expect(phase2.container.textContent).toContain('Past the deadline (not counted)');
  });

  it('never calls a closure it did not load unrecorded', () => {
    const engineOnly = portalClock({ record: null, extension: null, engine: analysis(), findings: [], observedAt: '2026-09-10T00:00:00Z' });
    expect(engineOnly.headline).not.toContain('no closing date recorded');
    expect(engineOnly.headline).toContain('closing date not loaded here');
    const proxy = portalClock({ record: null, extension: null, engine: analysis({ determinationAt: '2026-08-20T00:00:00Z', determinationBasis: 'EARLIEST_RECORDED_ACTIVITY_PROXY', calendarDaysElapsed: 19 }), findings: [], observedAt: '2026-09-10T00:00:00Z' });
    expect(proxy.headline).toContain('Earliest recorded activity (a proxy for the determination) on day 19');
    // With no dates at all, a stored finding is still stated rather than dropped.
    const noDates = portalClock({ record: null, extension: null, engine: null, findings: [finding('OPEN')], observedAt: '2026-09-10T00:00:00Z' });
    expect(noDates.noClock).toContain('counted, exceeded by 30 days');
  });

  it('places a stored variance back from the instant it was measured to, never from today', () => {
    const open = { createdAt: '2026-08-01T00:00:00Z', closedAt: null, status: 'open' };
    const stored = { ...finding('OPEN'), varianceDays: 5, evaluatedAt: '2026-08-21T00:00:00Z' };
    const early = portalClock({ record: open, extension: null, engine: null, findings: [stored], observedAt: '2026-09-10T00:00:00Z' });
    const later = portalClock({ record: open, extension: null, engine: null, findings: [stored], observedAt: '2026-12-10T00:00:00Z' });
    // Evaluated on day 20 with 5 days over: the counted span starts on day 15 whatever today is.
    expect(countedSpanStartDay(early)).toBe(15);
    expect(countedSpanStartDay(later)).toBe(15);
  });

  it('never paints a casework model as counted, even past its modelled date', () => {
    const data = caseworkClock(plan, []);
    expect(pastDeadlinePaint(data)).toBe('uncounted');
    const { container } = render(<ClockStrip data={data} storageKey="c1" />);
    expect(pastSpans(container)).toEqual(['uncounted']);
    expect(container.textContent).toContain('past the deadline (modelled)');
    expect(container.textContent).not.toContain('Counted');
  });

  it('stacks casework clocks as modelled and lists an unfiled request without a bar', () => {
    const base = {
      id: 'r1', title: 'Amendments', custodian: 'Purchasing', gap: '', resolves: '', draft: '', state: 'SUBMITTED', expectedRecordTypes: [], expectedFields: [],
      expectedPeriod: '', submittedAt: '2026-08-01', responseDue: null, externalReference: '', productions: [], unresolvedOmissions: [], followUpDraft: '',
      citationIds: [], compiledDraft: '', filingPlan: plan,
    };
    const found = {
      id: 'case', reportId: 'r', topic: '', title: 'Case', owner: '', disposition: 'OPEN', revision: 1, createdAt: '', updatedAt: '', questions: [],
      requests: [base, { ...base, id: 'r2', title: 'Unfiled', submittedAt: null, filingPlan: null }], decisions: [], commitments: [], evidence: [], notes: '',
    };
    const { container } = render(<ClockStack found={found} onOpenRequest={() => undefined} />);
    expect(pastSpans(container)).toEqual(['uncounted']);
    expect(container.textContent).toContain('modelled: past due, no recorded response');
    expect(container.textContent).toContain('not filed: no clock');
    expect(container.querySelectorAll('[data-clock-bar]')).toHaveLength(1);
  });

  it('draws no axis or bar when the tracker has no clock', () => {
    const { container } = render(
      <ClockStrip data={trackerClock(claim, deadline({ analysis: null, countdown: { kind: 'NO_SUBMISSION_DATE', days: null } }), KIND_LABELS)} storageKey="n1" />,
    );
    expect(container.querySelector('[data-clock-bar]')).toBeNull();
    expect(container.querySelector('svg')).toBeNull();
    expect(container.textContent).toContain('No clock is drawn: no submission date is recorded.');
  });
});

describe('evidence request clock identity', () => {
  const requestA: RequestSearchResult = {
    requestId: 101, portalKey: 'example-gov', agencyId: 7, agencyName: 'Public Works', title: 'Request A', descriptionPreview: '',
    status: 'closed', createdAt: '2026-01-05T00:00:00Z', closedAt: '2026-01-20T00:00:00Z', documentCount: 0, integrityFlags: [],
  };
  const requestB: RequestSearchResult = { ...requestA, requestId: 202, title: 'Request B', createdAt: '2026-08-01T00:00:00Z', closedAt: null, status: 'open' };

  beforeEach(() => {
    useAppStore.setState({
      workspaceView: 'evidence',
      evidenceMode: 'records',
      filteredResults: [requestA, requestB],
      searchError: null,
      selectedRequestId: requestA.requestId,
      selectedRequestPortalKey: requestA.portalKey,
      activeLegalFindings: [],
      agencyProfiles: [],
    });
  });

  it('drops a deadline read for the previous request when it resolves after the switch', async () => {
    render(<MainLayout />);
    await waitFor(() => expect(engineReads.has(requestA.requestId)).toBe(true));
    act(() => {
      useAppStore.setState({ selectedRequestId: requestB.requestId, selectedRequestPortalKey: requestB.portalKey });
    });
    await waitFor(() => expect(engineReads.has(requestB.requestId)).toBe(true));
    await act(async () => {
      engineReads.get(requestB.requestId)!(analysis({ requestId: requestB.requestId, createdAt: '2026-08-01T00:00:00Z', observedAt: '2026-08-05T00:00:00Z', calendarDaysElapsed: 4, isViolation: false }));
    });
    await screen.findAllByText(/Created Aug 1, 2026/);
    // A's engine analysis arrives last. Written, it would date B's clock from A's creation.
    await act(async () => {
      engineReads.get(requestA.requestId)!(analysis({ requestId: requestA.requestId, createdAt: '2026-01-05T00:00:00Z', determinationAt: '2026-01-20T00:00:00Z', determinationBasis: 'EARLIEST_RECORDED_ACTIVITY_PROXY', calendarDaysElapsed: 15 }));
    });
    expect(screen.getAllByText(/Created Aug 1, 2026/).length).toBeGreaterThan(0);
    expect(screen.queryAllByText(/Created Jan 5, 2026/)).toHaveLength(0);
  });
});
