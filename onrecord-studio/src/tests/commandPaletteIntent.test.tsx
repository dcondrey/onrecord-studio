// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before store import.
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

interface Deferred {
  utterance: string;
  resolve: (value: unknown) => void;
  reject: (reason: unknown) => void;
}

const bridge = vi.hoisted(() => ({
  invoked: [] as { command: string; args: { utterance?: string; limit?: number } }[],
  pending: [] as Deferred[],
}));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args: { utterance?: string; limit?: number }) => {
    bridge.invoked.push({ command, args });
    if (command !== 'run_intent_query') return Promise.resolve(null);
    return new Promise((resolve, reject) => {
      bridge.pending.push({ utterance: args.utterance ?? '', resolve, reject });
    });
  },
}));

import CommandPalette, { OPEN_COMMAND_PALETTE_EVENT } from '../components/dashboard/CommandPalette';
import type { IntentAnswer, IntentRow } from '../types';

function row(overrides: Partial<IntentRow> = {}): IntentRow {
  return {
    id: 'row-1',
    headline: 'Jane Doe (PERSON) has a SIGNED_BY relationship with City Attorney (ROLE)',
    detail: 'Recorded in 1 document(s).',
    basis: 'TYPED_RELATIONSHIP',
    basisStatement: 'A relationship type the document text itself carried.',
    relationshipKind: 'SIGNED_BY',
    confidenceBasisPoints: 6500,
    agencyName: null,
    portalKeys: ['sd-city'],
    citations: [
      {
        portalKey: 'sd-city',
        documentId: 7,
        characterStart: 12,
        characterEnd: 60,
        quote: 'Sincerely, Jane Q. Doe City Attorney',
        label: 'One passage contains both names.',
      },
    ],
    omittedCitationCount: 0,
    citationsUnavailableReason: null,
    ...overrides,
  };
}

function answer(summary: string, overrides: Partial<IntentAnswer> = {}): IntentAnswer {
  return {
    utterance: 'question',
    status: 'ANSWERED',
    plan: null,
    summary,
    rows: [row()],
    truncated: false,
    rowLimit: 10,
    notes: [],
    options: [],
    ...overrides,
  };
}

function openPalette() {
  act(() => {
    window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT));
  });
  return screen.getByRole('combobox') as HTMLInputElement;
}

function type(input: HTMLInputElement, value: string) {
  fireEvent.change(input, { target: { value } });
}

function pressEnter() {
  fireEvent.keyDown(window, { key: 'Enter' });
}

