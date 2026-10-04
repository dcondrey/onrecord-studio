// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { EvidenceTrace } from '../components/ui/EvidenceBasis';
import type { FindingCitation, FindingCitationAnchor } from '../types';

type Pending = { id: string; resolve: (anchors: FindingCitationAnchor[]) => void };
const pending: Pending[] = [];

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
    if (command !== 'resolve_finding_citations') return Promise.reject(new Error(`unexpected ${command}`));
    return new Promise((resolve) => pending.push({ id: String(args?.id), resolve }));
  },
}));

afterEach(() => {
  cleanup();
  pending.length = 0;
});

function citation(documentId: number, start: number | null, end: number | null): FindingCitation {
  return { portalKey: 'p', documentId, characterStart: start, characterEnd: end, quote: 'the cited words', label: null };
}

function anchorEntry(cited: FindingCitation, index: number, anchor: FindingCitationAnchor['anchor']): FindingCitationAnchor {
  return { citationIndex: index, documentId: cited.documentId, characterStart: cited.characterStart, characterEnd: cited.characterEnd, anchor };
}

async function settle(id: string, anchors: FindingCitationAnchor[]) {
  const request = pending.find((entry) => entry.id === id);
  if (!request) throw new Error(`no request for ${id}`);
  await act(async () => { request.resolve(anchors); });
}

describe('citation anchors in the evidence trace', () => {
  it('states moved and lost passages, and opens each where it now is or unmarked', async () => {
    const moved = citation(7, 3, 9);
    const lost = citation(8, 10, 20);
    const unverified = citation(9, 1, 4);
    const onOpenSource = vi.fn();
    render(<EvidenceTrace label="x" explanation="because" citations={[moved, lost, unverified]} onOpenSource={onOpenSource} findingId="f1" />);
    fireEvent.click(screen.getByRole('button', { name: /why we believe this/i }));
    expect(screen.getAllByText(/checking the passage/i)).toHaveLength(3);

    await settle('f1', [
      anchorEntry(moved, 0, { kind: 'ANCHORED', spanId: 's0', resolution: { state: 'DRIFTED', start: 40, end: 46 } }),
      anchorEntry(lost, 1, { kind: 'ANCHORED', spanId: 's1', resolution: { state: 'STALE', reason: 'MISSING' } }),
      anchorEntry(unverified, 2, { kind: 'UNANCHORED', reason: 'NOT_ANCHORED' }),
    ]);
    expect(screen.getByText(/now at characters 40–46/)).toBeTruthy();
    expect(screen.getByText(/no longer in the document text/)).toBeTruthy();
    expect(screen.getByText(/position not verifiable/i)).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: /open source document 7/i }));
    expect(onOpenSource).toHaveBeenLastCalledWith({ ...moved, characterStart: 40, characterEnd: 46 });
    fireEvent.click(screen.getByRole('button', { name: /open source document 8/i }));
    expect(onOpenSource).toHaveBeenLastCalledWith({ ...lost, characterStart: null, characterEnd: null });
  });

  it('ignores an answer read for a different version of the citation', async () => {
    const current = citation(7, 3, 9);
    render(<EvidenceTrace label="x" explanation="because" citations={[current]} findingId="f1" />);
    fireEvent.click(screen.getByRole('button', { name: /why we believe this/i }));
    await settle('f1', [anchorEntry(citation(7, 50, 60), 0, { kind: 'ANCHORED', spanId: 's', resolution: { state: 'STALE', reason: 'MISSING' } })]);
    expect(screen.queryByText(/passage not found/i)).toBeNull();
  });

  // Proven by removing the `gate.isCurrent(token)` checks in `useCitationAnchors`: the late answer
  // for f1 then replaces f2's state and the trace falls back to "Checking", failing the last line.
  it('never lets a slower check for an earlier finding replace the current one', async () => {
    const first = citation(7, 3, 9);
    const second = citation(8, 1, 5);
    const { rerender } = render(<EvidenceTrace label="x" explanation="because" citations={[first]} findingId="f1" />);
    fireEvent.click(screen.getByRole('button', { name: /why we believe this/i }));
    rerender(<EvidenceTrace label="x" explanation="because" citations={[second]} findingId="f2" />);

    await settle('f2', [anchorEntry(second, 0, { kind: 'ANCHORED', spanId: 's2', resolution: { state: 'INTACT', start: 1, end: 5, projection: 'UNCHANGED' } })]);
    expect(screen.getByText(/passage in place/i)).toBeTruthy();

    await settle('f1', [anchorEntry(first, 0, { kind: 'ANCHORED', spanId: 's1', resolution: { state: 'STALE', reason: 'MISSING' } })]);
    expect(screen.queryByText(/passage not found/i)).toBeNull();
    expect(screen.getByText(/passage in place/i)).toBeTruthy();
  });
});
