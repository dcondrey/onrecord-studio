// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { CalendarMeeting, FindingCitation, MeetingCalendar, MeetingDocument } from '../types';
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../ipc', async (importOriginal) => ({ ...(await importOriginal<typeof import('../ipc')>()), invokeCommand: bridge.invoke }));
import MeetingsView from '../components/dashboard/MeetingsView';

const cite = (quote: string, start: number | null = 0): FindingCitation => ({
  portalKey: 'city', documentId: 2, characterStart: start, characterEnd: start === null ? null : start + quote.length, quote, label: 'agenda.pdf',
});

function agendaFile(overrides: Partial<MeetingDocument> = {}): MeetingDocument {
  return {
    role: 'AGENDA', portalKey: 'city', documentId: 2, label: 'agenda.pdf', sourceUrl: null,
    posting: { state: 'NOT_RECORDED', reason: 'NO_FETCH_RECORD' }, minutesBeforeMeeting: null, differingFetches: 0, ...overrides,
  };
}

function meeting(id: string, body: string, overrides: Partial<CalendarMeeting> = {}): CalendarMeeting {
  return {
    id, jurisdiction: 'City of San Diego', body, bodyCitation: cite(body), date: '2026-10-01', dateCitation: cite('THURSDAY, OCTOBER 1, 2026'),
    dateNote: null, timing: 'UPCOMING', startTime: null, documents: [agendaFile()], revisions: [],
    commentDeadlines: [], commentInstructions: [], suggestions: [], ...overrides,
  };
}

function calendar(meetings: CalendarMeeting[], postingCounts = { agendaFilesWithBothTimes: 0, under72HoursBeforeStart: 0, afterMeetingStart: 0 }): MeetingCalendar {
  return { today: '2026-09-30', meetings, postingCounts, limitations: ['Bounded lookup.'] };
}

function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>((done) => { resolve = done; }); return { promise, resolve }; }
beforeEach(() => { bridge.invoke.mockReset(); });
afterEach(cleanup);

