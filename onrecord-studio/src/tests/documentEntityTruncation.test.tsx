// @vitest-environment jsdom
//
// A truncation notice states that this document's entity list is incomplete. One requested for
// document A that resolves after the viewer moved to document B would publish A's gap as a
// statement about B.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import DocumentViewer from '../components/documents/DocumentViewer';

const bridge = vi.hoisted(() => ({
  kinds: new Map<number, string[]>(),
  gates: new Map<number, Promise<void>>(),
}));

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { documentId?: number }) => {
    if (command === 'get_document_entity_truncations') {
      const documentId = args?.documentId ?? 0;
      const answer = bridge.kinds.get(documentId) ?? [];
      const gate = bridge.gates.get(documentId);
      return gate ? gate.then(() => answer) : Promise.resolve(answer);
    }
    if (command === 'describe_document_origin') return Promise.resolve({ origin: 'COLLECTED_FROM_SOURCE', heldRelation: null });
    return Promise.reject(new Error(`unexpected command ${command}`));
  },
}));

const viewer = (documentId: number) => (
  <DocumentViewer documentId={documentId} portalKey="example" fileName="a.pdf" textContent="" fileSizeFormatted="1 KB" originalAvailable={false} />
);

afterEach(() => {
  cleanup();
  bridge.kinds.clear();
  bridge.gates.clear();
});

describe('entity truncation notice', () => {
  it('states an incomplete entity list only when a truncation was recorded', async () => {
    bridge.kinds.set(7, ['MONEY']);
    const view = render(viewer(7));
    await waitFor(() => expect(screen.getByRole('status').textContent).toContain('money'));
    view.rerender(viewer(8));
    await waitFor(() => expect(screen.queryByText(/Entity extraction stopped early/)).toBeNull());
  });

  it('discards a truncation that resolves after the viewer moved on', async () => {
    bridge.kinds.set(7, ['MONEY']);
    let release = () => {};
    bridge.gates.set(7, new Promise<void>((resolve) => { release = () => resolve(); }));
    const view = render(viewer(7));
    view.rerender(viewer(8));
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText(/Entity extraction stopped early/)).toBeNull();
  });
});
