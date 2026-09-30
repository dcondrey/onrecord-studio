// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';

const service = vi.hoisted(() => ({ checks: [] as Array<(value: unknown) => void>, tauri: true }));

vi.mock('../components/updates/updateService', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../components/updates/updateService')>();
  return {
    ...actual,
    isTauriRuntime: () => service.tauri,
    checkForUpdate: vi.fn(() => new Promise((resolve) => service.checks.push(resolve))),
  };
});

import UpdatePrompt from '../components/updates/UpdatePrompt';
import { useUpdateCheck } from '../components/updates/useUpdateCheck';
import { checkForUpdate, shortNotes, NOTES_LIMIT } from '../components/updates/updateService';

const offer = (version: string, install = vi.fn(async () => undefined)) => ({ version, notes: `notes ${version}`, install });

beforeEach(() => {
  service.checks.length = 0;
  service.tauri = true;
  vi.mocked(checkForUpdate).mockClear();
});
afterEach(cleanup);

describe('UpdatePrompt', () => {
  it('is inert outside Tauri', () => {
    service.tauri = false;
    const { container } = render(<UpdatePrompt />);
    expect(checkForUpdate).not.toHaveBeenCalled();
    expect(container.innerHTML).toBe('');
  });

  it('shows the version and notes, and hides for that version after Later', async () => {
    const { container } = render(<UpdatePrompt />);
    await act(async () => service.checks[0](offer('1.1.0')));
    expect(screen.getByText('Update available: 1.1.0')).toBeTruthy();
    expect(screen.getByText('notes 1.1.0')).toBeTruthy();
    fireEvent.click(screen.getByText('Later'));
    expect(container.innerHTML).toBe('');
  });

  it('shows an install failure plainly and allows a retry', async () => {
    const install = vi.fn().mockRejectedValueOnce(new Error('signature mismatch')).mockResolvedValue(undefined);
    render(<UpdatePrompt />);
    await act(async () => service.checks[0](offer('1.1.0', install)));
    await act(async () => fireEvent.click(screen.getByText('Install and restart')));
    expect(screen.getByRole('alert').textContent).toContain('signature mismatch');
    await act(async () => fireEvent.click(screen.getByText('Retry install')));
    expect(install).toHaveBeenCalledTimes(2);
  });
});

describe('overlapping checks', () => {
  function Probe() {
    const { state, check } = useUpdateCheck(1_000_000_000);
    return (
      <>
        <button onClick={() => void check()}>again</button>
        <output>{state.phase === 'available' ? state.update.version : state.phase}</output>
      </>
    );
  }

  it('discards a slower, superseded result', async () => {
    render(<Probe />);
    fireEvent.click(screen.getByText('again'));
    expect(service.checks).toHaveLength(2);
    await act(async () => service.checks[1](null));
    await act(async () => service.checks[0](offer('9.9.9')));
    expect(screen.getByRole('status').textContent).toBe('idle');
  });
});

describe('shortNotes', () => {
  it('bounds long notes at the limit', () => {
    expect(shortNotes('x'.repeat(NOTES_LIMIT))).toHaveLength(NOTES_LIMIT);
    expect(shortNotes('x'.repeat(NOTES_LIMIT + 1)).endsWith('...')).toBe(true);
    expect(shortNotes(undefined)).toBe('');
  });
});
