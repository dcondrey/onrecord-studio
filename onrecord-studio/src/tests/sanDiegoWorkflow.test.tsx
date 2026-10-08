// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

vi.hoisted(() => {
  const entries = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => void entries.set(key, value),
    removeItem: (key: string) => void entries.delete(key),
    clear: () => entries.clear(),
  } });
});
const bridge = vi.hoisted(() => ({
  calls: [] as { command: string; args: Record<string, unknown> }[],
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
  hasNewDocument: false,
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: (name: string, handler: (event: { payload: unknown }) => void) => {
  bridge.listeners.set(name, handler);
  return Promise.resolve(() => bridge.listeners.delete(name));
} }));
vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: async (command: string, args: Record<string, unknown>) => {
    bridge.calls.push({ command, args });
    if (command === 'query_audit_records_page') return { nextCursor: null, total: 1, records: [{
      requestId: 42, portalKey: 'sandiego', agencyId: 7, agencyName: 'Police Department',
      title: 'San Diego surveillance policy', descriptionPreview: 'Retention and review requirements',
      status: 'closed', createdAt: '2026-09-01T00:00:00Z', closedAt: '2026-09-02T00:00:00Z',
      documentCount: bridge.hasNewDocument ? 2 : 1, integrityFlags: [],
    }] };
    if (command === 'list_request_documents') return bridge.hasNewDocument
      ? [documentSummary(1, 'policy.txt'), documentSummary(2, 'new-response.txt')]
      : [documentSummary(1, 'policy.txt')];
    if (command === 'get_document_text_chunk') {
      const offset = Number(args.characterOffset);
      const text = offset === 0 ? 'FIRST CHUNK ' + 'x'.repeat(99988) : 'FINAL CHUNK: review and retention';
      return { documentId: 1, portalKey: 'sandiego', characterOffset: offset, returnedCharacterCount: text.length, totalCharacterCount: 100032, nextCharacterOffset: offset === 0 ? 100000 : null, maskedFindingCount: 0, maskedText: text };
    }
    if (command === 'verify_document_provenance') throw new Error('No fixture provenance');
    if (command === 'get_request_statutory_extension') return null;
    if (command === 'get_affiliation_disclosure') return null;
    if (command === 'get_meeting_calendar') return { today: '2026-09-13', meetings: [], postingCounts: { agendaFilesWithBothTimes: 0, under72HoursBeforeStart: 0, afterMeetingStart: 0 }, limitations: [] };
    if (command === 'get_ocr_capability') return { engine: 'fixture', engineAvailable: false, engineVersion: null, languagePacks: [], languageProbeComplete: true, pdfRenderer: null, pdfRendererAvailable: false, pdfRendererVersion: null, imageOcrAvailable: false, pdfOcrAvailable: false, diagnostics: [] };
    if (command === 'start_data_source') return {
      source: { portalKey: 'sd-policies', displayName: 'SDPD policies', canonicalUrl: args.url, originUrl: 'https://www.sandiego.gov', apiLayout: {}, verifiedAt: '2026-09-13T00:00:00Z', sourceKind: 'WEB_CRAWL' },
      syncRunId: 'test-run', detectedKind: 'WEB_CRAWL', usedSavedGuidance: false,
    };
    return [];
  },
}));

import App from '../App';
import AgencySelector from '../components/filters/AgencySelector';
import { useAppStore } from '../store/useAppStore';

/** The per-source tools live behind the composer's "Source options" menu. */
function sourceTool(name: string) {
  fireEvent.click(screen.getByRole('button', { name: 'Source options' }));
  return screen.getByRole('menuitem', { name });
}

function documentSummary(documentId: number, fileName: string) {
  return { documentId, portalKey: 'sandiego', fileName, fileSize: 100032, contentType: 'text/plain', visibility: 'public', downloadStatus: 'DOWNLOADED', rawFileSha256: null, extractedTextSha256: null, extractionStatus: 'SEARCHABLE', extractionFormat: 'text', extractionDeferredReason: null };
}

async function openRequest() {
  act(() => useAppStore.getState().setWorkspaceView('evidence'));
  fireEvent.click(await screen.findByRole('button', { name: /San Diego surveillance policy/ }));
  await screen.findByRole('button', { name: /^policy.txt/ });
}

