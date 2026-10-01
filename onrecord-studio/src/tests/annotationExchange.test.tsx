// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import AnnotationCodebook from '../components/documents/AnnotationCodebook';
import AnnotationExchange from '../components/documents/AnnotationExchange';
import type { AnnotationImportPreview, CodebookImportPreview, DocumentAnnotation } from '../types';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({
  invokeCommand: invoke,
  formatNativeError: (reason: unknown) => (reason instanceof Error ? reason.message : String(reason)),
}));
vi.stubGlobal('__APP_VERSION__', '1.0.0');

afterEach(() => {
  cleanup();
  invoke.mockReset();
});

const entry = (code: string, definition: string) => ({ code, label: `${code} label`, definition });

describe('codebook import', () => {
  const preview: CodebookImportPreview = {
    added: [entry('NEW', 'brand new')],
    identical: 1,
    conflicts: [
      { code: 'FEE', local: entry('FEE', 'my fee definition'), incoming: entry('FEE', 'their fee definition') },
      { code: 'DELAY', local: entry('DELAY', 'my delay definition'), incoming: entry('DELAY', 'their delay definition') },
    ],
  };

  it('shows each conflict and replaces a definition only for a code the user ticks', async () => {
    invoke.mockImplementation((command: string) =>
      command === 'preview_codebook_import'
        ? Promise.resolve(preview)
        : Promise.resolve({ reviewer: null, codebook: [] }),
    );
    render(<AnnotationCodebook settings={{ reviewer: null, codebook: [] }} onChanged={() => {}} />);
    fireEvent.change(screen.getByLabelText('Codebook JSON'), { target: { value: '{"codes":[]}' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview import' }));

    await screen.findByLabelText('Codebook import preview');
    expect(screen.getByText('1 new, 1 identical, 2 with a different definition.')).toBeTruthy();
    expect(screen.getByText(/their fee definition/)).toBeTruthy();
    expect(screen.getByText(/my delay definition/)).toBeTruthy();

    fireEvent.click(screen.getByLabelText('Replace my definition of FEE with the incoming one'));
    fireEvent.click(screen.getByRole('button', { name: 'Apply import' }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith('apply_codebook_import', { payload: '{"codes":[]}', overwriteCodes: ['FEE'] }));
  });

  it('replaces nothing when no conflict is ticked', async () => {
    invoke.mockImplementation((command: string) =>
      command === 'preview_codebook_import' ? Promise.resolve(preview) : Promise.resolve({ reviewer: null, codebook: [] }),
    );
    render(<AnnotationCodebook settings={null} onChanged={() => {}} />);
    fireEvent.change(screen.getByLabelText('Codebook JSON'), { target: { value: '{}' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview import' }));
    await screen.findByLabelText('Codebook import preview');
    expect(screen.getByText('Definitions you leave unchecked are kept as they are.')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Apply import' }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith('apply_codebook_import', { payload: '{}', overwriteCodes: [] }));
  });
});

describe('signed annotation packet import', () => {
  const incoming: DocumentAnnotation = {
    id: 'incoming-annotation-1',
    portalKey: 'sd',
    documentId: 7,
    kind: 'HIGHLIGHT',
    characterOffset: 4,
    quote: 'the quoted passage',
    prefix: '',
    suffix: '',
    textSha256: null,
    contextUnanchored: false,
    note: '',
    tag: null,
    author: 'Colleague',
    status: 'OPEN',
    findingId: null,
    importedFromKey: null,
    createdAt: '2026-01-01T00:00:00Z',
  };
  const preview: AnnotationImportPreview = {
    signerKeyId: 'ab'.repeat(32),
    packetSha256: 'c'.repeat(64),
    exportedAt: '2026-02-02T00:00:00Z',
    appVersion: '1.0.0',
    exporter: 'Colleague',
    portalKey: 'sd',
    documentId: 7,
    packetTextSha256: 'd'.repeat(64),
    localTextSha256: 'd'.repeat(64),
    toAdd: [incoming],
    alreadyPresent: 0,
    conflicts: [],
  };

  const answer = (overrides: Record<string, () => Promise<unknown>> = {}) =>
    invoke.mockImplementation((command: string) => {
      if (command in overrides) return overrides[command]();
      if (command === 'list_trusted_contributors') return Promise.resolve([]);
      if (command === 'get_local_contributor_identity') return Promise.resolve(null);
      if (command === 'preview_annotation_import') return Promise.resolve(preview);
      return Promise.reject(new Error(`unexpected ${command}`));
    });

  const open = () => {
    render(<AnnotationExchange portalKey="sd" documentId={7} hasOwnMarks onImported={vi.fn()} />);
    fireEvent.change(screen.getByLabelText('Packet JSON'), { target: { value: '{"signed":true}' } });
    fireEvent.click(screen.getByRole('button', { name: 'Preview packet' }));
  };

  it('warns that an unlisted signer is unverified before the import can be applied', async () => {
    answer();
    open();
    expect(await screen.findByText(/This key is not on your contributor list/)).toBeTruthy();
    expect(screen.getByText(/does not show who holds the key/)).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Add these annotations' }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('offers no import when the signature check rejects the packet', async () => {
    answer({ preview_annotation_import: () => Promise.reject(new Error("This packet's signature does not verify")) });
    open();
    expect((await screen.findByRole('alert')).textContent).toContain('signature does not verify');
    expect(screen.queryByRole('button', { name: 'Add these annotations' })).toBeNull();
    expect(screen.queryByLabelText('Annotation import preview')).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith('import_annotation_packet', expect.anything());
  });

  it('never claims success when the import command rejects', async () => {
    answer({ import_annotation_packet: () => Promise.reject(new Error('The import was refused')) });
    open();
    fireEvent.click(await screen.findByRole('button', { name: 'Add these annotations' }));
    await waitFor(() => expect(screen.getAllByRole('alert').some((node) => node.textContent === 'The import was refused')).toBe(true));
    expect(screen.queryByText(/^Added /)).toBeNull();
  });

  it('refuses a packet made for a different document', async () => {
    answer({ preview_annotation_import: () => Promise.resolve({ ...preview, documentId: 8 }) });
    open();
    await screen.findByLabelText('Annotation import preview');
    expect((screen.getByRole('button', { name: 'Add these annotations' }) as HTMLButtonElement).disabled).toBe(true);
  });
});
