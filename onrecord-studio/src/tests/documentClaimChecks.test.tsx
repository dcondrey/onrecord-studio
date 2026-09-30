// @vitest-environment jsdom
//
// The "Check figures" panel states, for one document, which figures were checked against the
// imported tables and with what result. A check requested for document A that resolves after the
// viewer moved to document B would publish A's figures as statements about B.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import DocumentViewer from '../components/documents/DocumentViewer';
import type { DocumentClaimChecks } from '../types';

const bridge = vi.hoisted(() => ({
  responses: new Map<number, DocumentClaimChecks>(),
  gates: new Map<number, Promise<void>>(),
  packGates: new Map<number, Promise<void>>(),
  packRequests: [] as number[],
  downloads: [] as string[],
}));

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { documentId?: number }) => {
    if (command === 'get_document_entity_truncations') return Promise.resolve([]);
    if (command === 'export_claim_query_pack') {
      const documentId = args?.documentId ?? 0;
      bridge.packRequests.push(documentId);
      const pack = { documentId, portalKey: 'example', generatedAt: '2026-09-30T00:00:00Z', checks: [], queries: [], datasets: [], truncated: false };
      const gate = bridge.packGates.get(documentId);
      return gate ? gate.then(() => pack) : Promise.resolve(pack);
    }
    if (command === 'list_document_claim_checks') {
      const documentId = args?.documentId ?? 0;
      const answer = bridge.responses.get(documentId) ?? { documentId, portalKey: 'example', checks: [], truncated: false };
      const gate = bridge.gates.get(documentId);
      return gate ? gate.then(() => answer) : Promise.resolve(answer);
    }
    return Promise.reject(new Error(`unexpected command ${command}`));
  },
}));

const viewer = (documentId: number) => (
  <DocumentViewer documentId={documentId} portalKey="example" fileName="memo.pdf" textContent="" fileSizeFormatted="1 KB" originalAvailable={false} />
);

function checksFor(documentId: number): DocumentClaimChecks {
  const scope = { fiscalYear: 2023, fund: '300000', department: null, basis: 'adopted budget' };
  return {
    documentId,
    portalKey: 'example',
    truncated: false,
    checks: [
      { claimText: 'The adopted budget of $1.2 billion', kindLabel: 'stated total, adopted budget', characterStart: 0, characterEnd: 34, scope, result: { outcome: 'consistent', observed: '$1,187,654,321.00', band: '±$50000000.00', queryIds: ['q1'] } },
      { claimText: 'Spending rose 12% year over year', kindLabel: 'percent change, actual expenditures', characterStart: 40, characterEnd: 72, scope, result: { outcome: 'mismatch', claimed: '+12.00%', observed: '+3.10%', difference: '8.90 pt', band: '±0.50 pt', queryIds: ['q2', 'q3'] } },
      { claimText: 'The budget was balanced', kindLabel: 'balanced budget', characterStart: 80, characterEnd: 103, scope: { ...scope, basis: null }, result: { outcome: 'unreconciled', reason: 'unsupported claim', detail: 'no mapped role distinguishes revenue from expenditure' } },
    ],
  };
}

afterEach(() => {
  cleanup();
  bridge.responses.clear();
  bridge.packGates.clear();
  bridge.packRequests = [];
  bridge.downloads = [];
  bridge.gates.clear();
});

describe('document claim checks', () => {
  it('renders each outcome as consistent, mismatch, or not checked with its reason', async () => {
    bridge.responses.set(7, checksFor(7));
    render(viewer(7));
    fireEvent.click(screen.getByRole('button', { name: 'Check figures' }));
    await screen.findByText(/Consistent: the data shows \$1,187,654,321\.00, within ±\$50000000\.00/);
    expect(screen.getByText(/Mismatch: states \+12\.00%; the data shows \+3\.10%, a difference of 8\.90 pt, outside ±0\.50 pt\. Query id\(s\) q2, q3\./)).toBeTruthy();
    expect(screen.getByText(/Not checked \(unsupported claim\): no mapped role distinguishes revenue from expenditure/)).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Recheck figures' })).toBeTruthy();
  });

  it('offers a query pack only once checks exist and names the file after the document it was built for', async () => {
    bridge.responses.set(7, checksFor(7));
    const createObjectURL = vi.fn(() => 'blob:pack');
    const revokeObjectURL = vi.fn();
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: createObjectURL });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: revokeObjectURL });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      bridge.downloads.push(this.download);
    });
    render(viewer(7));
    expect(screen.queryByRole('button', { name: 'Download query pack' })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Check figures' }));
    await screen.findByRole('button', { name: 'Download query pack' });
    fireEvent.click(screen.getByRole('button', { name: 'Download query pack' }));
    await waitFor(() => expect(bridge.downloads).toEqual(['claim-query-pack-example-7.json']));
    expect(bridge.packRequests).toEqual([7]);
    click.mockRestore();
  });

  it('does not save a query pack that resolves after the viewer moved to another document', async () => {
    bridge.responses.set(7, checksFor(7));
    let release = () => {};
    bridge.packGates.set(7, new Promise<void>((resolve) => { release = () => resolve(); }));
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      bridge.downloads.push(this.download);
    });
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: () => 'blob:pack' });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: () => {} });
    const view = render(viewer(7));
    fireEvent.click(screen.getByRole('button', { name: 'Check figures' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Download query pack' }));
    view.rerender(viewer(8));
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(bridge.downloads).toEqual([]);
    click.mockRestore();
  });

  it('discards checks that resolve after the viewer moved to another document', async () => {
    bridge.responses.set(7, checksFor(7));
    let release = () => {};
    bridge.gates.set(7, new Promise<void>((resolve) => { release = () => resolve(); }));
    const view = render(viewer(7));
    fireEvent.click(screen.getByRole('button', { name: 'Check figures' }));
    await screen.findByText(/Checking the figures in this document/);
    view.rerender(viewer(8));
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText(/Consistent: the data shows/)).toBeNull();
    expect(screen.queryByText(/Checking the figures in this document/)).toBeNull();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Check figures' })).toBeTruthy());
  });
});
