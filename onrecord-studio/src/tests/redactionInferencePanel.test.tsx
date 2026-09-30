// @vitest-environment jsdom
//
// The redaction-inference panel fetches inference for a specific span. If a slower fetch for a
// previously-opened span publishes into a panel now showing a different span, the panel becomes a
// false statement about which redaction the candidates apply to. Every fetch goes through
// `useLatestRequestGate`; removing the gate must fail these tests.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';

const responses = new Map<string, unknown>();

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
    if (command === 'infer_redaction') {
      const key = `${args?.documentId}:${args?.spanStart}-${args?.spanEnd}`;
      return Promise.resolve(responses.get(key) ?? emptyReport());
    }
    return Promise.reject(new Error(`unexpected command ${command}`));
  },
}));

import RedactionInferencePanel from '../components/documents/RedactionInferencePanel';

function emptyReport() {
  return {
    candidates: [],
    corroboratingDocs: [],
    confidenceCeiling: { kind: 'UNASSESSED' },
    semanticRankingUsed: false,
    contextSnippet: '',
  };
}

function populatedReport() {
  return {
    candidates: [
      {
        kind: 'PERSON',
        value: 'Jane Doe',
        likelihoodBasisPoints: 5200,
        supportingEvidence: [
          { portalKey: 'north', documentId: 42, quote: 'signed by Jane Doe', matchScore: 4000 },
          { portalKey: 'north', documentId: 43, quote: 'Jane Doe attended the meeting', matchScore: 3800 },
        ],
      },
    ],
    corroboratingDocs: [
      { portalKey: 'north', documentId: 42 },
      { portalKey: 'north', documentId: 43 },
    ],
    confidenceCeiling: { kind: 'MEASURED', basisPoints: 6000 },
    semanticRankingUsed: false,
    contextSnippet: 'The letter was signed by «REDACTED» on the following date',
  };
}

afterEach(() => {
  cleanup();
  responses.clear();
});

describe('RedactionInferencePanel', () => {
  it('renders ranked candidates with corroborating evidence', async () => {
    responses.set('7:10-20', populatedReport());
    render(<RedactionInferencePanel documentId={7} portalKey="north" spanStart={10} spanEnd={20} />);
    await screen.findByText(/2 corroborating document/);
    expect(screen.getAllByText(/Jane Doe/).length).toBeGreaterThan(0);
    expect(screen.getByText(/signed by Jane Doe/)).toBeTruthy();
    expect(screen.getByText(/52\.0%/)).toBeTruthy();
    expect(screen.getByText(/lexical corroboration/)).toBeTruthy();
  });

  it('renders an empty state when no candidates are inferred', async () => {
    responses.set('7:10-20', emptyReport());
    render(<RedactionInferencePanel documentId={7} portalKey="north" spanStart={10} spanEnd={20} />);
    await screen.findByText(/No corroborating context/);
  });

  it('frames the output as inference, never as decryption', async () => {
    responses.set('7:10-20', populatedReport());
    render(<RedactionInferencePanel documentId={7} portalKey="north" spanStart={10} spanEnd={20} />);
    await screen.findByText(/2 corroborating document/);
    expect(screen.getByText(/inference, not decryption/i)).toBeTruthy();
  });

  it('discards a slower prior-span fetch after the span changes', async () => {
    let resolveFirst!: (value: unknown) => void;
    const controlled = new Promise((resolve) => {
      resolveFirst = resolve;
    });
    let callCount = 0;
    vi.doMock('@tauri-apps/api/core', () => ({
      isTauri: () => false,
      invoke: (command: string) => {
        if (command !== 'infer_redaction') return Promise.reject(new Error('no'));
        callCount += 1;
        if (callCount === 1) return controlled;
        return Promise.resolve({
          candidates: [
            {
              kind: 'PERSON',
              value: 'New Person',
              likelihoodBasisPoints: 6000,
              supportingEvidence: [
                { portalKey: 'north', documentId: 99, quote: 'New Person appeared', matchScore: 4000 },
              ],
            },
          ],
          corroboratingDocs: [{ portalKey: 'north', documentId: 99 }],
          confidenceCeiling: { kind: 'MEASURED', basisPoints: 6000 },
          semanticRankingUsed: false,
          contextSnippet: 'new context',
        });
      },
    }));
    const { rerender } = render(
      <RedactionInferencePanel documentId={7} portalKey="north" spanStart={10} spanEnd={20} />,
    );
    rerender(<RedactionInferencePanel documentId={7} portalKey="north" spanStart={100} spanEnd={110} />);
    // Late-resolve first fetch — must not publish stale data.
    resolveFirst({
      candidates: [
        {
          kind: 'PERSON',
          value: 'Stale Person',
          likelihoodBasisPoints: 5000,
          supportingEvidence: [],
        },
      ],
      corroboratingDocs: [],
      confidenceCeiling: { kind: 'MEASURED', basisPoints: 6000 },
      semanticRankingUsed: false,
      contextSnippet: 'stale',
    });
    await waitFor(() => expect(screen.queryByText(/Stale Person/)).toBeNull());
  });
});
