// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

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
      get length() { return entries.size; },
    },
  });
});

const version = (versionId: string) => ({ versionId, statuteId: 's', contentSha256: `${versionId}0000000000`, effectiveFrom: null, effectiveTo: null, sourceUri: null, importedAt: '2026-01-01T00:00:00Z' });
const held = new Map<string, (text: string) => void>();

vi.mock('../ipc', () => ({
  formatNativeError: String,
  invokeCommand: (command: string, args: { versionId?: string }) => {
    if (command === 'list_legal_authorities') {
      return Promise.resolve([{ statuteId: 's', jurisdiction: 'CA', statuteCitation: 'Gov. Code 7922', title: 'CPRA', authorityKind: 'STATE_STATUTE', currentVersionId: 'v3', contentSha256: 'x', mandatedDeadlineDays: 10, canonicalSourceUri: null, effectiveFrom: null, effectiveTo: null, provisionCount: 0, mandateCount: 0, importedAt: '2026-01-01T00:00:00Z' }]);
    }
    if (command === 'list_legal_authority_versions') return Promise.resolve([version('v3'), version('v2'), version('v1')]);
    if (command === 'get_legal_authority_version') {
      const id = args.versionId!;
      if (id === 'v1') return new Promise((resolve) => held.set(id, (fullText) => resolve({ version: version(id), fullText })));
      return Promise.resolve({ version: version(id), fullText: `text of ${id}` });
    }
    return Promise.resolve(null);
  },
}));

import LegalAuthorityManager from '../components/legal/LegalAuthorityManager';

afterEach(() => {
  cleanup();
  held.clear();
});

describe('LegalAuthorityManager version comparison', () => {
  it('never shows the previously selected version text under a newly selected version', async () => {
    render(<LegalAuthorityManager isOpen onClose={() => {}} />);
    const baseline = await screen.findByLabelText('Earlier / baseline authority text', undefined, { timeout: 5000 }) as HTMLTextAreaElement;
    await waitFor(() => expect(baseline.value).toBe('text of v2'));

    const select = baseline.closest('label')!.querySelector('select')!;
    fireEvent.change(select, { target: { value: 'v1' } });
    await waitFor(() => expect(held.has('v1')).toBe(true));
    expect(baseline.value).not.toBe('text of v2');

    await act(async () => held.get('v1')!('text of v1'));
    await waitFor(() => expect(baseline.value).toBe('text of v1'));
  });
});