beforeEach(() => {
  bridge.calls.length = 0;
  bridge.listeners.clear();
  bridge.hasNewDocument = false;
  window.history.replaceState(null, '', '/');
  vi.spyOn(HTMLElement.prototype, 'getClientRects').mockReturnValue({ length: 1 } as DOMRectList);
  window.matchMedia = vi.fn().mockImplementation(() => ({ matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() }));
  useAppStore.setState(useAppStore.getInitialState(), true);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('San Diego research workflow', () => {
  it('selects a corrected budget PDF with its proposed status and single-document acquisition scope', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('Browse sources and add records'));
    const desk = within(await screen.findByRole('region', { name: 'San Diego research desk' }));
    fireEvent.change(desk.getByLabelText('Evidence type'), { target: { value: 'Budget proposal' } });
    fireEvent.change(desk.getByLabelText('Find a regional source'), { target: { value: 'FY2026' } });
    const card = desk.getByText('SDHC FY2026 proposed budget — City format').closest('article')!;
    expect(desk.queryByText('FY2026 HSSD capital improvements')).toBeNull();
    fireEvent.click(within(card).getByRole('button', { name: 'Choose source' }));
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() => expect(bridge.calls.some((call) => call.command === 'start_data_source')).toBe(true));
    const args = bridge.calls.find((call) => call.command === 'start_data_source')!.args;
    expect(args.url).toBe('https://sdhc.org/wp-content/uploads/2025/01/FY26-Budget-City-Format.pdf');
    expect(args.crawlPolicy).toMatchObject({ maximumPages: 1, maximumDepth: 0, maximumResources: 1 });
  });

  it('stages the SDPD source and applies its scoped acquisition recipe when Add is clicked', async () => {
    render(<App />);
    fireEvent.click(screen.getByText('Browse sources and add records'));
    const description = await screen.findByText('Find the written policies, procedures, department orders, and training documents behind a police practice.');
    const card = description.closest('article')!;
    fireEvent.click(within(card).getByRole('button', { name: 'Choose source' }));
    expect(bridge.calls.some((call) => call.command === 'start_data_source')).toBe(false);
    expect((screen.getByLabelText('Public data source URL') as HTMLInputElement).value).toBe('https://www.sandiego.gov/police/data-transparency/policies-procedures');
    fireEvent.click(sourceTool('Crawl settings'));
    expect((screen.getByLabelText(/^Page limit/) as HTMLInputElement).value).toBe('100');
    expect((screen.getByLabelText(/^Crawl depth/) as HTMLInputElement).value).toBe('4');
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() => expect(bridge.calls.some((call) => call.command === 'start_data_source')).toBe(true));
    const args = bridge.calls.find((call) => call.command === 'start_data_source')!.args;
    expect(args.crawlPolicy).toMatchObject({ maximumPages: 100, followSupportedExternalPlatforms: false });
    expect(args.guidance).toMatchObject({ reuseForPublisherPattern: false, allowedHostnames: ['www.sandiego.gov'], includeUrlPrefixes: ['https://www.sandiego.gov/police/data-transparency/policies-procedures', 'https://www.sandiego.gov/sites/default/files/'] });
  });

  it('keeps a department tied to its source when agency IDs collide', () => {
    const profile = { agencyId: 7, totalRequestsProcessed: 1, totalClosed: 1, validLatencySampleSize: 1, excludedInvalidLifetimeCount: 0, averageClosureDays: 1, globalMeanClosureDays: 1, globalStddevClosureDays: null, performanceZScore: null, complianceIndicator: 'Compliant' as const, anomalyCount: 0, statutoryViolationCount: 0, statutoryViolationRate: 0, statutoryDeadlinePassedCount: 0, suspiciousFastClosureCount: 0, totalDocumentsReleased: 1, keywordFrequencyMap: {} };
    useAppStore.setState({ agencyProfiles: [
      { ...profile, agencyName: 'Police Department', portalSubdomain: 'sandiego' },
      { ...profile, agencyName: 'Public Works', portalSubdomain: 'other-source' },
    ] });
    render(<AgencySelector />);
    const department = screen.getByRole('combobox', { name: 'Department' });
    expect(screen.getAllByRole('option')).toHaveLength(3);
    fireEvent.change(department, { target: { value: JSON.stringify(['sandiego', 7]) } });
    expect(useAppStore.getState().activePortalKey).toBe('sandiego');
    expect(useAppStore.getState().selectedAgencyId).toBe(7);
    expect(screen.queryByRole('option', { name: /Public Works/ })).toBeNull();
  });

  it('refreshes an open request after ingestion without closing its document', async () => {
    render(<App />);
    await openRequest();
    fireEvent.click(screen.getByRole('button', { name: /policy.txt.*Searchable text/ }));
    await screen.findByRole('dialog', { name: 'policy.txt' });
    bridge.hasNewDocument = true;
    act(() => bridge.listeners.get('ingestion-telemetry')!({ payload: {
      protocolVersion: 1, sequence: '1', type: 'documentCompleted', payload: { portalKey: 'sandiego', documentId: 2, bytesSaved: 50 },
    } }));
    await waitFor(() => expect(screen.getByText('new-response.txt')).toBeTruthy(), { timeout: 2000 });
    expect(screen.getByRole('dialog', { name: 'policy.txt' })).toBeTruthy();
  });

  it('shows the final chunk after paging and permits returning to the previous chunk', async () => {
    render(<App />);
    await openRequest();
    fireEvent.click(screen.getByRole('button', { name: /policy.txt.*Searchable text/ }));
    await screen.findByRole('dialog', { name: 'policy.txt' });
    fireEvent.click(screen.getByRole('button', { name: 'Next text page' }));
    fireEvent.click(screen.getByRole('button', { name: 'Next chunk' }));
    expect(await screen.findByText('FINAL CHUNK: review and retention')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Previous chunk' }));
    await waitFor(() => expect(screen.getByText(/^FIRST CHUNK/)).toBeTruthy());
  });

  it('opens a linked document at its offset on a fresh launch', async () => {
    window.history.replaceState(null, '', '/#portal=sandiego&request=42&document=1&offset=100000');
    render(<App />);
    expect(await screen.findByText('FINAL CHUNK: review and retention')).toBeTruthy();
    expect(useAppStore.getState().workspaceView).toBe('evidence');
    expect(screen.getByRole('dialog', { name: 'policy.txt' })).toBeTruthy();
  });
});
