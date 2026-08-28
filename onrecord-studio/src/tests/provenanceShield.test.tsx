// @vitest-environment jsdom
//
// The confidence badge is the only place an investigator sees what a provenance check actually
// found, so it is pinned by rendering the viewer rather than by type-checking the payload. The
// regression this guards is a collapsed ladder: a digest mismatch is a substitution finding, and
// it previously rendered as the same amber "Provenance not confirmed" chip as "no digest was ever
// recorded" -- a finding presented as an absence of information.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import DocumentViewer from '../components/documents/DocumentViewer';
import type { DocumentProvenanceVerification, ProvenanceConfidence, ProvenanceVerificationRecord, RecoveredVersionProfile } from '../types';

const bridge = vi.hoisted(() => ({
  result: null as DocumentProvenanceVerification | null,
  history: [] as ProvenanceVerificationRecord[],
  gate: null as Promise<void> | null,
  // Per-document verification answers and the gates that hold them open, so a check started for
  // one document can be made to resolve after the viewer has already moved to another.
  verificationsByDocument: new Map<number, DocumentProvenanceVerification>(),
  verifyGates: new Map<number, Promise<void>>(),
  revisionProfile: null as RecoveredVersionProfile | null,
  revisionGate: null as Promise<void> | null,
}));

vi.mock('../ipc', () => ({
  invokeCommand: (command: string, args?: { documentId?: number }) => {
    if (command === 'verify_document_provenance') {
      const documentId = args?.documentId ?? 0;
      const answer = bridge.verificationsByDocument.get(documentId) ?? bridge.result;
      const gate = bridge.verifyGates.get(documentId);
      return gate ? gate.then(() => answer) : Promise.resolve(answer);
    }
    if (command === 'list_document_provenance_verifications') {
      return bridge.gate ? bridge.gate.then(() => bridge.history) : Promise.resolve(bridge.history);
    }
    if (command === 'carve_document_version_history') {
      const profile = bridge.revisionProfile;
      return bridge.revisionGate ? bridge.revisionGate.then(() => profile) : Promise.resolve(profile);
    }
    return Promise.reject(new Error(`unexpected command ${command}`));
  },
}));

function revisionProfile(): RecoveredVersionProfile {
  return {
    sourceSha256: 'd'.repeat(64),
    sourceSizeBytes: 2048,
    totalEofMarkers: 2,
    historicalVersionsFound: 1,
    inspectedHistoricalVersions: 1,
    recoveredTextSegments: [],
    postEofMetadataArtifacts: [],
    analysisTruncated: false,
    warnings: [],
  };
}

// Two microtask turns: one for the gate, one for the continuation that reads the guard.
async function settlePendingResolutions(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function verification(overrides: Partial<DocumentProvenanceVerification>): DocumentProvenanceVerification {
  return {
    documentId: 7,
    portalKey: 'example',
    expectedSha256: 'a'.repeat(64),
    computedSha256: 'a'.repeat(64),
    evidenceFilePresent: true,
    hashMatches: true,
    recordedDigestSource: 'INGESTION_RECORD',
    manifestPresent: false,
    isCryptographicallySigned: false,
    signatureVerification: 'NOT_ATTEMPTED',
    signatureIssuer: null,
    signingTimestamp: null,
    assertionCount: 0,
    invalidAssertionCount: 0,
    textProjectionSha256: 'b'.repeat(64),
    textProjectionMatches: true,
    provenanceConfidence: 'LOCAL_DIGEST_MATCH',
    chainOfCustodyConfirmed: false,
    verificationErrors: [],
    ...overrides,
  };
}

async function badgeFor(overrides: Partial<DocumentProvenanceVerification>): Promise<string[]> {
  bridge.result = verification(overrides);
  render(<DocumentViewer documentId={7} portalKey="example" fileName="release.pdf" textContent="" fileSizeFormatted="1 KB" />);
  await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());
  return [...document.querySelectorAll('span.rounded-full')].map((node) => node.textContent ?? '');
}

