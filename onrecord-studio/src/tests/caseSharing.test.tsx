// @vitest-environment jsdom
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { packetFileName, SignerTrustNotice } from '../components/dashboard/CaseSharing';
import type { useContributors } from '../components/dashboard/useContributors';

vi.mock('../ipc', () => ({ invokeCommand: vi.fn(), formatNativeError: String }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(cleanup);

const KEY = 'ab'.repeat(32);
const contributors = (list: { keyId: string; label: string }[]): ReturnType<typeof useContributors> => ({
  state: {
    status: 'ready',
    contributors: list.map((entry) => ({ ...entry, trustedAt: '2026-09-30T00:00:00Z' })),
    identity: { keyId: 'cd'.repeat(32) },
  },
  reload: vi.fn(),
  trust: vi.fn(),
  untrust: vi.fn(),
});

describe('SignerTrustNotice', () => {
  it('never presents an unknown key as verified, and names a key the user verified', () => {
    const { rerender } = render(<SignerTrustNotice signerKeyId={KEY} contributors={contributors([])} />);
    expect(screen.getByText(/not on your contributor list/i)).toBeTruthy();
    expect(screen.queryByText(/Signed by Maria/)).toBeNull();
    rerender(<SignerTrustNotice signerKeyId={KEY} contributors={contributors([{ keyId: KEY, label: 'Maria' }])} />);
    expect(screen.getByText(/Signed by Maria/)).toBeTruthy();
    rerender(<SignerTrustNotice signerKeyId={KEY.toUpperCase()} contributors={contributors([{ keyId: KEY, label: 'Maria' }])} />);
    expect(screen.getByText(/Signed by Maria/)).toBeTruthy();
  });
});

describe('packetFileName', () => {
  it('always yields a name the native side accepts', () => {
    const accepted = /^[A-Za-z0-9][A-Za-z0-9._-]{0,119}\.json$/;
    for (const title of ['Budget / 2026: "Contracts"', '   ', 'ünïcode ☃', 'x'.repeat(400)]) {
      expect(packetFileName(title, 12, KEY)).toMatch(accepted);
    }
  });
});
