// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { open } from '@tauri-apps/plugin-dialog';
import { invokeCommand } from '../ipc';
import DataPackImport from '../components/dashboard/DataPackImport';
import type { DataPackSummary } from '../types';

vi.mock('../ipc', () => ({ invokeCommand: vi.fn(), formatNativeError: String }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

const summary = (pack: string): DataPackSummary => ({
  pack,
  packVersion: '2026.10.6',
  schemaVersion: '1.0',
  builtAt: '2026-10-06T22:59:25Z',
  signerKeyId: 'ab'.repeat(32),
  licenceId: 'ODC-PDDL-1.0',
  licenceUrl: 'https://opendefinition.org/licenses/odc-pddl/',
  disclaimer: 'Informational only.',
  tables: [],
});

describe('DataPackImport', () => {
  it('never shows a verification that resolves after another folder was chosen', async () => {
    const pending = new Map<string, (value: DataPackSummary) => void>();
    vi.mocked(invokeCommand).mockImplementation(((_command: string, args: { path: string }) =>
      new Promise((resolve) => pending.set(args.path, resolve))) as unknown as typeof invokeCommand);
    vi.mocked(open).mockResolvedValueOnce('/packs/first').mockResolvedValueOnce('/packs/second');
    render(<DataPackImport />);
    const choose = screen.getByRole('button', { name: 'Choose pack folder' });

    await act(async () => { fireEvent.click(choose); });
    await act(async () => { fireEvent.click(choose); });
    await act(async () => { pending.get('/packs/second')?.(summary('second-pack')); });
    await act(async () => { pending.get('/packs/first')?.(summary('first-pack')); });

    expect(screen.getByText(/second-pack/)).toBeTruthy();
    expect(screen.queryByText(/first-pack/)).toBeNull();
  });
});
