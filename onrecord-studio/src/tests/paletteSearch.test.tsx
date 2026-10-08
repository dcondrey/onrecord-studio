// @vitest-environment jsdom
//
// The palette's object search: an answer that has not arrived is not "no match", a typed word
// matches the start of a word and never the inside of one, and the first key after the shortcut
// lands in the field.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const bridge = vi.hoisted(() => ({ holdRecords: false, findings: [] as unknown[] }));

vi.mock('@tauri-apps/api/core', () => ({
  isTauri: () => false,
  invoke: (command: string) => {
    if (command === 'query_audit_records_page') {
      return bridge.holdRecords ? new Promise(() => {}) : Promise.resolve({ records: [], nextCursor: null, total: 0 });
    }
    if (command === 'list_findings') return Promise.resolve(bridge.findings);
    return Promise.resolve([]);
  },
}));

import CommandPalette, { OPEN_COMMAND_PALETTE_EVENT } from '../components/dashboard/CommandPalette';

const finding = (id: string, headline: string) => ({ id, headline, kind: 'CLAIM_CONTRADICTS_DATA', severity: 'NOTABLE', dismissedAt: null });

afterEach(() => {
  cleanup();
  bridge.holdRecords = false;
  bridge.findings = [];
});

function open() {
  Element.prototype.scrollIntoView = () => {};
  render(<CommandPalette />);
  act(() => { window.dispatchEvent(new Event(OPEN_COMMAND_PALETTE_EVENT)); });
  return screen.getByRole('combobox') as HTMLInputElement;
}

describe('palette object search', () => {
  it('says it is searching, not that nothing matched, while the search is running', () => {
    bridge.holdRecords = true;
    const input = open();
    fireEvent.change(input, { target: { value: 'harborview' } });
    expect(screen.getByText('Searching findings, records and passages…')).toBeTruthy();
    expect(screen.queryByText(/No match/)).toBeNull();
  });

  it('matches a typed word at the start of a word, never inside one', async () => {
    bridge.findings = [finding('f-release', 'Records release schedule missed'), finding('f-lease', 'Lease agreement for 1220 Harbor Drive')];
    const input = open();
    fireEvent.change(input, { target: { value: 'lease' } });
    expect(await screen.findByRole('option', { name: /Lease agreement for 1220 Harbor Drive/ })).toBeTruthy();
    expect(screen.queryByRole('option', { name: /Records release schedule/ })).toBeNull();
  });

  it('settles on no match with a way to search every record for the text', async () => {
    const input = open();
    fireEvent.change(input, { target: { value: 'zzqx' } });
    expect(await screen.findByText('No matches for “zzqx”')).toBeTruthy();
    await waitFor(() => expect(screen.getByRole('option', { name: /Search all records for “zzqx”/ }).getAttribute('aria-selected')).toBe('true'));
  });

  it('keeps a key typed straight after the palette opens', () => {
    open();
    // The keystroke goes wherever focus is right after the open: it must already be the field.
    fireEvent.change(document.activeElement as Element, { target: { value: 's' } });
    expect((screen.getByRole('combobox') as HTMLInputElement).value).toBe('s');
  });
});
