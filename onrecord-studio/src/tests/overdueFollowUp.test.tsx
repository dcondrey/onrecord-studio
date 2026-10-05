// @vitest-environment jsdom
//
// A letter drafted for request A that resolves after the viewer moved to request B would state
// A's received date, deadline and day count as facts about B.
import { afterEach, describe, expect, test, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import * as ipc from '../ipc';
import OverdueFollowUp from '../components/dashboard/OverdueFollowUp';
import type { OverdueFollowUpDraft } from '../types';

const option = { portalKey: 'sd', agencyId: 7, agencyName: 'Purchasing', closedRequests: 12 };

function letter(text: string, withAgency = false): OverdueFollowUpDraft {
  return {
    artifact: {
      id: 'a1',
      kind: 'FOLLOWUP_REQUEST',
      subjectFindingId: 'casework:case-a:r1',
      jurisdiction: 'US-CA',
      templateId: 'cpra_overdue_followup_v1',
      generatedText: text,
      disclosure: null,
      citedAuthorities: [{ statute: 'California Government Code § 7922.535', quote: null }],
      evidenceBasis: 'DEADLINE_PASSED_WITHOUT_RECORDED_RESPONSE',
      confidence: { kind: 'UNASSESSED' },
      citations: [],
      unknownFacts: [],
      generatedAt: '2026-10-05T00:00:00Z',
    },
    agencyRecord: withAgency
      ? { ...option, sourcedDeterminations: 3, lateSourcedDeterminations: 2, noRecordsClosures: 0, noRecordsWithRecordedEarlierRelease: 0, period: ['2026-01-02', '2026-01-13'] }
      : null,
    agencyParagraphIncluded: withAgency,
    agencyOptions: [option],
  };
}

const draftButton = () => screen.getByRole('button', { name: /draft overdue follow-up letter/i });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('<OverdueFollowUp />', () => {
  test('a letter for request r1 that resolves after the viewer moved to request r2 is never shown', async () => {
    let resolveFirst: (value: OverdueFollowUpDraft) => void = () => {};
    vi.spyOn(ipc, 'invokeCommand').mockImplementation(
      () => new Promise<OverdueFollowUpDraft>((resolve) => (resolveFirst = resolve)) as never,
    );
    const { rerender } = render(<OverdueFollowUp caseId="case-a" requestId="r1" dueOn="2026-03-12" />);
    fireEvent.click(draftButton());
    rerender(<OverdueFollowUp caseId="case-a" requestId="r2" dueOn="2026-04-01" />);
    resolveFirst(letter('ABOUT REQUEST R1'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(screen.queryByLabelText(/drafted artifact text/i)).toBeNull();
    expect(draftButton()).toHaveProperty('disabled', false);
  });

  test('a refusal before the deadline has passed is shown as the alert and no letter text exists', async () => {
    vi.spyOn(ipc, 'invokeCommand').mockRejectedValue(
      new Error('No overdue follow-up letter can be drafted: the determination is due on 2026-03-12 and that date has not passed as of 2026-03-12'),
    );
    render(<OverdueFollowUp caseId="case-a" requestId="r1" dueOn="2026-03-12" />);
    expect(screen.queryByLabelText(/drafted artifact text/i)).toBeNull();
    fireEvent.click(draftButton());
    expect((await screen.findByRole('alert')).textContent).toContain('has not passed as of 2026-03-12');
    expect(screen.queryByLabelText(/drafted artifact text/i)).toBeNull();
  });

  test('choosing an agency redrafts with that portal and id, and choosing none drafts without it', async () => {
    const invoke = vi.spyOn(ipc, 'invokeCommand').mockImplementation(((_: string, args: { agency: unknown }) =>
      Promise.resolve(args.agency ? letter('WITH RECORD', true) : letter('PLAIN'))) as never);
    render(<OverdueFollowUp caseId="case-a" requestId="r1" dueOn="2026-03-12" />);
    fireEvent.click(draftButton());
    const select = await screen.findByLabelText(/agency record from your library/i);
    expect(invoke).toHaveBeenLastCalledWith('draft_overdue_follow_up', { caseId: 'case-a', requestId: 'r1', agency: null });
    fireEvent.change(select, { target: { value: 'sd\u001f7' } });
    await waitFor(() => expect(screen.getByLabelText(/drafted artifact text/i)).toHaveProperty('value', 'WITH RECORD'));
    expect(invoke).toHaveBeenLastCalledWith('draft_overdue_follow_up', { caseId: 'case-a', requestId: 'r1', agency: { portalKey: 'sd', agencyId: 7 } });
    expect(screen.getByText(/counted from 12 closed requests in your library/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText(/agency record from your library/i), { target: { value: '' } });
    await waitFor(() => expect(screen.getByLabelText(/drafted artifact text/i)).toHaveProperty('value', 'PLAIN'));
  });
});
