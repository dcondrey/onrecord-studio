// @vitest-environment jsdom
//
// A dossier assembled for finding A that resolves after the reader opened finding B would present
// A's evidence chain and gaps as statements about B.
import { afterEach, describe, expect, it, vi } from 'vitest';
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
  gates: new Map<string, Promise<void>>(),
}));

// The tail `analysis/service_leads.rs` appends to a table-derived lead, in its stored form.
const TABLE_LEAD_EXPLANATION = vi.hoisted(() => `Open reports exceed the budgeted crew capacity. Method: grouped by category. Worth asking: which categories wait longest?\n\nTables read:\n1. 'Get It Done open reports' (https://example.test/open.csv), retrieved 2026-10-04, sha256 ${'ab'.repeat(32)}\n2. 'City operating budget' (no source address recorded), retrieved 2026-10-03, sha256 ${'cd'.repeat(32)}\n\nSQL:\nSELECT category FROM service_backlog`);

function dossier(findingId: string, headline: string) {
  return {
    finding: {
      id: findingId,
      kind: 'CLAIM_CONTRADICTS_DATA',
      severity: 'SERIOUS',
      confidence: { kind: 'MEASURED', basisPoints: 8000 },
      subject: { portalKey: 'sandiego', agencyId: null, requestId: 42, documentId: 9, entityId: null },
      headline,
      explanation: 'The stated total lies outside its rounding band.',
      citations: [{ portalKey: 'sandiego', documentId: 9, characterStart: 10, characterEnd: 40, quote: 'the adopted budget of $1.2 billion', label: null }],
      discoveredAt: '2026-09-30T00:00:00Z',
      lastSeenAt: '2026-09-30T00:00:00Z',
      dismissedAt: null,
      sourceDetector: 'claim_data_mismatch',
    },
    evidence: [{ portalKey: 'sandiego', documentId: 9, fileName: 'budget-memo.pdf', requestId: 42, agencyLabel: 'Finance', rawFileSha256: 'abc123def456abc', characterStart: 10, characterEnd: 40, quote: 'the adopted budget of $1.2 billion', label: null }],
    evidenceTruncated: false,
    related: [],
    relatedTruncated: false,
    falsification: { statement: 'The stated figure refers to a later revision of the data.', recordsToSeek: ['the report the statement drew its figure from'] },
    gaps: [
      { gap: 'notComputed', portalKey: 'sandiego', documentId: 9, reason: 'the document text could not be read: missing' },
      { gap: 'unreconciledClaim', portalKey: 'sandiego', documentId: 9, claimText: 'the adopted budget of $1.2 billion', reason: 'no matching column', detail: 'no imported table names a budget total' },
    ],
    gapsTruncated: false,
    assembledAt: '2026-09-30T01:00:00Z',
  };
}

// A stored lead's figure, keyed by the finding it belongs to.
const FIGURES = vi.hoisted(() => new Map<string, unknown>([
  ['figure-lead', { kind: 'ranked', measure: 'Open reports by service', claim: 'descriptive', caption: null, notes: [], unit: 'count', excluded: [],
    rows: [{ key: 'lights', label: 'Street lights', value: { state: 'value', value: 12 }, subject: true, detail: null }] }],
  ['stale-a', { kind: 'ranked', measure: 'Figure for stale-a', claim: 'descriptive', caption: null, notes: [], unit: 'count', excluded: [],
    rows: [{ key: 'a', label: 'Row of stale-a', value: { state: 'value', value: 3 }, subject: true, detail: null }] }],
]));

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { findingId?: string; filter?: { id?: string } }) => {
    if (command === 'draft_availability_for_finding') return Promise.resolve({ followup: true, appeal: false });
    if (command === 'list_document_provenance_verifications') return Promise.resolve([]);
    if (command === 'list_findings') {
      const id = args?.filter?.id ?? '';
      const answer = FIGURES.has(id) ? [{ id, figure: FIGURES.get(id) }] : [];
      const gate = bridge.gates.get(`figure:${id}`);
      return gate ? gate.then(() => answer) : Promise.resolve(answer);
    }
    if (command !== 'build_lead_dossier') return Promise.reject(new Error(`unexpected ${command}`));
    const id = args?.findingId ?? '';
    const answer = dossier(id, `Headline for ${id}`);
    if (id === 'table-lead' || id === 'figure-lead') answer.finding.explanation = TABLE_LEAD_EXPLANATION;
    const gate = bridge.gates.get(id);
    return gate ? gate.then(() => answer) : Promise.resolve(answer);
  },
  formatNativeError: (reason: unknown) => String(reason),
}));

import LeadDossier from '../components/dashboard/LeadDossier';

afterEach(() => {
  cleanup();
  bridge.gates.clear();
  Object.assign(navigator, { clipboard: undefined });
});