describe('CommandPalette intent router', () => {
  beforeEach(() => {
    bridge.invoked = [];
    bridge.pending = [];
    // jsdom does not implement scrolling.
    Element.prototype.scrollIntoView = () => {};
  });

  afterEach(() => {
    cleanup();
  });

  it('keeps view-jump commands and offers Ask only for a phrase', () => {
    render(<CommandPalette />);
    const input = openPalette();
    type(input, 'overview');
    expect(screen.queryByText(/^Ask:/)).toBeNull();
    expect(screen.getAllByRole('option').length).toBeGreaterThan(0);
    type(input, 'who signed for the City Attorney');
    expect(screen.getAllByRole('option')[0].textContent).toContain('Ask: who signed for the City Attorney');
  });

  it('asks on Enter, keeps the palette open, and renders basis, location and quote', async () => {
    render(<CommandPalette />);
    const input = openPalette();
    type(input, 'who signed for the City Attorney');
    expect(bridge.invoked).toHaveLength(0);
    pressEnter();
    expect(bridge.invoked).toEqual([
      { command: 'run_intent_query', args: { utterance: 'who signed for the City Attorney', limit: 10 } },
    ]);
    expect(screen.getByRole('dialog')).toBeTruthy();
    await act(async () => {
      bridge.pending[0].resolve(answer('One typed SIGNED_BY relationship recorded.'));
    });
    expect(screen.getByText('One typed SIGNED_BY relationship recorded.')).toBeTruthy();
    expect(screen.getByText('Typed relationship')).toBeTruthy();
    expect(screen.getByText('sd-city · document 7 · characters 12-60')).toBeTruthy();
    expect(screen.getByText('Sincerely, Jane Q. Doe City Attorney')).toBeTruthy();
    expect(screen.getByRole('dialog')).toBeTruthy();
  });

  it('shows a missing citation as unavailable and a missing confidence as not assessed', async () => {
    render(<CommandPalette />);
    type(openPalette(), 'show contradictions in agency Parks');
    pressEnter();
    await act(async () => {
      bridge.pending[0].resolve(
        answer('One finding.', {
          rows: [
            row({
              basis: 'STORED_FINDING',
              relationshipKind: null,
              confidenceBasisPoints: null,
              citations: [],
              citationsUnavailableReason: 'The detector stored this finding without citations.',
            }),
          ],
        }),
      );
    });
    expect(screen.getByText(/Citation unavailable: The detector stored this finding without citations/)).toBeTruthy();
    expect(screen.getByText('Confidence not assessed')).toBeTruthy();
  });

  it('discards an answer that resolves after a newer question was asked', async () => {
    render(<CommandPalette />);
    const input = openPalette();
    type(input, 'who signed for the City Attorney');
    pressEnter();
    // Back to the list, then ask something else while the first is still in flight.
    fireEvent.keyDown(window, { key: 'Escape' });
    type(input, 'show contradictions in agency Parks');
    pressEnter();
    expect(bridge.pending.map((call) => call.utterance)).toEqual([
      'who signed for the City Attorney',
      'show contradictions in agency Parks',
    ]);
    await act(async () => {
      bridge.pending[1].resolve(answer('Answer for Parks.'));
    });
    await act(async () => {
      bridge.pending[0].resolve(answer('Answer for the City Attorney.'));
    });
    expect(screen.getByText('Answer for Parks.')).toBeTruthy();
    expect(screen.queryByText('Answer for the City Attorney.')).toBeNull();
  });

  it('discards an answer that arrives after the text it answered was edited', async () => {
    render(<CommandPalette />);
    const input = openPalette();
    type(input, 'who signed for the City Attorney');
    pressEnter();
    type(input, 'who signed for the Mayor');
    await act(async () => {
      bridge.pending[0].resolve(answer('Answer for the City Attorney.'));
    });
    expect(screen.queryByText('Answer for the City Attorney.')).toBeNull();
    expect(screen.queryByRole('status')).toBeNull();
  });

  it('discards an answer that arrives after the palette was closed and reopened', async () => {
    render(<CommandPalette />);
    type(openPalette(), 'who signed for the City Attorney');
    pressEnter();
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    openPalette();
    await act(async () => {
      bridge.pending[0].resolve(answer('Answer for the City Attorney.'));
    });
    expect(screen.queryByText('Answer for the City Attorney.')).toBeNull();
  });

  it('discards a failure that belongs to a superseded question', async () => {
    render(<CommandPalette />);
    const input = openPalette();
    type(input, 'who signed for the City Attorney');
    pressEnter();
    type(input, 'who signed for the Mayor');
    await act(async () => {
      bridge.pending[0].reject(new Error('old failure'));
    });
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('lists what can be asked when the question is not understood and asks a chosen example', async () => {
    render(<CommandPalette />);
    type(openPalette(), 'tell me a joke please');
    pressEnter();
    await act(async () => {
      bridge.pending[0].resolve(
        answer('Not understood. This router answers only the questions listed below and does not guess.', {
          status: 'NOT_UNDERSTOOD',
          rows: [],
          options: [
            {
              label: 'Why a request missed its deadline',
              description: 'The statutory evaluation of one request.',
              example: 'why did request 42 miss its deadline',
            },
          ],
        }),
      );
    });
    expect(screen.getByText('Not understood')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /why a request missed its deadline/i }));
    await waitFor(() => expect(bridge.pending).toHaveLength(2));
    expect(bridge.pending[1].utterance).toBe('why did request 42 miss its deadline');
  });

  it('shows a native failure instead of an empty answer', async () => {
    render(<CommandPalette />);
    type(openPalette(), 'who signed for the City Attorney');
    pressEnter();
    await act(async () => {
      bridge.pending[0].reject(new Error('index offline'));
    });
    expect(screen.getByRole('alert').textContent).toContain('index offline');
  });
});
