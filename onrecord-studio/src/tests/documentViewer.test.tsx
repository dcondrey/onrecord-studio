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

const bridge = vi.hoisted(() => ({
  bundles: new Map<number, EvidenceBundle>(),
  gates: new Map<number, Promise<void>>(),
  findingGates: new Map<number, Promise<void>>(),
}));

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { documentId?: number; filter?: { documentId?: number; portalKey?: string } }) => {
    if (command === 'list_document_annotations') return Promise.resolve({ annotations: [] });
    if (command === 'list_findings') {
      const documentId = args?.filter?.documentId ?? 0;
      const answer = [{
        id: `finding-${documentId}`,
        headline: `finding about document ${documentId}`,
        citations: [{ portalKey: args?.filter?.portalKey ?? 'example', documentId, characterStart: 10, characterEnd: 20, quote: 'q', label: null }],
      }];
      const gate = bridge.findingGates.get(documentId);
      return gate ? gate.then(() => answer) : Promise.resolve(answer);
    }
    if (command === 'verify_document_provenance') return Promise.resolve(null);
    if (command === 'get_document_entity_truncations') return Promise.resolve([]);
    if (command === 'describe_document_origin') return Promise.resolve({ origin: 'COLLECTED_FROM_SOURCE', heldRelation: null });
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
  bridge.findingGates.clear();
});

describe('document minimap', () => {
  it('discards cited passages read for the previous document when they resolve after the switch', async () => {
    let release = () => {};
    bridge.findingGates.set(7, new Promise<void>((resolve) => { release = () => resolve(); }));
    bridge.findingGates.set(8, new Promise<void>(() => {}));

    const view = render(viewer(7));
    view.rerender(viewer(8));
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));

    // Document 7's citation, written under document 8, would state where 7's passage sits in 8.
    expect(screen.getByTestId('minimap-headline').textContent).not.toContain('cited passage');
    expect(screen.queryByText(/finding about document 7/)).toBeNull();
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
