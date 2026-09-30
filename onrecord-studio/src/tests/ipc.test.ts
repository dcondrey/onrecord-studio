// The real Tauri bridge rejects with the exact adjacently-tagged shape `AppError` serializes to
// (`{ errorType, message }`, per src-tauri/src/error.rs), never with a JS `Error` instance. Every
// other suite that mocks `invoke` rejects with `new Error('...')` instead (see
// src/tests/evidenceErrorStates.test.tsx), which only exercises the `error instanceof Error`
// branch of `formatNativeError` — a branch the production bridge does not take. These tests cover
// the branches that actually run in production: the plain-string `message` case, the structured
// `OcrRequired` object-`message` case documented in docs/ipc-contract.md, and the fallback path
// for a payload with neither shape.
import { describe, expect, it, vi } from 'vitest';

describe('formatNativeError', () => {
  it('formats a standard AppError variant from its string message, not errorType alone', async () => {
    const { formatNativeError } = await import('../ipc');
    expect(formatNativeError({ errorType: 'Storage', message: 'database unavailable' })).toBe(
      'Storage: database unavailable',
    );
  });

  it('stringifies the structured message on OcrRequired instead of rendering [object Object]', async () => {
    const { formatNativeError } = await import('../ipc');
    const wireShape = {
      errorType: 'OcrRequired',
      message: {
        path: '/tmp/scan.pdf',
        reason: 'LOW_CHARACTER_DENSITY',
        density: 0.004,
        minimum_density: 0.02,
      },
    };
    const formatted = formatNativeError(wireShape);
    expect(formatted).not.toBe('[object Object]');
    expect(formatted.startsWith('OcrRequired: ')).toBe(true);
    expect(formatted).toContain('LOW_CHARACTER_DENSITY');
    expect(formatted).toContain('minimum_density');
  });

  it('falls back to a readable message for a real Error instance', async () => {
    const { formatNativeError } = await import('../ipc');
    expect(formatNativeError(new Error('transport failed'))).toBe('transport failed');
  });

  it('falls back to a stable message for a payload with neither errorType nor message', async () => {
    const { formatNativeError } = await import('../ipc');
    expect(formatNativeError({ unrelated: true })).toBe('{"unrelated":true}');
    expect(formatNativeError(undefined)).toBe(
      'The native command failed without a usable error message.',
    );
  });
});

describe('invokeCommand', () => {
  it('wraps a rejection carrying the real wire shape in a NativeCommandError with a readable message', async () => {
    vi.resetModules();
    vi.doMock('@tauri-apps/api/core', () => ({
      invoke: () => Promise.reject({ errorType: 'License', message: 'seat limit reached' }),
    }));

    const { invokeCommand, NativeCommandError } = await import('../ipc');

    await expect(invokeCommand('get_license_status', {})).rejects.toMatchObject({
      name: 'NativeCommandError',
      command: 'get_license_status',
      message: 'License: seat limit reached',
    });
    await expect(invokeCommand('get_license_status', {})).rejects.toBeInstanceOf(NativeCommandError);

    vi.doUnmock('@tauri-apps/api/core');
    vi.resetModules();
  });

  it('resolves with the backend result on success', async () => {
    vi.resetModules();
    vi.doMock('@tauri-apps/api/core', () => ({
      invoke: () => Promise.resolve(true),
    }));

    const { invokeCommand } = await import('../ipc');

    await expect(invokeCommand('cancel_embedding_index', {})).resolves.toBe(true);

    vi.doUnmock('@tauri-apps/api/core');
    vi.resetModules();
  });
});