function record(overrides: Partial<ProvenanceVerificationRecord>): ProvenanceVerificationRecord {
  return {
    verificationId: 'v1',
    documentId: 7,
    portalKey: 'example',
    verifiedAt: '2026-08-01T00:00:00Z',
    expectedSha256: 'a'.repeat(64),
    computedSha256: 'a'.repeat(64),
    evidenceFilePresent: true,
    hashMatches: true,
    manifestPresent: false,
    isCryptographicallySigned: false,
    signatureIssuer: null,
    signingTimestamp: null,
    assertionCount: 0,
    invalidAssertionCount: 0,
    recordedDigestSource: 'INGESTION_RECORD',
    signatureVerification: 'NOT_ATTEMPTED',
    provenanceConfidence: 'LOCAL_DIGEST_MATCH',
    textProjectionSha256: 'b'.repeat(64),
    textProjectionMatches: true,
    chainOfCustodyConfirmed: false,
    verificationErrors: [],
    predatesClassification: false,
    ...overrides,
  };
}

async function openLog(history: ProvenanceVerificationRecord[]): Promise<HTMLElement> {
  bridge.result = verification({});
  bridge.history = history;
  render(<DocumentViewer documentId={7} portalKey="example" fileName="release.pdf" textContent="" fileSizeFormatted="1 KB" />);
  await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());
  fireEvent.click(screen.getByRole('button', { name: 'Verification log' }));
  await waitFor(() => expect(screen.queryByText('Reading verification log…')).toBeNull());
  return screen.getByRole('button', { name: 'Hide verification log' });
}

afterEach(() => {
  cleanup();
  window.history.replaceState(null, '', '/');
  bridge.result = null;
  bridge.history = [];
  bridge.gate = null;
  bridge.verificationsByDocument.clear();
  bridge.verifyGates.clear();
  bridge.revisionProfile = null;
  bridge.revisionGate = null;
});

