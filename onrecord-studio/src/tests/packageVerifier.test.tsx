// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import PackageVerifier from '../components/dashboard/PackageVerifier';
import type { PackageVerificationResult } from '../types/investigation';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('../ipc', async (importOriginal) => ({ ...(await importOriginal<typeof import('../ipc')>()), invokeCommand: invoke }));
afterEach(() => { cleanup(); invoke.mockReset(); });

const passing = (): PackageVerificationResult => ({
  schemaVersion: '1',
  canonicalBytesHashMatches: true,
  signatureValid: true,
  manifestMatchesCanonicalBytes: true,
  overallPassed: true,
  identityAssurance: 'self-managed-key',
  unverifiedCitationLabels: [],
  withheldCitationLabels: [],
  ledgerSequence: null,
  previousExportSha256: null,
  revokedSigner: null,
  parseError: null,
});

function upload(file: File) {
  const input = screen.getByLabelText(/Open a signed package/) as HTMLInputElement;
  fireEvent.change(input, { target: { files: [file] } });
}

describe('PackageVerifier', () => {
  it('sends the opened file to verify_evidence_package and renders a passing result', async () => {
    invoke.mockResolvedValue(passing());
    render(<PackageVerifier />);
    fireEvent.click(screen.getByText('Verify a signed chain-of-custody or evidence package'));
    upload(new File(['{"manifest":{}}'], 'package.json', { type: 'application/json' }));
    await screen.findByText(/Overall: PASS/);
    expect(invoke).toHaveBeenCalledWith('verify_evidence_package', { payload: '{"manifest":{}}' });
    expect(screen.getByText('package.json')).toBeTruthy();
  });

  it('surfaces each independent failing check and never folds a hidden unverified citation into overall PASS', async () => {
    invoke.mockResolvedValue({
      ...passing(),
      signatureValid: false,
      overallPassed: false,
      unverifiedCitationLabels: ['Exhibit 4'],
    });
    render(<PackageVerifier />);
    fireEvent.click(screen.getByText('Verify a signed chain-of-custody or evidence package'));
    upload(new File(['{}'], 'tampered.json', { type: 'application/json' }));
    await screen.findByText(/Overall: FAIL/);
    expect(screen.getByText(/FAIL — Ed25519 signature verifies/)).toBeTruthy();
    expect(screen.getByText(/PASS — canonical bytes hash/)).toBeTruthy();
    expect(screen.getByText('Exhibit 4')).toBeTruthy();
  });

  it('rejects a file over 64 MB without calling the backend', () => {
    render(<PackageVerifier />);
    fireEvent.click(screen.getByText('Verify a signed chain-of-custody or evidence package'));
    const oversized = new File([new Uint8Array(1)], 'huge.json', { type: 'application/json' });
    Object.defineProperty(oversized, 'size', { value: 64 * 1024 * 1024 + 1 });
    upload(oversized);
    expect(screen.getByRole('alert').textContent).toMatch(/at most 64 MB/);
    expect(invoke).not.toHaveBeenCalled();
  });

  it('discards a slow first verification once the viewer has moved on to a second file', async () => {
    let resolveFirst!: (value: PackageVerificationResult) => void;
    invoke.mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }));
    invoke.mockResolvedValueOnce(passing());
    render(<PackageVerifier />);
    fireEvent.click(screen.getByText('Verify a signed chain-of-custody or evidence package'));
    upload(new File(['{}'], 'slow-large.json', { type: 'application/json' }));
    upload(new File(['{}'], 'fast-small.json', { type: 'application/json' }));
    await screen.findByText('fast-small.json');
    await act(async () => {
      resolveFirst({ ...passing(), overallPassed: false, parseError: 'STALE RESULT FOR slow-large.json' });
    });
    expect(screen.queryByText(/STALE RESULT/)).toBeNull();
    expect(screen.getByText('fast-small.json')).toBeTruthy();
  });

  it('reports a parse error distinctly from a failed check', async () => {
    invoke.mockResolvedValue({ ...passing(), overallPassed: false, parseError: 'not valid JSON' });
    render(<PackageVerifier />);
    fireEvent.click(screen.getByText('Verify a signed chain-of-custody or evidence package'));
    upload(new File(['not json'], 'broken.json', { type: 'application/json' }));
    await screen.findByText('not valid JSON');
    expect(screen.queryByText(/Overall:/)).toBeNull();
  });

  it.each([
    { status: 'RETIRED', overallPassed: true, overall: /Overall: PASS/, notice: /Signer key retired/, untrusted: false },
    { status: 'COMPROMISED', overallPassed: false, overall: /Overall: FAIL/, notice: /revoked as compromised/, untrusted: true },
  ] as const)('words a $status signer by what it means for trust', async ({ status, overallPassed, overall, notice, untrusted }) => {
    invoke.mockResolvedValue({
      ...passing(),
      overallPassed,
      revokedSigner: { keyId: 'abc123', revokedAt: '2026-09-29T00:00:00Z', reason: 'Rotation.', status },
    });
    render(<PackageVerifier />);
    fireEvent.click(screen.getByText('Verify a signed chain-of-custody or evidence package'));
    upload(new File(['{}'], `${status}.json`, { type: 'application/json' }));
    await screen.findByText(overall);
    expect(screen.getByText(notice)).toBeTruthy();
    expect(screen.queryByText(/treat the package as untrusted/) !== null).toBe(untrusted);
  });
});
