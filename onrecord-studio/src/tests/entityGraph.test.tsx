// @vitest-environment jsdom
//
// The entity graph publishes a neighbourhood that was requested for one entity. If a slower request
// for an entity the investigator has already clicked away from is allowed to land, the picture on
// screen shows one entity's name at its centre and another entity's relationships around it. That
// is not a rendering glitch: this application presents evidence, and the graph would be asserting
// co-occurrences the corpus never recorded for the entity it is labelled with.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';

const resolvers = new Map<string, (value: unknown) => void>();

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
    if (command === 'list_top_corpus_entities') {
      return Promise.resolve([
        node('entity-slow', 'Housing Authority', 9),
        node('entity-fast', 'rtfhsd.org', 4),
      ]);
    }
    if (command === 'get_corpus_entity_neighborhood') {
      const entityId = String(args?.entityId);
      // Held open so the test decides the resolution order, which is the whole point: the slow
      // request is the first one issued and lands last.
      return new Promise((resolve) => resolvers.set(entityId, resolve));
    }
    return Promise.resolve([]);
  },
}));

import EntityGraph from '../components/dashboard/EntityGraph';

function node(entityId: string, displayValue: string, documentCount: number) {
  return {
    entityId,
    kind: 'ORGANIZATION' as const,
    displayValue,
    documentCount,
    validationGrade: 'GAZETTEER_HIT' as const,
  };
}

function neighborhood(centreId: string, centreLabel: string, neighborLabel: string) {
  return {
    center: node(centreId, centreLabel, 9),
    edges: [
      {
        neighbor: node(`${centreId}-neighbor`, neighborLabel, 3),
        basis: 'COMPUTED' as const,
        sharedDocumentCount: 3,
        confidenceBasisPoints: 7000,
      },
    ],
    truncated: false,
  };
}

afterEach(() => {
  resolvers.clear();
  cleanup();
});

describe('EntityGraph', () => {
  it('discards a neighbourhood that resolves after the investigator moved to another entity', async () => {
    render(<EntityGraph />);
    await waitFor(() => expect(resolvers.has('entity-slow')).toBe(true));

    // The investigator clicks through to the second entity while the first is still in flight.
    screen.getByRole('button', { name: /rtfhsd\.org/ }).click();
    await waitFor(() => expect(resolvers.has('entity-fast')).toBe(true));
    resolvers.get('entity-fast')?.(neighborhood('entity-fast', 'rtfhsd.org', 'CURRENT NEIGHBOUR'));
    await screen.findByText('CURRENT NEIGHBOUR');

    // Now the superseded request lands. Its result belongs to an entity that is no longer selected.
    resolvers.get('entity-slow')?.(
      neighborhood('entity-slow', 'Housing Authority', 'SUPERSEDED NEIGHBOUR'),
    );

    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByText('SUPERSEDED NEIGHBOUR')).toBeNull();
    expect(screen.getByText('CURRENT NEIGHBOUR')).toBeTruthy();
  });
});
