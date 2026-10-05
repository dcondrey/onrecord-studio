// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ReconstructionPanel from '../components/documents/ReconstructionPanel';
import DocumentViewer from '../components/documents/DocumentViewer';
import type { ReconstructionReport } from '../types';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand: invoke }));
afterEach(() => { cleanup(); invoke.mockReset(); });
const report = (id: number, terms = ['Chief of Police']): ReconstructionReport => ({ analysisVersion: 'fixture', documentId: id, portalKey: 'city', sourceSha256: 'source', corpusFingerprint: 'corpus', documentsCompared: 4, passagesCompared: 8, findings: [], limitations: ['Unknown mask widths remain unknown.'], searchTerms: terms });

describe('cross-record reconstruction', () => {
  it('keeps cross-record analysis available for retained text after original cleanup', async () => {
    invoke.mockResolvedValue(report(1));
    render(<DocumentViewer documentId={1} portalKey="city" fileName="correspondence.txt" originalAvailable={false} textContent="From: [REDACTED]" fileSizeFormatted="Referenced" />);
    fireEvent.click(screen.getByRole('button', { name: 'Audit redactions' }));
    fireEvent.click(screen.getByRole('button', { name: 'Compare related records' }));
    await screen.findByText(/Compared 4 records/);
    expect(screen.queryByRole('button', { name: 'Run integrity scan' })).toBeNull();
    expect(invoke.mock.calls.filter(([command]) => !['get_document_entity_truncations', 'describe_document_origin'].includes(command))).toHaveLength(1);
  });

  it('compares retained text with explicit options and explains an unresolved result', async () => {
    invoke.mockResolvedValue(report(1));
    render(<ReconstructionPanel documentId={1} portalKey="city" />);
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Chief of Police' } });
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: 'Compare related records' }));
    await screen.findByText(/No supported reconstruction target/);
    expect(invoke).toHaveBeenCalledWith('reconstruct_document_context', { documentId: 1, portalKey: 'city', options: { contextQuote: 'Chief of Police', fixedWidthMasks: true } });
    expect(screen.getByText(/Compared 4 records/)).toBeTruthy();
  });

  it('discards an old document result after the viewer changes identity', async () => {
    let resolve!: (value: ReconstructionReport) => void;
    invoke.mockReturnValue(new Promise<ReconstructionReport>((done) => { resolve = done; }));
    const view = render(<ReconstructionPanel documentId={1} portalKey="city" />);
    fireEvent.click(screen.getByRole('button', { name: 'Compare related records' }));
    view.rerender(<ReconstructionPanel documentId={2} portalKey="city" />);
    await act(async () => { resolve(report(1, ['STALE DOCUMENT RESULT'])); });
    expect(screen.queryByText(/STALE DOCUMENT RESULT/)).toBeNull();
    expect(screen.queryByText(/Compared 4 records/)).toBeNull();
    expect(screen.getByRole('button', { name: 'Compare related records' }).hasAttribute('disabled')).toBe(false);
  });

  it('discards an old failure while the new document comparison remains pending', async () => {
    let reject!: (reason: Error) => void;
    invoke.mockReturnValueOnce(new Promise<ReconstructionReport>((_, fail) => { reject = fail; }));
    invoke.mockReturnValueOnce(new Promise<ReconstructionReport>(() => {}));
    const view = render(<ReconstructionPanel documentId={1} portalKey="city" />);
    fireEvent.click(screen.getByRole('button', { name: 'Compare related records' }));
    view.rerender(<ReconstructionPanel documentId={2} portalKey="city" />);
    fireEvent.click(screen.getByRole('button', { name: 'Compare related records' }));
    await act(async () => { reject(new Error('STALE FAILURE')); });
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByRole('button', { name: 'Comparing records…' }).hasAttribute('disabled')).toBe(true);
  });
});
