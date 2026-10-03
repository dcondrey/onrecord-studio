// @vitest-environment jsdom
//
// Opening the app must contact no source. The plan asks first, preselects only what the last
// session left unfinished, and starts exactly the chosen sources for the typed subject.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { CollectionPlanSource } from '../types';

const bridge = vi.hoisted(() => ({
  calls: [] as { command: string; args: unknown }[],
  sources: [] as unknown[],
}));

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }));
vi.mock('../ipc', () => ({
  formatNativeError: (reason: unknown) => String(reason),
  invokeCommand: (command: string, args: unknown) => {
    bridge.calls.push({ command, args });
    if (command === 'get_collection_plan') return Promise.resolve({ sources: bridge.sources });
    if (command === 'start_collection') return Promise.resolve([]);
    if (command === 'start_data_source') return Promise.resolve({ source: { displayName: 'City records' } });
    return Promise.resolve(null);
  },
}));

import CollectionPlanDialog, { lastSubject } from '../components/dashboard/CollectionPlanDialog';

function source(portalKey: string, hasUnfinishedWork: boolean, subject: string | null = null): CollectionPlanSource {
  return {
    portalKey,
    displayName: portalKey,
    canonicalUrl: `https://${portalKey}.example.gov/`,
    sourceKind: 'NEXT_REQUEST',
    hasUnfinishedWork,
    subject,
  };
}

afterEach(() => {
  cleanup();
  bridge.calls = [];
});

describe('CollectionPlanDialog', () => {
  it('starts nothing on open and preselects only unfinished sources', async () => {
    bridge.sources = [source('city', true, 'encampments'), source('county', false)];
    render(<CollectionPlanDialog />);
    await screen.findByText('What should this session collect?');
    expect(bridge.calls.map((call) => call.command)).toEqual(['get_collection_plan']);
    const boxes = screen.getAllByRole('checkbox') as HTMLInputElement[];
    expect(boxes.map((box) => box.checked)).toEqual([true, false]);
    expect((screen.getAllByRole('textbox')[0] as HTMLInputElement).value).toBe('encampments');
  });

  it('starts exactly the chosen sources for the typed subject', async () => {
    bridge.sources = [source('city', true), source('county', false)];
    render(<CollectionPlanDialog />);
    await screen.findByText('What should this session collect?');
    fireEvent.click(screen.getAllByRole('checkbox')[1]);
    fireEvent.change(screen.getAllByRole('textbox')[0], { target: { value: '  police overtime ' } });
    fireEvent.click(screen.getByRole('button', { name: /Start collecting from 2 sources/ }));
    await waitFor(() => expect(screen.queryByText('What should this session collect?')).toBeNull());
    expect(bridge.calls.at(-1)).toEqual({
      command: 'start_collection',
      args: { sourceKeys: ['city', 'county'], subject: 'police overtime' },
    });
  });

  it('starts an added source scoped to the typed subject', async () => {
    bridge.sources = [source('city', false)];
    render(<CollectionPlanDialog />);
    await screen.findByText('What should this session collect?');
    const [subjectField, urlField] = screen.getAllByRole('textbox');
    fireEvent.change(subjectField, { target: { value: 'encampment' } });
    fireEvent.change(urlField, { target: { value: 'https://city.nextrequest.com' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add and collect' }));
    await screen.findByText('Collecting from City records.');
    const call = bridge.calls.at(-1) as { command: string; args: { url: string; guidance: { subject: string } } };
    expect(call.command).toBe('start_data_source');
    expect(call.args.url).toBe('https://city.nextrequest.com');
    expect(call.args.guidance.subject).toBe('encampment');
  });

  it('stays closed when there are no saved sources', async () => {
    bridge.sources = [];
    render(<CollectionPlanDialog />);
    await waitFor(() => expect(bridge.calls).toHaveLength(1));
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('starts the subject field from the subject most sources last used', () => {
    expect(lastSubject([source('a', false, 'x'), source('b', false, 'y'), source('c', false, 'y')])).toBe('y');
    expect(lastSubject([source('a', false)])).toBe('');
  });
});
