// @vitest-environment jsdom
//
// The subject dossier fires three concurrent fetches for one subject; if a slower fetch from a
// previously-opened subject is allowed to publish, the dossier labelled with Person A shows
// Person B's neighborhood or findings. That is not a rendering glitch: this application presents
// evidence, and the panel would be asserting relationships the corpus never recorded for the
// subject it names. Every fetch goes through `useLatestRequestGate`.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

const resolvers = new Map<string, (rows: unknown) => void>();
let findingsResponse: unknown[] = [];

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

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
    if (command === 'list_top_corpus_entities') {
      const which = String((args ?? {}).__testEntitiesTag ?? 'ambiguous');
      // Default: two matches for "smith" so the picker renders.
      if (which === 'ambiguous') {
        return Promise.resolve([
          node('entity-a', 'John Smith', 12),
          node('entity-b', 'Sarah Smith', 4),
        ]);
      }
      return Promise.resolve([node('entity-only', 'John Smith', 12)]);
    }
    if (command === 'get_corpus_entity_neighborhood') {
      const entityId = String(args?.entityId ?? '');
      return new Promise((resolve) => resolvers.set(`nb:${entityId}`, resolve));
    }
    if (command === 'list_findings') {
      return Promise.resolve(findingsResponse);
    }
    return Promise.resolve(null);
  },
}));

import { useAppStore } from '../store/useAppStore';
import SubjectDossier from '../components/dashboard/SubjectDossier';

function node(entityId: string, displayValue: string, documentCount: number) {
  return {
    entityId,
    kind: 'EMAIL_ADDRESS',
    displayValue,
    documentCount,
    validationGrade: 'STRICT',
  };
}

function finding(id: string, headline: string) {
  return {
    id,
    kind: 'ANOMALY',
    severity: 'NOTABLE',
    confidence: 'MEDIUM',
    subject: null,
    headline,
    explanation: `${headline} explanation`,
    citations: [],
    discoveredAt: new Date().toISOString(),
    lastSeenAt: new Date().toISOString(),
    dismissedAt: null,
    sourceDetector: 'test-detector',
  };
}

afterEach(() => {
  cleanup();
  resolvers.clear();
  findingsResponse = [];
  act(() => {
    useAppStore.setState({ subjectDossierTarget: null });
  });
});

describe('SubjectDossier', () => {
  it('renders nothing when no subject is set', () => {
    render(<SubjectDossier />);
    expect(screen.queryByLabelText('Subject dossier')).toBeNull();
  });

  it('shows a disambiguation picker on a name that matches more than one entity', async () => {
    act(() => {
      useAppStore.setState({ subjectDossierTarget: { kind: 'name', value: 'Smith' } });
    });
    render(<SubjectDossier />);
    await screen.findByText(/matches 2 entities/i);
    expect(screen.getByRole('button', { name: /John Smith/i })).toBeTruthy();
    expect(screen.getByRole('button', { name: /Sarah Smith/i })).toBeTruthy();
  });

  it('resolves picking one candidate into a dossier for that entity', async () => {
    act(() => {
      useAppStore.setState({ subjectDossierTarget: { kind: 'name', value: 'Smith' } });
    });
    render(<SubjectDossier />);
    const pick = await screen.findByRole('button', { name: /John Smith/i });
    fireEvent.click(pick);
    await waitFor(() => {
      expect(useAppStore.getState().subjectDossierTarget).toEqual({
        kind: 'entity',
        value: 'entity-a',
      });
    });
    // Sections render as loading independently.
    await screen.findByRole('status');
  });

  it('discards a slower neighborhood fetch after the subject changes', async () => {
    // First subject: John Smith (entity-a). Second: entity-b. The first neighborhood is held open,
    // then resolved AFTER the second is committed. Without the gate the first's payload would
    // publish under the second's label.
    act(() => {
      useAppStore.setState({ subjectDossierTarget: { kind: 'entity', value: 'entity-a' } });
    });
    render(<SubjectDossier />);
    await waitFor(() => expect(resolvers.has('nb:entity-a')).toBe(true));
    // Switch subject before entity-a resolves.
    act(() => {
      useAppStore.setState({ subjectDossierTarget: { kind: 'entity', value: 'entity-b' } });
    });
    await waitFor(() => expect(resolvers.has('nb:entity-b')).toBe(true));
    // Resolve b first (winner), then a (must be dropped).
    act(() => {
      resolvers.get('nb:entity-b')!({
        center: node('entity-b', 'Sarah Smith', 4),
        edges: [],
        truncated: false,
      });
      resolvers.get('nb:entity-a')!({
        center: node('entity-a', 'John Smith', 12),
        edges: [
          {
            neighbor: node('poison', 'wrong-poison-neighbor@example.com', 2),
            basis: 'COMPUTED',
            sharedDocumentCount: 5,
            confidenceBasisPoints: 8000,
          },
        ],
        truncated: false,
      });
    });
    await waitFor(() => {
      const neighborhoodSection = screen.getByText('Corpus neighborhood').closest('section');
      expect(within(neighborhoodSection!).getByText('Sarah Smith')).toBeTruthy();
    });
    // The stale neighborhood's neighbor label must not appear anywhere.
    expect(screen.queryByText(/wrong-poison-neighbor/i)).toBeNull();
  });

  it('backfills the label from the neighborhood when opened by entity id, so findings populate', async () => {
    // This is the "opened via EntityGraph or a finding row" path: `resolveTarget` sets
    // `label: null` for `{kind:'entity'}` targets, and `loadFindings` treats a null/empty label
    // as "no query" (matches: []). Without backfilling the label from the resolved neighborhood,
    // findings for this subject never populate, even when matching findings exist.
    findingsResponse = [finding('f1', 'Payment to John Smith flagged')];
    act(() => {
      useAppStore.setState({ subjectDossierTarget: { kind: 'entity', value: 'entity-a' } });
    });
    render(<SubjectDossier />);
    await waitFor(() => expect(resolvers.has('nb:entity-a')).toBe(true));
    act(() => {
      resolvers.get('nb:entity-a')!({
        center: node('entity-a', 'John Smith', 12),
        edges: [],
        truncated: false,
      });
    });
    await screen.findByText('Payment to John Smith flagged');

    // Switch subject before entity-b's neighborhood resolves: the label must revert to unresolved
    // and John Smith's finding must not remain attributed to the new subject.
    act(() => {
      useAppStore.setState({ subjectDossierTarget: { kind: 'entity', value: 'entity-b' } });
    });
    await waitFor(() => expect(resolvers.has('nb:entity-b')).toBe(true));
    expect(screen.queryByText('Payment to John Smith flagged')).toBeNull();
    expect(screen.queryByRole('heading', { name: 'John Smith' })).toBeNull();
  });
});
