// @vitest-environment jsdom
//
// A packet requested for one scope that resolves after the scope changed would present one
// scope's file as the packet for another. The gate and the scope comparison must both hold.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const pending = new Map<string, (value: unknown) => void>();

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
    if (command !== 'export_evidence_packet') return Promise.reject(new Error(`unexpected command ${command}`));
    return new Promise((resolve) => pending.set(String((args?.request as { portalKey?: unknown } | undefined)?.portalKey), resolve));
  },
}));

import EvidencePacketExport from '../components/dashboard/EvidencePacketExport';

function result(markdownPath: string) {
  return {
    markdownPath,
    jsonPath: markdownPath.replace(/\.md$/, '.json'),
    contentSha256: 'c'.repeat(64),
    claimCount: 3,
    gapCount: 2,
    withheldCount: 1,
    generatedAt: '2026-01-01T00:00:00Z',
  };
}

afterEach(() => {
  cleanup();
  pending.clear();
});

describe('EvidencePacketExport', () => {
  it('discards a packet that resolves after the scope changed', async () => {
    const { rerender } = render(<EvidencePacketExport portalKey="portal-a" />);
    fireEvent.click(screen.getByRole('button', { name: /export evidence packet/i }));
    await waitFor(() => expect(pending.has('portal-a')).toBe(true));
    rerender(<EvidencePacketExport portalKey="portal-b" />);
    pending.get('portal-a')?.(result('/reports/for-a.md'));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText(/for-a\.md/)).toBeNull();
    expect((screen.getByRole('button', { name: /export evidence packet/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('shows the packet for the scope that is still current and states gaps apart from claims', async () => {
    render(<EvidencePacketExport portalKey="portal-a" />);
    fireEvent.click(screen.getByRole('button', { name: /export evidence packet/i }));
    await waitFor(() => expect(pending.has('portal-a')).toBe(true));
    pending.get('portal-a')?.(result('/reports/for-a.md'));
    expect(await screen.findByText(/for-a\.md/)).toBeTruthy();
    expect(screen.getByText(/Wrote 3 claim\(s\); 2 gap\(s\) listed separately/)).toBeTruthy();
  });
});
