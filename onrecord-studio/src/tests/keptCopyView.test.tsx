// @vitest-environment jsdom
//
// A kept copy is the bytes one source check received. A read for check A that resolves after the
// view moved to check B would show A's bytes as B's.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import KeptCopyView from '../components/documents/KeptCopyView';

const bridge = vi.hoisted(() => ({ gates: new Map<number, Promise<void>>() }));

function frame(text: string): ArrayBuffer {
  const bytes = new TextEncoder().encode(text);
  const header = new TextEncoder().encode(JSON.stringify({ byteLength: bytes.length, sha256: 'a'.repeat(64), recordedSha256: 'a'.repeat(64), matchesRecorded: true }));
  const out = new Uint8Array(bytes.length + header.length + 4);
  out.set(bytes, 0);
  out.set(header, bytes.length);
  new DataView(out.buffer).setUint32(bytes.length + header.length, header.length, false);
  return out.buffer;
}

vi.mock('../components/documents/pdfjsLoader', () => ({
  STANDARD_FONT_URL: '',
  WASM_URL: '',
  canvasScale: () => 1,
  loadPdfjs: () => Promise.reject(new Error('pdf.js is not used for text copies')),
}));

vi.mock('../ipc', () => ({
  formatNativeError: (error: unknown) => String(error),
  invokeCommand: (command: string, args?: { checkId?: number }) => {
    if (command !== 'read_source_check_copy') return Promise.reject(new Error(`unexpected command ${command}`));
    const checkId = args?.checkId ?? 0;
    const answer = frame(`bytes kept by check ${checkId}`);
    const gate = bridge.gates.get(checkId);
    return gate ? gate.then(() => answer) : Promise.resolve(answer);
  },
}));

afterEach(() => {
  cleanup();
  bridge.gates.clear();
});

describe('kept copy view', () => {
  it('discards a kept copy that resolves after the view moved to another check', async () => {
    let release = () => {};
    bridge.gates.set(1, new Promise<void>((resolve) => { release = () => resolve(); }));
    const view = render(<KeptCopyView documentId={7} portalKey="example" checkId={1} />);
    view.rerender(<KeptCopyView documentId={7} portalKey="example" checkId={2} />);
    await screen.findByText('bytes kept by check 2');
    release();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText('bytes kept by check 1')).toBeNull();
    expect(screen.getByText('bytes kept by check 2')).toBeTruthy();
  });
});
