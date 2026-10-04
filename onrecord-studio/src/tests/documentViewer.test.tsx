// @vitest-environment jsdom
//
// The residual-evidence audit panel states, for one document, what forensic residue the
// collectors found in its original file. An audit requested for document A that resolves after
// the viewer moved to document B would publish A's findings as evidence about B. This is the one
// `useLatestRequestGate` instance in DocumentViewer.tsx (`residualAuditGate`) that had no test
// proving its guard (see CLAUDE.md's async-identity-gate rule).
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import DocumentViewer from '../components/documents/DocumentViewer';
import type { EvidenceBundle } from '../types/investigation';
import type { DocumentSourceHistory } from '../types';

const bridge = vi.hoisted(() => ({
  bundles: new Map<number, EvidenceBundle>(),
  gates: new Map<number, Promise<void>>(),
  historyGates: new Map<number, Promise<void>>(),
}));

function historyOf(documentId: number): DocumentSourceHistory {
  return {
    checks: [{
      checkId: documentId,
      runId: 'run',
      portalKey: 'example',
      documentId,
      requestedUrl: `https://records.example.gov/document-${documentId}`,
      checkedAt: '2026-10-02T00:00:00Z',
      outcome: 'GONE',
      httpStatus: 404,
      collectedSha256: '1'.repeat(64),
      observedSha256: null,
      detail: null,
      acquisitionId: null,
      observedCopyRetained: false,
    }],
    acquisitions: [],
  };
}

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { documentId?: number }) => {
    if (command === 'verify_document_provenance') return Promise.resolve(null);
    if (command === 'get_document_entity_truncations') return Promise.resolve([]);
    if (command === 'list_document_source_history') {
      const documentId = args?.documentId ?? 0;
      const gate = bridge.historyGates.get(documentId);
      return gate ? gate.then(() => historyOf(documentId)) : Promise.resolve(historyOf(documentId));
    }
    if (command === 'audit_document_residual_evidence') {
      const documentId = args?.documentId ?? 0;
      const answer = bridge.bundles.get(documentId) ?? { findings: [], warnings: [], coverageComplete: true };
      const gate = bridge.gates.get(documentId);
      return gate ? gate.then(() => answer) : Promise.resolve(answer);
    }
    return Promise.reject(new Error(`unexpected command ${command}`));
  },
}));

function bundleWithFinding(documentId: number): EvidenceBundle {
  return {
    findings: [{
      class: 'preserved',
      confidence: 'strong',
      kind: `residual-from-document-${documentId}`,
      locator: { containerPart: null, objectReference: null, byteOffset: null, pageNumber: null, bounds: null, xmlPath: null },
      valueSha256: null,
      maskedPreview: null,
      explanation: `finding belonging to document ${documentId}`,
    }],
    warnings: [],
    coverageComplete: true,
  };
}

const viewer = (documentId: number) => (
  <DocumentViewer documentId={documentId} portalKey="example" fileName="memo.pdf" textContent="" fileSizeFormatted="1 KB" originalAvailable />
);

afterEach(() => {
  cleanup();
  bridge.bundles.clear();
  bridge.gates.clear();
  bridge.historyGates.clear();
});

describe('document source history', () => {
  it('discards a source history that resolves after the viewer moved to another document', async () => {
    let release = () => {};
    bridge.historyGates.set(7, new Promise<void>((resolve) => { release = () => resolve(); }));

    const view = render(viewer(7));
    fireEvent.click(screen.getByRole('button', { name: 'Source history' }));
    await screen.findByText('Loading source history…');

    view.rerender(viewer(8));
    fireEvent.click(screen.getByRole('button', { name: 'Source history' }));
    await screen.findByText('https://records.example.gov/document-8');
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));

    // Document 7's fetch record must never be listed as document 8's.
    expect(screen.queryByText('https://records.example.gov/document-7')).toBeNull();
    expect(screen.getByText('https://records.example.gov/document-8')).toBeTruthy();
  });
});

describe('document residual-evidence audit', () => {
  it('discards a residual-evidence audit that resolves after the viewer moved to another document', async () => {
    bridge.bundles.set(7, bundleWithFinding(7));
    let release = () => {};
    bridge.gates.set(7, new Promise<void>((resolve) => { release = () => resolve(); }));

    const view = render(viewer(7));
    fireEvent.click(screen.getByRole('button', { name: 'Audit residual evidence' }));
    await screen.findByRole('button', { name: 'Auditing residuals…' });

    view.rerender(viewer(8));
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));

    // Document 7's finding must never appear under document 8, and the button must reflect that
    // no audit for document 8 has run -- not a stale "Reaudit" label borrowed from document 7.
    expect(screen.queryByText('finding belonging to document 7')).toBeNull();
    expect(screen.queryByLabelText('Residual evidence audit result')).toBeNull();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Audit residual evidence' })).toBeTruthy());
  });
});