describe('document evidence navigation', () => {
  it('finds extracted text and copies a Unicode-correct evidence offset', async () => {
    bridge.result = verification({});
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    render(<DocumentViewer documentId={7} requestId={3} portalKey="example" fileName="release.pdf" textContent="😀 evidence appears here" characterOffset={10} fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());

    fireEvent.change(screen.getByRole('textbox', { name: 'Search within this extracted-text chunk' }), { target: { value: 'evidence' } });
    expect(screen.getByText('1/1')).toBeTruthy();
    expect(document.querySelector('mark')?.textContent).toBe('evidence');
    fireEvent.click(screen.getByRole('button', { name: 'Copy evidence link' }));

    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const copied = new URL(writeText.mock.calls[0][0]);
    const fragment = new URLSearchParams(copied.hash.slice(1));
    expect(Object.fromEntries(fragment)).toEqual({ portal: 'example', request: '3', document: '7', offset: '12' });
  });

  it('paginates a large extracted-text chunk instead of mounting it all at once', async () => {
    bridge.result = verification({});
    render(<DocumentViewer documentId={7} portalKey="example" fileName="release.pdf" textContent={`${'a'.repeat(12_000)}SECOND_PAGE`} fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());

    expect(screen.queryByText('SECOND_PAGE')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Next text page' }));
    expect(screen.getByText('SECOND_PAGE')).toBeTruthy();
  });
});

describe('provenance shield', () => {
  it('gives every rung of the ladder its own label', async () => {
    const expected: Array<[ProvenanceConfidence, string]> = [
      ['TRUSTED_CHAIN_OF_CUSTODY', 'Chain of custody verified'],
      ['VERIFIED_SIGNATURE', 'Manifest signature verified'],
      ['LOCAL_DIGEST_MATCH', 'Local file hash verified'],
      ['RECORDED_CLAIM_ONLY', 'Recorded claim, unchecked'],
      ['NO_RECORDED_DIGEST', 'No recorded digest'],
      ['EVIDENCE_FILE_UNAVAILABLE', 'Evidence file missing'],
      ['DIGEST_MISMATCH', 'Digest mismatch'],
    ];
    const labels: string[] = [];
    for (const [confidence] of expected) {
      labels.push((await badgeFor({ provenanceConfidence: confidence }))[0]);
      cleanup();
    }
    expect(labels).toEqual(expected.map(([, label]) => label));
    expect(new Set(labels).size).toBe(expected.length);
  });

  it('marks a digest mismatch as a finding, not as missing information', async () => {
    const [badge] = await badgeFor({
      provenanceConfidence: 'DIGEST_MISMATCH',
      hashMatches: false,
      computedSha256: 'c'.repeat(64),
      verificationErrors: ['Stored bytes do not match the recorded digest.'],
    });
    expect(badge).toBe('Digest mismatch');
    expect(document.querySelector('span.text-rose-300')).not.toBeNull();
  });

  it('separates a checked-and-missing evidence file from a check that never ran', async () => {
    const [missing] = await badgeFor({ provenanceConfidence: 'EVIDENCE_FILE_UNAVAILABLE', evidenceFilePresent: false, hashMatches: false, computedSha256: '' });
    expect(missing).toBe('Evidence file missing');
    expect(document.querySelector('span.text-amber-300')).not.toBeNull();
  });

  it('surfaces an invalid manifest signature without discarding the rung the digest earned', async () => {
    const badges = await badgeFor({
      provenanceConfidence: 'LOCAL_DIGEST_MATCH',
      manifestPresent: true,
      isCryptographicallySigned: true,
      signatureVerification: 'INVALID',
    });
    expect(badges).toContain('Manifest signature invalid');
    expect(badges).toContain('Local file hash verified');
  });

  it('refuses to name a rung it does not recognize', async () => {
    const [badge] = await badgeFor({ provenanceConfidence: 'SOMETHING_RUST_ADDED_LATER' as ProvenanceConfidence });
    expect(badge).toBe('Unrecognized confidence state');
  });

  it('flags text-projection drift alongside an intact asset', async () => {
    const badges = await badgeFor({ textProjectionMatches: false });
    expect(badges).toContain('Local file hash verified');
    expect(badges).toContain('Quoted text drifted');
  });
});

// The log is the artifact an investigator cites. A row that predates the confidence ladder records
// no level, and reading that absence through PROVENANCE_BADGES would report an unrecognized rung --
// the same collapse of "nothing was said" into "something was said" the badge set exists to stop.
describe('provenance verification log', () => {
  it('reads the log only when asked for it', async () => {
    bridge.result = verification({});
    bridge.history = [record({})];
    render(<DocumentViewer documentId={7} portalKey="example" fileName="release.pdf" textContent="" fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());
    expect(screen.queryByText(/recorded check/)).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Verification log' }));
    await waitFor(() => expect(screen.getByText(/1 recorded check, newest first/)).toBeTruthy());
  });

  it('does not read a row that predates classification as an unrecognized rung', async () => {
    await openLog([record({ provenanceConfidence: null, recordedDigestSource: null, signatureVerification: null, predatesClassification: true })]);
    expect(screen.getByText('Logged before classification')).toBeTruthy();
    expect(screen.queryByText('Unrecognized confidence state')).toBeNull();
  });

  it('separates an unclassified row from one that predates classification', async () => {
    await openLog([record({ provenanceConfidence: null, predatesClassification: false })]);
    expect(screen.getByText('No confidence recorded')).toBeTruthy();
    expect(screen.queryByText('Logged before classification')).toBeNull();
  });

  it('states that an empty log is an absence of checks, not a result', async () => {
    await openLog([]);
    expect(screen.getByText(/No verification has been recorded for this document/)).toBeTruthy();
  });

  it('keeps each logged rung and finding distinct', async () => {
    await openLog([
      record({ verificationId: 'v2', verifiedAt: '2026-08-02T00:00:00Z', provenanceConfidence: 'DIGEST_MISMATCH', hashMatches: false, computedSha256: 'c'.repeat(64), verificationErrors: ['Stored bytes do not match the recorded digest.'] }),
      record({ verificationId: 'v1', signatureVerification: 'INVALID', textProjectionMatches: false, evidenceFilePresent: false }),
    ]);
    expect(screen.getByText('Digest mismatch')).toBeTruthy();
    expect(screen.getByText('Stored bytes do not match the recorded digest.')).toBeTruthy();
    expect(screen.getByText('Signature invalid')).toBeTruthy();
    expect(screen.getByText('No evidence file')).toBeTruthy();
    const [firstRow] = [...document.querySelectorAll('ol > li')];
    const stamp = firstRow?.querySelector('time');
    // The exact instant an investigator would cite stays recoverable verbatim, offset and all.
    expect(stamp?.getAttribute('dateTime')).toBe('2026-08-02T00:00:00Z');
    expect(stamp?.getAttribute('title')).toBe('2026-08-02T00:00:00Z');
    // What is read on screen is not the raw string.
    expect(stamp?.textContent).not.toBe('2026-08-02T00:00:00Z');
    expect(stamp?.textContent).toMatch(/2026/);
  });

  it('shows a timestamp it cannot parse exactly as it was recorded', async () => {
    await openLog([record({ verifiedAt: 'not-a-timestamp' })]);
    const stamp = document.querySelector('ol > li time');
    expect(stamp?.textContent).toBe('not-a-timestamp');
  });

  it('drops the previous document\'s log when the viewer switches documents', async () => {
    bridge.result = verification({});
    bridge.history = [record({})];
    const view = render(<DocumentViewer documentId={7} portalKey="example" fileName="a.pdf" textContent="" fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'Verification log' }));
    await waitFor(() => expect(screen.getByText(/1 recorded check/)).toBeTruthy());
    view.rerender(<DocumentViewer documentId={8} portalKey="example" fileName="b.pdf" textContent="" fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.queryByText(/recorded check/)).toBeNull());
    expect(screen.getByRole('button', { name: 'Verification log' })).toBeTruthy();
  });

  it('discards a provenance verification that resolves after the viewer moved on', async () => {
    bridge.verificationsByDocument.set(7, verification({ documentId: 7, provenanceConfidence: 'DIGEST_MISMATCH', hashMatches: false, computedSha256: 'c'.repeat(64) }));
    bridge.verificationsByDocument.set(8, verification({ documentId: 8, provenanceConfidence: 'LOCAL_DIGEST_MATCH' }));
    let releaseSeven = () => {};
    bridge.verifyGates.set(7, new Promise<void>((resolve) => {
      releaseSeven = () => resolve();
    }));

    const view = render(<DocumentViewer documentId={7} portalKey="example" fileName="a.pdf" textContent="" fileSizeFormatted="1 KB" />);
    view.rerender(<DocumentViewer documentId={8} portalKey="example" fileName="b.pdf" textContent="" fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.getByText('Local file hash verified')).toBeTruthy());

    releaseSeven();
    await settlePendingResolutions();
    // Document 7's finding must not be presented as document 8's.
    expect(screen.queryByText('Digest mismatch')).toBeNull();
    expect(screen.getByText('Local file hash verified')).toBeTruthy();
  });

  it('discards a revision scan that resolves after the viewer moved on', async () => {
    bridge.result = verification({});
    bridge.revisionProfile = revisionProfile();
    let releaseScan = () => {};
    bridge.revisionGate = new Promise<void>((resolve) => {
      releaseScan = () => resolve();
    });

    const view = render(<DocumentViewer documentId={7} portalKey="example" fileName="a.pdf" textContent="" fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'Inspect revision history' }));
    view.rerender(<DocumentViewer documentId={8} portalKey="example" fileName="b.pdf" textContent="" fileSizeFormatted="1 KB" />);

    releaseScan();
    await settlePendingResolutions();
    // Revisions carved out of document 7's bytes are not evidence about document 8.
    expect(screen.queryByText('Incremental revision inspection')).toBeNull();
  });

  it('discards a log that arrives after the viewer moved to another document', async () => {
    bridge.result = verification({});
    bridge.history = [record({ verificationId: 'stale' })];
    let release = () => {};
    bridge.gate = new Promise<void>((resolve) => {
      release = () => resolve();
    });
    const view = render(<DocumentViewer documentId={7} portalKey="example" fileName="a.pdf" textContent="" fileSizeFormatted="1 KB" />);
    await waitFor(() => expect(screen.queryByText('Verifying…')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: 'Verification log' }));
    view.rerender(<DocumentViewer documentId={8} portalKey="example" fileName="b.pdf" textContent="" fileSizeFormatted="1 KB" />);
    bridge.gate = null;
    release();
    await settlePendingResolutions();
    bridge.history = [];
    fireEvent.click(screen.getByRole('button', { name: 'Verification log' }));
    await waitFor(() => expect(screen.getByText(/No verification has been recorded/)).toBeTruthy());
  });
});
