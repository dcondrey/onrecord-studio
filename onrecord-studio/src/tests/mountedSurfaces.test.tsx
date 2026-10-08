// @vitest-environment jsdom
//
// StatutoryDeadlineAppeal and VerifiableReportExport must be reachable from a real surface, must
// call their command only when clicked, and must be absent outside Tauri.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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

const bridge = vi.hoisted(() => ({
  tauri: true,
  calls: [] as string[],
  flags: [] as string[],
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => bridge.tauri,
  invoke: (command: string) => {
    bridge.calls.push(command);
    if (command === 'query_audit_records_page') return Promise.resolve({ records: [record()], nextCursor: null, total: 1 });
    if (command === 'get_request_statutory_extension') return Promise.resolve(null);
    if (command === 'draft_appeal_from_statutory_deadline') return Promise.reject(new Error('determination date is a proxy'));
    if (command === 'export_verifiable_report') return Promise.reject(new Error('export refused'));
    if (command === 'get_affiliation_disclosure') {
      return Promise.resolve({ status: 'UNSET', sentence: null, affiliations: [], ownPortalKeys: [], ownAgencies: [], ownEntityIds: [] });
    }
    if (command === 'get_profile') return Promise.resolve({ status: 'UNSET', profile: null });
    return Promise.resolve([]);
  },
}));

import MainLayout from '../components/dashboard/MainLayout';
import FindingsInbox from '../components/dashboard/FindingsInbox';
import CommandPalette from '../components/dashboard/CommandPalette';
import { useAppStore } from '../store/useAppStore';
import type { RequestSearchResult } from '../types';

function record(): RequestSearchResult {
  return {
    requestId: 42,
    portalKey: 'example-gov',
    agencyId: 7,
    agencyName: 'Public Works',
    title: 'Record 42',
    descriptionPreview: 'Responsive records',
    status: 'closed',
    createdAt: '2026-08-01T00:00:00Z',
    closedAt: '2026-08-10T00:00:00Z',
    documentCount: 0,
    integrityFlags: bridge.flags,
  };
}

function mountLayout() {
  useAppStore.setState({
    syncingPortalKeys: [],
    workspaceView: 'evidence',
    activePortalKey: null,
    searchTerm: '',
    selectedAgencyId: null,
    selectedStatus: null,
    selectedPolicyDeviation: null,
    filteredResults: [record()],
    searchError: null,
    selectedRequestId: 42,
    selectedRequestPortalKey: 'example-gov',
    activeLegalFindings: [],
    agencyProfiles: [],
  });
  render(<MainLayout />);
}

beforeEach(() => {
  bridge.tauri = true;
  bridge.calls.length = 0;
  bridge.flags = [];
});
afterEach(cleanup);

describe('StatutoryDeadlineAppeal mount', () => {
  it('renders for a counted statutory violation and drafts only on click', async () => {
    bridge.flags = ['STATUTORY_LATENCY_VIOLATION'];
    mountLayout();
    const button = await screen.findByRole('button', { name: 'Draft statutory deadline appeal' });
    expect(bridge.calls).not.toContain('draft_appeal_from_statutory_deadline');
    fireEvent.click(button);
    await waitFor(() => expect(bridge.calls).toContain('draft_appeal_from_statutory_deadline'));
    expect(await screen.findByText(/determination date is a proxy/)).toBeTruthy();
  });

  it('is absent without a statutory violation flag', async () => {
    bridge.flags = ['SUSPICIOUSLY_FAST_CLOSURE'];
    mountLayout();
    await screen.findByText('Generate administrative appeal draft');
    expect(screen.queryByRole('button', { name: 'Draft statutory deadline appeal' })).toBeNull();
  });

  it('is absent outside Tauri', async () => {
    bridge.tauri = false;
    bridge.flags = ['STATUTORY_LATENCY_VIOLATION'];
    mountLayout();
    await screen.findByText('Generate administrative appeal draft');
    expect(screen.queryByRole('button', { name: 'Draft statutory deadline appeal' })).toBeNull();
  });
});

describe('VerifiableReportExport mount', () => {
  it('renders in the findings inbox and exports only on click', async () => {
    render(<FindingsInbox />);
    fireEvent.click(await screen.findByRole('button', { name: 'Export…' }));
    const button = await screen.findByRole('button', { name: 'Export verifiable report', hidden: true });
    expect(bridge.calls).not.toContain('export_verifiable_report');
    fireEvent.click(button);
    await waitFor(() => expect(bridge.calls).toContain('export_verifiable_report'));
  });

  it('is absent outside Tauri', async () => {
    bridge.tauri = false;
    render(<FindingsInbox />);
    await waitFor(() => expect(bridge.calls).toContain('list_findings'));
    expect(screen.queryByRole('button', { name: 'Export verifiable report', hidden: true })).toBeNull();
  });
});

describe('EvidencePacketExport mount', () => {
  it('renders in the findings inbox and exports only on click', async () => {
    render(<FindingsInbox />);
    fireEvent.click(await screen.findByRole('button', { name: 'Export…' }));
    const button = await screen.findByRole('button', { name: 'Export evidence packet', hidden: true });
    expect(bridge.calls).not.toContain('export_evidence_packet');
    fireEvent.click(button);
    await waitFor(() => expect(bridge.calls).toContain('export_evidence_packet'));
  });

  it('is absent outside Tauri', async () => {
    bridge.tauri = false;
    render(<FindingsInbox />);
    await waitFor(() => expect(bridge.calls).toContain('list_findings'));
    expect(screen.queryByRole('button', { name: 'Export evidence packet', hidden: true })).toBeNull();
  });
});

describe('Cmd/Ctrl+6', () => {
  it('opens the Briefing view', () => {
    act(() => useAppStore.setState({ workspaceView: 'overview' }));
    render(<CommandPalette />);
    fireEvent.keyDown(window, { key: '6', metaKey: true, ctrlKey: true });
    expect(useAppStore.getState().workspaceView).toBe('briefing');
  });
});
