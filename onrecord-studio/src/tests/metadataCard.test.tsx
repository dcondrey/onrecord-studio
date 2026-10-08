// @vitest-environment jsdom
//
// "Latest verification" states what the append-only log holds for this document. A read still in
// flight is not an empty log, and a read for document A resolving after the card moved to B would
// state A's check as B's.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, render, screen } from '@testing-library/react';
import MetadataCard from '../components/documents/MetadataCard';
import type { ProvenanceVerificationRecord } from '../types';

const bridge = vi.hoisted(() => ({ pending: new Map<number, (records: ProvenanceVerificationRecord[]) => void>() }));

vi.mock('../ipc', () => ({
  invokeCommand: (_command: string, args: { documentId: number }) =>
    new Promise((resolve) => bridge.pending.set(args.documentId, resolve)),
}));

const card = (documentId: number) => (
  <MetadataCard documentId={documentId} portalKey="example" sha256={null} contentType={null} visibility="public" downloadStatus="stored" fileSize={null} />
);
const record = (documentId: number) => ({ documentId, provenanceConfidence: null, predatesClassification: true, verifiedAt: 'not-a-date' }) as unknown as ProvenanceVerificationRecord;
const latest = () => screen.getByText('Latest verification').nextElementSibling?.textContent;

afterEach(() => { cleanup(); bridge.pending.clear(); });

describe('MetadataCard latest verification', () => {
  it('says the log is being read until it answers, and discards an answer for a document no longer shown', async () => {
    const { rerender } = render(card(1));
    expect(latest()).toBe('Reading log…');

    rerender(card(2));
    await act(async () => bridge.pending.get(1)!([record(1)]));
    expect(latest()).toBe('Reading log…');

    await act(async () => bridge.pending.get(2)!([]));
    expect(latest()).toBe('No recorded check');
  });

  it('names a check logged before classification as such and keeps an unparseable time as recorded', async () => {
    render(card(3));
    await act(async () => bridge.pending.get(3)!([record(3)]));
    expect(latest()).toBe('logged before classification · not-a-date');
  });
});