describe('LeadDossier', () => {
  it('renders the evidence chain, falsification test and an uncomputed gap as such', async () => {
    render(<LeadDossier findingId="finding-a" />);
    await screen.findByText('Headline for finding-a');
    expect(screen.getByRole('button', { name: 'budget-memo.pdf' })).toBeTruthy();
    expect(screen.getByText(/Source 1 · Finance/)).toBeTruthy();
    expect(screen.getByText(/later revision of the data/)).toBeTruthy();
    expect(screen.getByText(/figures not examined/)).toBeTruthy();
    expect(screen.queryByText(/Every figure found/)).toBeNull();
    // A document finding reads no table: no tables section, no query.
    expect(screen.queryByText(/Tables read/)).toBeNull();
    expect(screen.queryByLabelText('Query the figures were computed with')).toBeNull();
  });

  it('shows every table a table lead read, with its source, retrieval date and full digest, and the query', async () => {
    render(<LeadDossier findingId="table-lead" />);
    await screen.findByText('Tables read (2)');
    expect(screen.getByText('Get It Done open reports')).toBeTruthy();
    expect(screen.getByText('Source: https://example.test/open.csv')).toBeTruthy();
    expect(screen.getByText('Source: no source address recorded')).toBeTruthy();
    expect(screen.getByText('Retrieved: 2026-10-03')).toBeTruthy();
    expect(screen.getByText(`SHA-256: ${'ab'.repeat(32)}`)).toBeTruthy();
    expect(screen.getByText(`SHA-256: ${'cd'.repeat(32)}`)).toBeTruthy();
    expect(screen.getByLabelText('Query the figures were computed with').textContent).toBe('SELECT category FROM service_backlog');
  });

  it('mounts the drafts panel for the finding below the evidence chain', async () => {
    render(<LeadDossier findingId="finding-a" />);
    await screen.findByText('Headline for finding-a');
    expect(screen.getByRole('region', { name: 'Drafted artifacts' })).toBeTruthy();
    expect(await screen.findByRole('button', { name: /draft records request/i })).toBeTruthy();
  });

  it('offers a request text per gap that quotes the claim and asks only for the records', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(<LeadDossier findingId="finding-a" />);
    await screen.findByText('Headline for finding-a');
    const buttons = screen.getAllByRole('button', { name: 'Copy request text' });
    expect(buttons).toHaveLength(2);
    fireEvent.click(buttons[1]);
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const text = writeText.mock.calls[0][0] as string;
    expect(text).toContain('the adopted budget of $1.2 billion');
    expect(text).toContain('confirmed or refuted');
    expect(text).not.toMatch(/false|incorrect|wrong/i);
    expect((await screen.findByRole('status')).textContent).toBe('Request text copied.');
    expect(document.querySelector('pre')).toBeNull();
  });

  it('shows the request text for selection when the clipboard is unavailable', async () => {
    Object.assign(navigator, { clipboard: undefined });
    render(<LeadDossier findingId="finding-a" />);
    await screen.findByText('Headline for finding-a');
    fireEvent.click(screen.getAllByRole('button', { name: 'Copy request text' })[0]);
    await screen.findByText(/Copying was blocked/);
    const pre = document.querySelector('pre');
    expect(pre?.textContent).toContain('document 9 held under sandiego');
    expect(pre?.textContent).toContain('could not be read: missing');
  });

  it('leads a table lead with its stored figure and keeps the prose citable under Figures as text', async () => {
    render(<LeadDossier findingId="figure-lead" />);
    await screen.findByText('Open reports by service');
    expect(screen.getByRole('button', { name: 'View as table' })).toBeTruthy();
    const prose = screen.getByText('Figures as text').closest('details')!;
    expect(prose.textContent).toContain('Open reports exceed the budgeted crew capacity.');
    expect(prose.textContent).not.toContain('SELECT category');
    expect(screen.getByText('Tables read (2)')).toBeTruthy();
  });

  it('discards a figure that resolves after the reader opened another finding', async () => {
    let releaseA = () => {};
    bridge.gates.set('figure:stale-a', new Promise<void>((resolve) => { releaseA = () => resolve(); }));
    const view = render(<LeadDossier findingId="stale-a" />);
    view.rerender(<LeadDossier findingId="stale-b" />);
    await screen.findByText('Headline for stale-b');
    await act(async () => {
      releaseA();
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() => expect(screen.queryByText('Figure for stale-a')).toBeNull());
    expect(screen.queryByText('Headline for stale-a')).toBeNull();
    expect(screen.queryByText('Figures as text')).toBeNull();
  });

  it('discards a dossier that resolves after the reader opened another finding', async () => {
    let releaseA = () => {};
    bridge.gates.set('finding-a', new Promise<void>((resolve) => { releaseA = () => resolve(); }));
    const view = render(<LeadDossier findingId="finding-a" />);
    view.rerender(<LeadDossier findingId="finding-b" />);
    await screen.findByText('Headline for finding-b');
    await act(async () => {
      releaseA();
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() => expect(screen.queryByText('Headline for finding-a')).toBeNull());
    expect(screen.getByText('Headline for finding-b')).toBeTruthy();
  });
});
