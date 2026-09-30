// @vitest-environment jsdom
//
// useC2PA is one of this codebase's two canonical implementations of the "discard on arrival"
// invariant: an async verification result must never be written into state once the identity it
// was requested for is no longer current (see CLAUDE.md). This file pins both halves of that
// contract directly against the hook, without going through DocumentViewer: the supersession
// guard on the eventual result, and the requirement that a stale manifest never survives visibly
// into the next request's own loading window.
import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { useC2PA } from '../hooks/useC2PA';
import * as ipc from '../ipc';
import type { DocumentProvenanceVerification } from '../types';

function verification(documentId: number): DocumentProvenanceVerification {
  return {
    documentId,
    portalKey: 'example',
    expectedSha256: 'a'.repeat(64),
    computedSha256: 'a'.repeat(64),
    evidenceFilePresent: true,
    hashMatches: true,
    recordedDigestSource: 'INGESTION_RECORD',
    manifestPresent: true,
    isCryptographicallySigned: false,
    signatureVerification: 'NOT_ATTEMPTED',
    signatureIssuer: null,
    signingCertificateSerialNumber: null,
    signingTimestamp: null,
    assertionCount: 1,
    invalidAssertionCount: 0,
    claimGraph: {
      activeManifestLabel: `urn:c2pa:doc-${documentId}`,
      claims: [{
        manifestLabel: `urn:c2pa:doc-${documentId}`,
        isActive: true,
        title: `Document ${documentId} claim`,
        format: null,
        instanceId: null,
        claimGenerator: null,
        assertionLabels: [],
      }],
      ingredients: [],
      validationStatuses: [],
      certificateChainTrusted: false,
      certificateChainDiagnostics: [],
      truncated: false,
    },
    textProjectionSha256: 'b'.repeat(64),
    textProjectionMatches: true,
    provenanceConfidence: 'LOCAL_DIGEST_MATCH',
    chainOfCustodyConfirmed: false,
    verificationErrors: [],
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('useC2PA', () => {
  it('discards a verification that resolves after a newer request has begun', async () => {
    let releaseSeven: (value: DocumentProvenanceVerification) => void = () => {};
    const sevenPromise = new Promise<DocumentProvenanceVerification>((resolve) => {
      releaseSeven = resolve;
    });
    vi.spyOn(ipc, 'invokeCommand').mockImplementation(((command: string, args: { documentId: number }) => {
      if (command !== 'verify_document_provenance') return Promise.reject(new Error(`unexpected command ${command}`));
      return args.documentId === 7 ? sevenPromise : Promise.resolve(verification(8));
    }) as typeof ipc.invokeCommand);

    const { result } = renderHook(() => useC2PA());

    let sevenCall!: Promise<DocumentProvenanceVerification | null>;
    await act(async () => {
      sevenCall = result.current.verifyAssetProvenance(7, 'example');
    });
    await act(async () => {
      await result.current.verifyAssetProvenance(8, 'example');
    });
    expect(result.current.activeManifest?.documentId).toBe(8);

    await act(async () => {
      releaseSeven(verification(7));
      await sevenCall;
    });

    // Document 7's answer arrived last but is no longer current: it must not overwrite 8's.
    expect(result.current.activeManifest?.documentId).toBe(8);
  });

  it('clears the previous manifest before the next verification resolves, so it never renders under the new request', async () => {
    let releaseEight: (value: DocumentProvenanceVerification) => void = () => {};
    const eightPromise = new Promise<DocumentProvenanceVerification>((resolve) => {
      releaseEight = resolve;
    });
    vi.spyOn(ipc, 'invokeCommand').mockImplementation(((command: string, args: { documentId: number }) => {
      if (command !== 'verify_document_provenance') return Promise.reject(new Error(`unexpected command ${command}`));
      return args.documentId === 7 ? Promise.resolve(verification(7)) : eightPromise;
    }) as typeof ipc.invokeCommand);

    const { result } = renderHook(() => useC2PA());

    await act(async () => {
      await result.current.verifyAssetProvenance(7, 'example');
    });
    expect(result.current.activeManifest?.documentId).toBe(7);

    let eightCall!: Promise<DocumentProvenanceVerification | null>;
    act(() => {
      eightCall = result.current.verifyAssetProvenance(8, 'example');
    });

    // Document 8's check is still in flight. Document 7's manifest must already be gone: a
    // consumer that renders `activeManifest` without checking `isValidating` (the claim-graph
    // panel in DocumentViewer.tsx) would otherwise show document 7's claims while document 8
    // is on screen and its own check has not finished.
    expect(result.current.activeManifest).toBeNull();

    await act(async () => {
      releaseEight(verification(8));
      await eightCall;
    });
    expect(result.current.activeManifest?.documentId).toBe(8);
  });
});
