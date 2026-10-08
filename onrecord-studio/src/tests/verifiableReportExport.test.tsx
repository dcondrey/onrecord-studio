// @vitest-environment jsdom
//
// A report requested for one scope that resolves after the scope changed would present one
// scope's file as the export for another. The gate and the scope comparison must both hold.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const pending = new Map<string, (value: unknown) => void>();

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) => {
    if (command !== 'export_verifiable_report') return Promise.reject(new Error(`unexpected command ${command}`));
    return new Promise((resolve) => pending.set(String((args?.request as { portalKey?: unknown } | undefined)?.portalKey), resolve));
  },
}));

import VerifiableReportExport from '../components/dashboard/VerifiableReportExport';

function result(outputPath: string) {
  return {
    outputPath,
    fileSha256: 'f'.repeat(64),
    payloadSha256: 'e'.repeat(64),
    signerKeyId: 'k'.repeat(64),
    identityAssurance: 'SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED',
    claimCount: 3,
    withheldCount: 1,
    generatedAt: '2026-01-01T00:00:00Z',
  };
}

afterEach(() => {
  cleanup();
  pending.clear();
});

describe('VerifiableReportExport', () => {
  it('discards a report that resolves after the scope changed', async () => {
    const { rerender } = render(<VerifiableReportExport portalKey="portal-a" />);
    fireEvent.click(screen.getByRole('button', { name: /export verifiable report/i }));
    await waitFor(() => expect(pending.has('portal-a')).toBe(true));
    rerender(<VerifiableReportExport portalKey="portal-b" />);
    pending.get('portal-a')?.(result('/reports/for-a.html'));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText(/for-a\.html/)).toBeNull();
    expect((screen.getByRole('button', { name: /export verifiable report/i }) as HTMLButtonElement).disabled).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: /export verifiable report/i }));
    await waitFor(() => expect(pending.has('portal-b')).toBe(true));
    pending.get('portal-b')?.(result('/reports/for-b.html'));
    expect(await screen.findByText(/for-b\.html/)).toBeTruthy();
  });
});
