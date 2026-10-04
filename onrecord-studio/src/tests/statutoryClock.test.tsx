// @vitest-environment jsdom
//
// The statutory clock a request is measured under is the source's, and a finding is only
// readable with the rule and the basis of its determination date beside it. These tests pin that
// the panel states the rule with its citation, says when no clock applies rather than assuming
// one, labels a proxied determination as a proxy, and never shows one source's clock under
// another's request.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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

const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
const jurisdictionResponses: Array<{ portalKey: string; resolve: (value: unknown) => void }> = [];
let findings: unknown[] = [];

const citation = (section: string) => ({ section, url: `https://example.gov/${encodeURIComponent(section)}` });
const RULES = [
  {
    id: 'CA_CPRA_DETERMINATION',
    jurisdiction: 'CA_CPRA',
    title: 'California Public Records Act determination deadline',
    actName: 'the California Public Records Act',
    statuteId: 'CPRA_7922.535',
    start: { event: 'receipt of the request', latestAfterReceipt: null, citation: citation('Cal. Gov. Code § 7922.535(a)') },
    stopEvent: 'determination',
    initial: { days: 10, kind: 'CALENDAR_CARRIED_PAST_CLOSED_DAYS' },
    initialCitation: citation('California Government Code § 7922.535'),
    calendar: 'CALIFORNIA_JUDICIAL',
    calendarCitation: citation('Cal. Code Civ. Proc. §§ 12, 12a, 135'),
    extension: { maximum: { days: 14, kind: 'CALENDAR_CARRIED_PAST_CLOSED_DAYS' }, grounds: ['VOLUMINOUS_RECORDS', 'DATA_COMPILATION'], citation: citation('Cal. Gov. Code § 7922.535(b)-(c)') },
    unrecordedClockStops: null,
    notes: [],
  },
  {
    id: 'US_FOIA_DETERMINATION',
    jurisdiction: 'US_FOIA',
    title: 'Federal FOIA determination deadline',
    actName: 'the Freedom of Information Act, 5 U.S.C. § 552',
    statuteId: 'FOIA_552_A_6',
    start: { event: 'receipt by the appropriate component', latestAfterReceipt: { days: 10, kind: 'WORKING_DAYS' }, citation: citation('5 U.S.C. § 552(a)(6)(A)') },
    stopEvent: 'determination',
    initial: { days: 20, kind: 'WORKING_DAYS' },
    initialCitation: citation('5 U.S.C. § 552(a)(6)(A)(i)'),
    calendar: 'FEDERAL_LEGAL_PUBLIC',
    calendarCitation: citation('5 U.S.C. § 6103'),
    extension: { maximum: { days: 10, kind: 'WORKING_DAYS' }, grounds: ['VOLUMINOUS_RECORDS'], citation: citation('5 U.S.C. § 552(a)(6)(B)(i), (iii)') },
    unrecordedClockStops: citation('5 U.S.C. § 552(a)(6)(A)(ii)(I)-(II)'),
    notes: [],
  },
];

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'list_statutory_clock_rules') return Promise.resolve(RULES);
    if (command === 'get_source_statutory_jurisdiction') {
      return new Promise((resolve) => {
        jurisdictionResponses.push({ portalKey: String(args.portalKey), resolve });
      });
    }
    if (command === 'get_request_statutory_extension') return Promise.resolve(null);
    if (command === 'list_request_legal_findings') return Promise.resolve(findings);
    if (command === 'set_source_statutory_jurisdiction') return Promise.resolve(1);
    if (command === 'evaluate_request_legal_compliance') {
      return Promise.resolve({ requestId: 1, portalKey: 'x', evaluatedMandateCount: 0, applicableMandateCount: 0, corpusCharacterCount: 0, evaluationComplete: true, warnings: [], findings: [] });
    }
    return Promise.resolve([]);
  },
}));

import LegalCompliancePanel from '../components/legal/LegalCompliancePanel';
import { useAppStore } from '../store/useAppStore';
import type { RequestSearchResult } from '../types';

function requestRecord(portalKey: string): RequestSearchResult {
  return {
    requestId: 42,
    portalKey,
    agencyId: 7,
    agencyName: 'Public Works',
    title: 'Street maintenance records',
    descriptionPreview: 'Responsive records',
    status: 'closed',
    createdAt: '2026-08-01T00:00:00Z',
    closedAt: '2026-08-10T00:00:00Z',
    documentCount: 2,
    integrityFlags: [],
  };
}

