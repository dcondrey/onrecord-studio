// @vitest-environment jsdom
//
// A review or export requested for one selection that resolves after the selection changed would
// present one selection's summary as what will be shared for another. The gate and the selection
// comparison must both hold, and nothing is written before the summary is confirmed.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';

const pending: Array<{ command: string; args: Record<string, unknown>; resolve: (value: unknown) => void }> = [];

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string, args?: Record<string, unknown>) =>
    new Promise((resolve) => pending.push({ command, args: args ?? {}, resolve })),
}));

import ShareableBundleExport from '../components/dashboard/ShareableBundleExport';

const findings = [
  { id: 'aaaa', headline: 'First finding' },
  { id: 'bbbb', headline: 'Second finding' },
];

function preview(headline: string, hash: string) {
  return {
    schemaVersion: 1,
    included: [
      { findingId: 'aaaa', headline, severity: 'NOTABLE', basisLabel: 'Pattern signal, not a legal conclusion', sourceCount: 1, excerptCount: 1, maskedValueCount: 2 },
    ],
    excluded: [
      { findingId: 'bbbb', headline: 'Private thing', reason: 'CITES_NON_PUBLIC_RECORD', explanation: 'rests on a record that is not public' },
    ],
    sourceNames: ['response.pdf'],
    contentSha256: hash,
  };
}

function requestsFor(command: string) {
  return pending.filter((entry) => entry.command === command);
}

afterEach(() => {
  cleanup();
  pending.length = 0;
});

describe('ShareableBundleExport', () => {
  it('writes nothing until the summary is confirmed, then sends the confirmed hash', async () => {
    render(<ShareableBundleExport findings={findings} />);
    fireEvent.click(screen.getByRole('button', { name: /review 2 finding/i }));
    await waitFor(() => expect(requestsFor('preview_shareable_bundle')).toHaveLength(1));
    requestsFor('preview_shareable_bundle')[0].resolve(preview('Shown headline', 'c'.repeat(64)));
    expect(await screen.findByText(/Shown headline/)).toBeTruthy();
    expect(screen.getByText(/rests on a record that is not public/)).toBeTruthy();
    const write = screen.getByRole('button', { name: /write shareable file/i }) as HTMLButtonElement;
    expect(write.disabled).toBe(true);
    expect(requestsFor('export_shareable_bundle')).toHaveLength(0);
    fireEvent.click(screen.getByRole('checkbox', { name: /i have reviewed this list/i }));
    expect(write.disabled).toBe(false);
    fireEvent.click(write);
    await waitFor(() => expect(requestsFor('export_shareable_bundle')).toHaveLength(1));
    expect(requestsFor('export_shareable_bundle')[0].args).toEqual({
      request: { findingIds: ['aaaa', 'bbbb'], confirmedContentSha256: 'c'.repeat(64), targetPath: null },
    });
  });

  it('discards a summary that resolves after the selection changed', async () => {
    render(<ShareableBundleExport findings={findings} />);
    fireEvent.click(screen.getByRole('button', { name: /review 2 finding/i }));
    await waitFor(() => expect(requestsFor('preview_shareable_bundle')).toHaveLength(1));
    fireEvent.click(screen.getByRole('checkbox', { name: /include: second finding/i }));
    requestsFor('preview_shareable_bundle')[0].resolve(preview('Stale headline', 'd'.repeat(64)));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText(/Stale headline/)).toBeNull();
    expect((screen.getByRole('button', { name: /review 1 finding/i }) as HTMLButtonElement).disabled).toBe(false);
  });

  it('discards a written result that resolves after the findings on screen changed', async () => {
    const { rerender } = render(<ShareableBundleExport findings={findings} />);
    fireEvent.click(screen.getByRole('button', { name: /review 2 finding/i }));
    await waitFor(() => expect(requestsFor('preview_shareable_bundle')).toHaveLength(1));
    requestsFor('preview_shareable_bundle')[0].resolve(preview('Shown headline', 'c'.repeat(64)));
    await screen.findByText(/Shown headline/);
    fireEvent.click(screen.getByRole('checkbox', { name: /i have reviewed this list/i }));
    fireEvent.click(screen.getByRole('button', { name: /write shareable file/i }));
    await waitFor(() => expect(requestsFor('export_shareable_bundle')).toHaveLength(1));
    // The inbox refreshes to a different list while the write is still in flight.
    rerender(<ShareableBundleExport findings={findings.slice(0, 1)} />);
    requestsFor('export_shareable_bundle')[0].resolve({
      outputPath: '/reports/stale.html',
      fileSha256: 'f'.repeat(64),
      contentSha256: 'c'.repeat(64),
      generatedAt: '2026-01-01T00:00:00Z',
      findingCount: 1,
      excludedCount: 1,
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText(/stale\.html/)).toBeNull();
  });
});
