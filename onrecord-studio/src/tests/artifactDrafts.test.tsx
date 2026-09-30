// @vitest-environment jsdom
import { afterEach, describe, expect, test, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent, cleanup } from '@testing-library/react';
import ArtifactDrafts from '../components/dashboard/ArtifactDrafts';
import * as ipc from '../ipc';
import type { DraftedArtifact } from '../types';

const followupDraft: DraftedArtifact = {
  id: 'artifact-abc',
  kind: 'FOLLOWUP_REQUEST',
  subjectFindingId: 'finding-xyz',
  jurisdiction: 'US-CA',
  templateId: 'cpra_followup_generic_v1',
  generatedText:
    'Subject: Follow-up request\n\nDate: 2026-09-29\nTo: [AGENCY NAME]\nRe: Follow-up to [YOUR REQUEST NUMBER]\n\nUnder the California Public Records Act ...',
  citedAuthorities: [{ statute: 'Cal. Gov. Code § 7920.000', quote: null }],
  generatedAt: '2026-09-29T12:00:00Z',
};

const appealDraft: DraftedArtifact = {
  ...followupDraft,
  id: 'artifact-appeal',
  kind: 'APPEAL',
  templateId: 'cpra_appeal_no_records_v1',
  generatedText: 'Subject: Appeal\n\nBody of the appeal ...',
};

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('<ArtifactDrafts />', () => {
  test('does not fetch on mount; renders both action buttons and no draft', () => {
    const spy = vi.spyOn(ipc, 'invokeCommand').mockResolvedValue(followupDraft);
    render(<ArtifactDrafts findingId="finding-xyz" />);
    expect(screen.getByRole('button', { name: /draft follow-up/i })).toBeTruthy();
    expect(screen.getByRole('button', { name: /draft appeal/i })).toBeTruthy();
    // No inline draft rendered until a button is clicked.
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(spy).not.toHaveBeenCalled();
  });

  test('clicking "Draft follow-up request" invokes the backend and shows the draft editable', async () => {
    const spy = vi.spyOn(ipc, 'invokeCommand').mockResolvedValue(followupDraft);
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(screen.getByRole('button', { name: /draft follow-up/i }));
    await waitFor(() => {
      expect(screen.getByLabelText(/drafted artifact text/i)).toBeTruthy();
    });
    expect(spy).toHaveBeenCalledWith('draft_followup_from_finding', {
      findingId: 'finding-xyz',
    });
    const textarea = screen.getByLabelText(/drafted artifact text/i) as HTMLTextAreaElement;
    expect(textarea.value).toContain('Under the California Public Records Act');
  });

  test('clicking "Draft appeal" routes to the appeal command', async () => {
    const spy = vi.spyOn(ipc, 'invokeCommand').mockResolvedValue(appealDraft);
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(screen.getByRole('button', { name: /draft appeal/i }));
    await waitFor(() => {
      expect(screen.getByLabelText(/drafted artifact text/i)).toBeTruthy();
    });
    expect(spy).toHaveBeenCalledWith('draft_appeal_from_finding', {
      findingId: 'finding-xyz',
    });
  });

  test('user edits to the draft are preserved on subsequent Copy / Download calls', async () => {
    vi.spyOn(ipc, 'invokeCommand').mockResolvedValue(followupDraft);
    // Fake clipboard so the test can assert what was copied.
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });

    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(screen.getByRole('button', { name: /draft follow-up/i }));
    const textarea = (await screen.findByLabelText(/drafted artifact text/i)) as HTMLTextAreaElement;

    // User edits.
    fireEvent.change(textarea, { target: { value: 'EDITED BODY' } });
    fireEvent.click(screen.getByRole('button', { name: /^copy$/i }));
    await waitFor(() => {
      expect(writeText).toHaveBeenCalledWith('EDITED BODY');
    });
  });

  test('backend errors surface in an alert without wiping the button state', async () => {
    vi.spyOn(ipc, 'invokeCommand').mockRejectedValue(new Error('No template for this kind'));
    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(screen.getByRole('button', { name: /draft appeal/i }));
    const alert = await screen.findByRole('alert');
    expect(alert.textContent).toContain('No template for this kind');
    // Buttons are re-enabled (busy state cleared).
    const button = screen.getByRole('button', { name: /draft appeal/i }) as HTMLButtonElement;
    expect(button.disabled).toBe(false);
  });
});

describe('gate proof (removal of staleness guard would corrupt the draft)', () => {
  beforeEach(() => {
    // Fresh mocks per test.
  });

  test('when a slow followup resolves after the user clicks appeal, the appeal wins', async () => {
    let resolveFollowup: (value: DraftedArtifact) => void = () => {};
    const followupPromise = new Promise<DraftedArtifact>((resolve) => {
      resolveFollowup = resolve;
    });
    const spy = vi
      .spyOn(ipc, 'invokeCommand')
      .mockImplementationOnce(() => followupPromise as Promise<DraftedArtifact>)
      .mockImplementationOnce(() => Promise.resolve(appealDraft));

    render(<ArtifactDrafts findingId="finding-xyz" />);
    fireEvent.click(screen.getByRole('button', { name: /draft follow-up/i }));
    fireEvent.click(screen.getByRole('button', { name: /draft appeal/i }));
    // Now let the slow followup finally arrive — the gate should discard it.
    resolveFollowup(followupDraft);
    const textarea = (await screen.findByLabelText(/drafted artifact text/i)) as HTMLTextAreaElement;
    await waitFor(() => {
      expect(textarea.value).toContain(appealDraft.generatedText);
    });
    // The followup body must not have overwritten the appeal body.
    expect(textarea.value).not.toContain(followupDraft.generatedText);
    expect(spy).toHaveBeenCalledTimes(2);
  });
});