describe('statutory clock in the legal compliance panel', () => {
  beforeEach(() => {
    calls.length = 0;
    jurisdictionResponses.length = 0;
    findings = [];
    useAppStore.setState({ activeLegalFindings: [], legalAuthorities: [] });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it('states the rule and its citations, and labels a proxied determination as a proxy', async () => {
    findings = [{
      violationId: 'v1', requestId: 42, portalKey: 'federal', matchedStatuteId: 'FOIA_552_A_6',
      matchedVersionId: 'builtin-foia-552-a-6-2026', mandateId: null,
      statuteCitation: '5 U.S.C. § 552(a)(6)(A)(i)', authorityTitle: 'FOIA', mandateTitle: null,
      classification: 'STATUTORY_LATENCY_VIOLATION', calculatedVarianceMetric: null, varianceDays: 3,
      confidenceBasisPoints: 8_750, evidenceSummary: 'Request 42: 45 calendar days', supportingDocumentIds: [],
      missingEvidencePhrases: [], reviewStatus: 'OPEN', firstDetectedAt: '2026-09-01T00:00:00Z',
      evaluatedAt: '2026-09-01T00:00:00Z', clockRuleId: 'US_FOIA_DETERMINATION',
      determinationBasis: 'EARLIEST_RECORDED_ACTIVITY_PROXY',
    }];
    render(<LegalCompliancePanel record={requestRecord('federal')} onOpenLibrary={() => undefined} />);
    await waitFor(() => expect(jurisdictionResponses).toHaveLength(1));
    jurisdictionResponses[0].resolve('US_FOIA');

    expect((await screen.findAllByText('Federal FOIA determination deadline')).length).toBeGreaterThan(0);
    const link = screen.getAllByRole('link', { name: '5 U.S.C. § 552(a)(6)(A)(i)' })[0];
    expect(link.getAttribute('href')).toContain('https://');
    expect(screen.getAllByText(/may also pause under/).length).toBeGreaterThan(0);
    expect(await screen.findByText(/Determination: proxy\./)).toBeTruthy();
    // The extension reasons offered are the source rule's grounds, not CPRA's.
    fireEvent.click(screen.getByRole('button', { name: 'Record notice' }));
    expect(screen.queryByRole('option', { name: 'Data Compilation' })).toBeNull();
    expect(screen.getByRole('option', { name: 'Voluminous Records' })).toBeTruthy();
  });

  it('says no deadline is evaluated when the source names no jurisdiction, and records a choice', async () => {
    render(<LegalCompliancePanel record={requestRecord('unknown')} onOpenLibrary={() => undefined} />);
    await waitFor(() => expect(jurisdictionResponses).toHaveLength(1));
    jurisdictionResponses[0].resolve(null);

    expect(await screen.findByText(/No jurisdiction is set for this source/)).toBeTruthy();
    expect(screen.queryByText('California Public Records Act determination deadline', { selector: 'div' })).toBeNull();

    fireEvent.change(screen.getByRole('combobox', { name: 'Statute for this source' }), {
      target: { value: 'NONE' },
    });
    await waitFor(() =>
      expect(calls.find((call) => call.command === 'set_source_statutory_jurisdiction')?.args).toEqual({
        portalKey: 'unknown',
        jurisdiction: null,
      }),
    );
    expect(await screen.findByText(/marked as under no modeled statute/)).toBeTruthy();
  });

  it('never shows the previous source clock under a request from another source', async () => {
    const view = render(<LegalCompliancePanel record={requestRecord('federal')} onOpenLibrary={() => undefined} />);
    await waitFor(() => expect(jurisdictionResponses).toHaveLength(1));
    view.rerender(<LegalCompliancePanel record={requestRecord('unknown')} onOpenLibrary={() => undefined} />);
    await waitFor(() => expect(jurisdictionResponses).toHaveLength(2));

    jurisdictionResponses[1].resolve(null);
    expect(await screen.findByText(/No jurisdiction is set for this source/)).toBeTruthy();
    // The first source's answer arrives late; it must not replace the second source's.
    jurisdictionResponses[0].resolve('US_FOIA');
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.getByText(/No jurisdiction is set for this source/)).toBeTruthy();
    expect(screen.queryByText('Federal FOIA determination deadline', { selector: 'div' })).toBeNull();
  });
});