describe('meetings view', () => {
  it('discards an earlier calendar read that resolves after a newer one', async () => {
    const first = deferred<MeetingCalendar>();
    const second = deferred<MeetingCalendar>();
    bridge.invoke.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    render(<MeetingsView />);
    fireEvent.click(screen.getByRole('button', { name: 'Reading documents…' }));
    await act(async () => second.resolve(calendar([meeting('new', 'Planning Commission')])));
    await act(async () => first.resolve(calendar([meeting('old', 'Board of Supervisors')])));
    expect(screen.getByText('Planning Commission')).toBeTruthy();
    expect(screen.queryByText('Board of Supervisors')).toBeNull();
  });

  it('never shows a guessed date or deadline, and opens a cited draft', async () => {
    const undated = meeting('undated', 'Board of Education', { date: null, dateCitation: null, dateNote: 'No weekday and full date appears in the heading or file name.', timing: 'DATE_NOT_RECORDED' });
    const upcoming = meeting('council', 'City Council', {
      suggestions: [{
        id: 's1', agendaItem: 'Item 605', title: 'Riverbed Encampment Resolution Funding', itemCitation: cite('Item 605: Riverbed'),
        reasons: [{ kind: 'EVIDENCE_GAP', basis: 'STATED', reason: 'An evidence gap is open.', matched: 'Amendment', citation: cite('First Amendment to the Agreement', 120), findingId: null, gapKey: 'g', claimCheck: null }],
        comment: { text: 'Public comment: City Council\nComment deadline: not recorded in the collected documents.', bases: ['STATED'], deadline: null, instructions: [] },
      }],
    });
    bridge.invoke.mockResolvedValue(calendar([upcoming, undated]));
    render(<MeetingsView />);
    await screen.findByText('City Council');
    const undatedCard = screen.getByRole('article', { name: 'Board of Education date not recorded' });
    expect(undatedCard.textContent).toContain('Date not recorded');
    expect(undatedCard.textContent).toContain('No weekday and full date');
    expect(screen.getAllByText('not recorded').length).toBeGreaterThan(0);
    fireEvent.click(screen.getByText('Item 605: Riverbed Encampment Resolution Funding'));
    expect(screen.getByText('Evidence gap (untested)')).toBeTruthy();
    expect(screen.getByText('Basis: Stated')).toBeTruthy();
    expect(screen.getByText(/characters 120–152/)).toBeTruthy();
    expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toContain('Comment deadline: not recorded');
  });

  it('shows a header time only where one is recorded, and counts without concluding', async () => {
    const posted = meeting('posted', 'Privacy Advisory Board', {
      startTime: '14:00',
      documents: [agendaFile({ posting: { state: 'RECORDED', lastModified: '2026-09-28T21:00:00Z' }, minutesBeforeMeeting: 4319 })],
    });
    const docket = meeting('docket', 'Audit Committee', {
      date: '2026-10-08', startTime: '09:00',
      documents: [agendaFile({ documentId: 3, label: 'docket.html', posting: { state: 'NOT_RECORDED', reason: 'REQUEST_TIME_HOST' }, differingFetches: 2 })],
    });
    bridge.invoke.mockResolvedValue(calendar([posted, docket], { agendaFilesWithBothTimes: 1, under72HoursBeforeStart: 1, afterMeetingStart: 0 }));
    render(<MeetingsView />);
    const card = await screen.findByRole('article', { name: 'Privacy Advisory Board 2026-10-01' });
    expect(card.textContent).toContain('Posted (per the file’s Last-Modified header): 2026-09-28 21:00 UTC · 71 h 59 min before the stated start');
    const docketCard = screen.getByRole('article', { name: 'Audit Committee 2026-10-08' });
    expect(docketCard.textContent).toContain('Posted: not recorded (this host reports the time of the request as Last-Modified)');
    expect(docketCard.textContent).not.toContain('UTC');
    expect(docketCard.textContent).toContain('2 fetches of this address returned different bytes');
    const counts = screen.getByLabelText('Posting time counts').textContent ?? '';
    expect(counts).toContain('1 of 1 agenda files');
    expect(counts).toContain('posted less than 72 hours before the meeting, per the file’s Last-Modified header');
    expect(document.body.textContent).not.toMatch(/\blate\b|violat|Brown Act|failed/i);
  });

  it('lists what changed between two copies of an agenda, and says when nothing did', async () => {
    const revised = meeting('revised', 'City Council', {
      documents: [agendaFile({ label: 'agenda.pdf' }), agendaFile({ documentId: 4, label: 'agenda-revised.pdf' })],
      revisions: [{
        earlierDocumentId: 2, earlierLabel: 'agenda.pdf', laterDocumentId: 4, laterLabel: 'agenda-revised.pdf', orderBasis: 'LAST_MODIFIED',
        changes: [
          { kind: 'REMOVED', item: 'Item 601', before: 'Approval of Council Minutes.', after: null },
          { kind: 'REWORDED', item: 'Item 605', before: 'Agreement with Provider A.', after: 'Agreement with Provider B.' },
          { kind: 'ADDED', item: 'Item 330', before: null, after: 'Emergency Repair Contract.' },
        ],
        titlesNotCompared: ['Item 606'],
      }],
    });
    const same = meeting('same', 'Planning Commission', {
      date: '2026-10-08',
      revisions: [{ earlierDocumentId: 5, earlierLabel: 'a.pdf', laterDocumentId: 6, laterLabel: 'b.pdf', orderBasis: 'COLLECTION_ORDER', changes: [], titlesNotCompared: [] }],
    });
    bridge.invoke.mockResolvedValue(calendar([revised, same]));
    render(<MeetingsView />);
    const card = await screen.findByRole('article', { name: 'City Council 2026-10-01' });
    const text = card.textContent ?? '';
    expect(text).toContain('agenda.pdf → agenda-revised.pdf (ordered by Last-Modified header)');
    expect(text).toContain('Removed: Item 601 “Approval of Council Minutes.”');
    expect(text).toContain('Title reworded: Item 605Earlier: “Agreement with Provider A.”Later: “Agreement with Provider B.”');
    expect(text).toContain('Added: Item 330 “Emergency Repair Contract.”');
    expect(text).toContain('Wording not compared (the title has no proven end, or the item number repeats): Item 606.');
    const unchanged = screen.getByRole('article', { name: 'Planning Commission 2026-10-08' }).textContent ?? '';
    expect(unchanged).toContain('No item was added, removed or changed between these copies.');
    expect(unchanged).toContain('ordered by when the app collected them');
  });
});
