// @vitest-environment jsdom
//
// A query failure is absence of a result, not a negative result. These tests keep the evidence and
// legal surfaces from converting bridge failures into "no matches" or "no findings" claims.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';

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

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    // A request with no recorded statutory extension. The command returns the record or
    // null, never a collection, and the panel reads its fields.
    if (command === 'get_request_statutory_extension') return Promise.resolve(null);
    if (command === 'query_audit_records_page') return Promise.reject(new Error('database busy'));
    if (command === 'list_request_legal_findings') {
      return Promise.reject(new Error('legal query failed'));
    }
    return Promise.resolve([]);
  },
}));

import MainLayout from '../components/dashboard/MainLayout';
import SearchFilter from '../components/filters/SearchFilter';
import LegalCompliancePanel from '../components/legal/LegalCompliancePanel';
import { useAppStore } from '../store/useAppStore';
import type { RequestSearchResult } from '../types';

const record: RequestSearchResult = {
  requestId: 42,
  portalKey: 'example-gov',
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

describe('evidence error states', () => {
  beforeEach(() => {
    useAppStore.setState({
      workspaceView: 'evidence',
      activePortalKey: null,
      searchTerm: '',
      selectedAgencyId: null,
      selectedStatus: null,
      selectedPolicyDeviation: null,
      filteredResults: [],
      searchError: null,
      selectedRequestId: null,
      selectedRequestPortalKey: null,
      activeLegalFindings: [],
      agencyProfiles: [],
    });
  });

  afterEach(cleanup);

  it('retains the last successful evidence result set when a later query fails', async () => {
    useAppStore.setState({ filteredResults: [record] });
    render(<SearchFilter />);

    expect((await screen.findByRole('alert')).textContent).toContain(
      'showing 1 results from the last successful query',
    );
    expect(useAppStore.getState().filteredResults).toEqual([record]);
    expect(useAppStore.getState().searchError).toContain('database busy');
  });

  it('does not label an unavailable empty query as no matching evidence', async () => {
    useAppStore.setState({ searchError: 'database busy' });
    render(<MainLayout />);

    expect(await screen.findByRole('heading', { name: 'Search unavailable' })).toBeTruthy();
    expect(screen.queryByRole('heading', { name: 'No matching evidence' })).toBeNull();
  });

  it('does not report no active legal findings when the findings query failed', async () => {
    render(<LegalCompliancePanel record={record} onOpenLibrary={() => undefined} />);

    expect(await screen.findByText('Legal findings unavailable')).toBeTruthy();
    expect(screen.queryByText('No active findings recorded')).toBeNull();
  });

  it('does not attribute the previous record findings to a record whose load failed', async () => {
    useAppStore.setState({
      activeLegalFindings: [{
        violationId: 'old-record-finding',
        requestId: 7,
        portalKey: 'other-gov',
        matchedStatuteId: 'statute-1',
        matchedVersionId: 'version-1',
        mandateId: null,
        statuteCitation: 'Old citation',
        authorityTitle: 'Old authority',
        mandateTitle: null,
        classification: 'OLD_RECORD_CLASSIFICATION',
        calculatedVarianceMetric: null,
        varianceDays: null,
        confidenceBasisPoints: 9_000,
        evidenceSummary: 'Belongs to another record',
        supportingDocumentIds: [],
        missingEvidencePhrases: [],
        reviewStatus: 'OPEN',
        firstDetectedAt: '2026-07-01T00:00:00Z',
        evaluatedAt: '2026-07-01T00:00:00Z',
      }],
    });

    render(<LegalCompliancePanel record={record} onOpenLibrary={() => undefined} />);

    expect(await screen.findByText('Legal findings unavailable')).toBeTruthy();
    expect(screen.queryByText('Old Record Classification')).toBeNull();
    expect(useAppStore.getState().activeLegalFindings).toEqual([]);
  });
});
