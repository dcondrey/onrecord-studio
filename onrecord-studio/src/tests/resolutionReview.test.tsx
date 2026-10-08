// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ResolutionReview, { decisionLines } from '../components/dashboard/ResolutionReview';
import type { ResolutionCandidate, ResolutionCandidatePage, ResolutionLedgerPage } from '../types';

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock('../ipc', () => ({ invokeCommand, formatNativeError: String }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(cleanup);

const side = (value: string) => ({ key: { kind: 'PERSON', value }, displayValue: value, documentCount: 3, examples: [] });
const candidate: ResolutionCandidate = {
  pairId: 'a||b',
  leftEntityId: 'id-a',
  rightEntityId: 'id-b',
  evidence: { basis: 'NAME_ONLY', left: side('john smith'), right: side('smith john'), sharedIdentifier: null, nameVariants: [] },
};
const queue: ResolutionCandidatePage = { candidates: [candidate], nextCursor: null, remaining: 1, scanTruncated: false, guards: { skippedNameGroups: 0, skippedIdentifiers: 0 }, ledgerHead: 'h' };
const ledger: ResolutionLedgerPage = { items: [], nextCursor: null, total: 0, entryCount: 0, head: 'h', summary: { byBasis: [] }, contradictions: [] };

beforeEach(() => {
  invokeCommand.mockReset();
  invokeCommand.mockImplementation(async (name: string) => {
    if (name === 'list_resolution_candidates') return queue;
    if (name === 'list_resolution_ledger') return ledger;
    if (name === 'record_resolution_judgment') return { entry: { id: 'e1' }, importedFrom: null, hash: 'x', isLatest: true };
    if (name === 'list_trusted_contributors') return [];
    return null;
  });
});

describe('decisionLines', () => {
  it('states a measured count of the reviewer decisions and nothing more', () => {
    expect(decisionLines({ byBasis: [{ basis: 'NAME_ONLY', same: 31, different: 7, unsure: 2 }] })).toEqual([
      'You marked 31 of 40 name-only suggestions as the same.',
    ]);
  });
});

describe('ResolutionReview shortcuts', () => {
  it('records on a key press only outside the reason field, and only with a reviewer name', async () => {
    render(<ResolutionReview />);
    const card = await screen.findByRole('article', { name: 'Candidate pair' });
    fireEvent.keyDown(card, { key: 's' });
    expect(invokeCommand).not.toHaveBeenCalledWith('record_resolution_judgment', expect.anything());

    fireEvent.change(screen.getByLabelText('Recorded as'), { target: { value: 'Maria' } });
    fireEvent.keyDown(screen.getByLabelText(/Reason/), { key: 's' });
    expect(invokeCommand).not.toHaveBeenCalledWith('record_resolution_judgment', expect.anything());

    fireEvent.keyDown(card, { key: 'd' });
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith('record_resolution_judgment', {
        input: { leftEntityId: 'id-a', rightEntityId: 'id-b', verdict: 'DIFFERENT', author: 'Maria', reason: '', supersedes: null },
      }),
    );
  });
});
