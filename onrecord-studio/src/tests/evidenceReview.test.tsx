// @vitest-environment jsdom
import { useLayoutEffect } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import EvidenceReviewPanel from '../components/dashboard/EvidenceReviewPanel';
import type { ReviewWorkspace } from '../types';
import { reviewPacket } from '../utils/review';
import { invokeCommand } from '../ipc';
vi.mock('../ipc', () => ({ invokeCommand: vi.fn() }));
const invoke = vi.mocked(invokeCommand);
function workspace(id: string): ReviewWorkspace {
  return { report: { id, topic: 'police', createdAt: '2026-09-26', model: 'fixture', inputTokens: 0, outputTokens: 0, investigationLog: [], evidence: [{ id: 'e1', label: 'Actual payments', sourceUrl: 'https://www.sandiego.gov/actuals.csv', sourceSha256: 'a'.repeat(64), portalKey: 'city', documentId: 1, requestId: 1, characterOffset: 0, text: 'Actual payments are 100.25; budget 90.00', sql: null, sourceReferences: [], snapshotProvenance: [], limitations: '' }], synthesis: { title: id, summary: 'A test case', findings: [{ title: `${id} finding`, interpretation: 'A variance requires the revised budget.', whyItMatters: 'Scope and authorization', citations: [{ evidenceId: 'e1', quote: 'Actual payments are 100.25' }], counterEvidence: [], alternativeExplanations: ['An amendment may explain the difference.'], nextTest: 'Obtain revised appropriation.' }], questions: [], requests: [], meetings: [], limitations: [] } }, reviews: [], challenges: [{ findingIndex: 0, title: `${id} finding`, independentSources: 1, checks: ['Exact quotation; interpretation requires review.'], blockers: ['Verify retained source identity.'], nextTest: 'Obtain revised appropriation.' }], replays: [], relationships: [], connections: [], identityWarnings: [] };
}
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; }
beforeEach(() => { vi.resetAllMocks(); });
afterEach(cleanup);
describe('accountable evidence review', () => {
  it('discards a delayed review workspace after another report is selected', async () => {
    const old = deferred<ReviewWorkspace>(); const next = workspace('B');
    invoke.mockImplementation(((command: string, args: { reportId: string }) => command === 'get_review_workspace' && args.reportId === 'A' ? old.promise : Promise.resolve(next)) as typeof invokeCommand);
    const view = render(<EvidenceReviewPanel reportId="A" />);
    view.rerender(<EvidenceReviewPanel reportId="B" />);
    await screen.findByText('B finding');
    await act(async () => old.resolve(workspace('A')));
    expect(screen.queryByText('A finding')).toBeNull();
    expect(screen.getByText('B finding')).toBeTruthy();
  });
  it('does not publish an old replay result into a different report', async () => {
    const old = deferred<ReviewWorkspace>();
    invoke.mockImplementation(((command: string, args: { reportId: string }) => command === 'replay_review_evidence' ? old.promise : Promise.resolve(workspace(args.reportId))) as typeof invokeCommand);
    const view = render(<EvidenceReviewPanel reportId="A" />); await screen.findByText('A finding');
    fireEvent.click(screen.getByText('Verify sources and replay calculations'));
    view.rerender(<EvidenceReviewPanel reportId="B" />); await screen.findByText('B finding');
    await act(async () => old.resolve(workspace('A')));
    expect(screen.queryByText('A finding')).toBeNull();
    expect(screen.getByText('B finding')).toBeTruthy();
  });
  it('never renders the previous report during the frame before effects clear state', async () => {
    const next = deferred<ReviewWorkspace>(); const frames: boolean[] = [];
    invoke.mockImplementation(((_command: string, args: { reportId: string }) => args.reportId === 'A' ? Promise.resolve(workspace('A')) : next.promise) as typeof invokeCommand);
    function Probe({ id }: { id: string }) {
      useLayoutEffect(() => { if (id === 'B') frames.push(screen.queryByText('A finding') !== null); }, [id]);
      return <EvidenceReviewPanel reportId={id} />;
    }
    const view = render(<Probe id="A" />); await screen.findByText('A finding');
    view.rerender(<Probe id="B" />);
    expect(frames).toEqual([false]);
    await act(async () => next.resolve(workspace('B')));
  });
  it('saves accountable review and displays native concurrency errors without losing the draft', async () => {
    invoke.mockImplementation(((command: string) => command === 'save_finding_review' ? Promise.reject(new Error('Newer review; reload before saving.')) : Promise.resolve(workspace('A'))) as typeof invokeCommand);
    render(<EvidenceReviewPanel reportId="A" />); await screen.findByText('A finding');
    fireEvent.change(screen.getAllByLabelText('Reviewer')[0], { target: { value: 'Editor' } });
    fireEvent.change(screen.getByLabelText('Reasoning and limits'), { target: { value: 'The revised authorization remains unknown.' } });
    fireEvent.change(screen.getByLabelText('Counterevidence search performed and result'), { target: { value: 'Searched amendments; none retrieved.' } });
    fireEvent.click(screen.getByText('Save review'));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('Newer review'));
    expect((screen.getByLabelText('Reasoning and limits') as HTMLTextAreaElement).value).toContain('authorization remains unknown');
    expect(invoke).toHaveBeenCalledWith('save_finding_review', { input: expect.objectContaining({ reportId: 'A', expectedRevision: 0, reviewer: 'Editor', verdict: 'OPEN' }) });
  });
  it('exports original evidence, calculation status, attributed review and explicit unknown dates', () => {
    const value = workspace('A');
    value.reviews = [{ id: 'review1', revision: 1, createdAt: '2026-09-26', input: { reportId: 'A', findingIndex: 0, expectedRevision: 0, reviewer: 'Editor', assignedTo: '', verdict: 'CONTESTED', reasoning: 'Authorization unknown.', falsificationTest: 'Find revised budget.', counterSearch: 'Amendments reviewed.', counterEvidence: [] } }];
    const packet = reviewPacket(value);
    expect(packet).toContain('100.25'); expect(packet).toContain('CONTESTED'); expect(packet).toContain('Editor (local attribution)'); expect(packet).toContain('Not replayed.'); expect(packet).toContain('https://www.sandiego.gov/actuals.csv');
  });
});
