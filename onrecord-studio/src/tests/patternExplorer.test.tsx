// @vitest-environment jsdom
//
// PatternExplorer's `openEvidence` is correctly gated against a second `openEvidence` call
// (`evidenceGate`). But selecting a different pattern in the list never invalidates that gate: if
// an evidence fetch for pattern X's excerpt is still in flight when the user selects pattern Y,
// X's excerpt still passes `evidenceGate.isCurrent(...)` and renders inside Y's now-selected
// "Source evidence" panel — a stale identity's finding published as the new selection's own.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { CorpusPattern } from '../types';

const resolvers = new Map<string, (chunk: unknown) => void>();
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));

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

vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));

function pattern(id: string, documentId: number, label: string): CorpusPattern {
  return {
    patternId: id,
    kind: 'SHARED_ENTITY',
    title: `Pattern ${id}`,
    explanation: `Explanation for ${id}`,
    confidenceBasisPoints: 8000,
    algorithmVersion: 'test',
    evidence: [{
      portalKey: 'city',
      requestId: 1,
      documentId,
      segmentIndex: null,
      characterStart: 10,
      characterEnd: 20,
      evidenceLabel: label,
      contentSha256: 'a'.repeat(64),
    }],
  };
}

import { useAppStore } from '../store/useAppStore';
import PatternExplorer from '../components/dashboard/PatternExplorer';

afterEach(() => {
  cleanup();
  resolvers.clear();
  act(() => {
    useAppStore.setState({ patternEvidenceSelection: null, activePortalKey: null });
  });
});

describe('PatternExplorer evidence gating', () => {
  it('does not attribute a stale in-flight excerpt to a newly selected pattern', async () => {
    bridge.invoke.mockImplementation(async (name: string, args?: { documentId?: number }) => {
      switch (name) {
        case 'query_corpus_patterns':
          return [pattern('pattern-x', 10, 'X evidence'), pattern('pattern-y', 20, 'Y evidence')];
        case 'get_corpus_index_status':
          return { indexedDocumentCount: 0, indexedSegmentCount: 0, indexedTermCount: 0, indexedRelationshipCount: 0, availableWriteCapacity: 0, algorithmVersion: 'test' };
        case 'get_document_text_chunk':
          return new Promise((resolve) => resolvers.set(String(args?.documentId ?? ''), resolve));
        case 'list_top_corpus_entities':
          return [];
        case 'list_resolution_candidates':
          return { candidates: [], nextCursor: null, remaining: 0, scanTruncated: false, guards: { skippedNameGroups: 0, skippedIdentifiers: 0 }, ledgerHead: '' };
        case 'list_resolution_ledger':
          return { items: [], nextCursor: null, total: 0, entryCount: 0, head: '', summary: { byBasis: [] }, contradictions: [] };
        default:
          return [];
      }
    });

    render(<PatternExplorer />);
    await screen.findByText('Pattern pattern-x', { selector: 'h3' });

    // Pattern X is auto-selected first; open its (slow) evidence excerpt.
    fireEvent.click(await screen.findByText('Open excerpt'));
    await waitFor(() => expect(resolvers.has('10')).toBe(true));

    // Before it resolves, the user selects pattern Y instead.
    fireEvent.click(screen.getByText('Pattern pattern-y', { selector: 'h3' }).closest('button')!);
    await screen.findByText('Y evidence');

    // X's excerpt fetch now resolves. Flush the `.then` chain fully (`waitFor`'s "eventually
    // true" semantics would otherwise pass on the first poll, before the stale write lands).
    await act(async () => {
      resolvers.get('10')!({
        documentId: 10,
        portalKey: 'city',
        characterOffset: 0,
        returnedCharacterCount: 10,
        nextCharacterOffset: null,
        totalCharacterCount: 10,
        maskedText: 'STALE TEXT FROM PATTERN X',
        maskedFindingCount: 0,
      });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // It must never appear: pattern Y is now selected, and this excerpt belongs to X.
    expect(screen.queryByText('STALE TEXT FROM PATTERN X')).toBeNull();
  });
});
