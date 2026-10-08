// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { MonitoringEvidence, MonitoringStatus, ServiceObservation } from '../types/monitoring';
import { observationPoints, optionalMeasurement } from '../utils/monitoring';
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../ipc', async (importOriginal) => ({ ...(await importOriginal<typeof import('../ipc')>()), invokeCommand: bridge.invoke }));
import MonitoringPanel from '../components/dashboard/MonitoringPanel';

const evidence: MonitoringEvidence = { portalKey: 'city', documentId: 1, title: 'Published restroom inventory', sourceUrl: 'https://www.sandiego.gov/facilities.csv', sourceSha256: 'a'.repeat(64), extractionSha256: 'b'.repeat(64), quote: 'Public restroom, 2026-09-01, latitude 32.7 longitude -117.1. Weekly hours not recorded.', characterOffset: 0, snapshotId: null, rowNumber: null };
function fixture(caseId: string): MonitoringStatus {
  return { topic: 'bathrooms', caseId, changes: [], observations: [], comparisons: [], sourceCandidates: [evidence], limitations: ['Candidate records need scope and period review.'], coverage: [{ expectation: { id: `${caseId}:coverage`, caseId, topic: 'bathrooms', agency: 'City of San Diego', recordType: `${caseId} restroom operating logs`, period: 'FY2027', queryTerms: ['restroom', 'hours'], consequence: 'Test promised opening hours against actual availability.', acquisitionAction: 'Please provide existing facility opening and closure logs for July–September 2026.', state: 'EXPECTED', evidence: [] }, status: 'MATCHES_NEED_REVIEW', explanation: 'Verify the required period before recording receipt.', candidates: [evidence] }] };
}
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
beforeEach(() => { bridge.invoke.mockReset(); }); afterEach(cleanup);

describe('investigative evidence monitoring', () => {
  it('discards an old case response after the selected investigation changes', async () => {
    const old = deferred<MonitoringStatus>();
    bridge.invoke.mockImplementation((_command: string, args: { caseId: string }) => args.caseId === 'old' ? old.promise : Promise.resolve(fixture('current')));
    const view = render(<MonitoringPanel caseId="old" topic="bathrooms" />);
    view.rerender(<MonitoringPanel caseId="current" topic="bathrooms" />);
    await screen.findByText('current restroom operating logs');
    await act(async () => old.resolve(fixture('old')));
    expect(screen.queryByText('old restroom operating logs')).toBeNull();
    expect(screen.getByText('current restroom operating logs')).toBeTruthy();
  });

  it('does not reload the previous case when its delayed receipt save finishes', async () => {
    const saved = deferred<null>();
    bridge.invoke.mockImplementation((command: string, args: { caseId?: string }) => command === 'save_monitoring_coverage' ? saved.promise : Promise.resolve(fixture(args.caseId ?? 'none')));
    const view = render(<MonitoringPanel caseId="old" topic="bathrooms" />);
    await screen.findByText('old restroom operating logs');
    fireEvent.keyDown(screen.getByRole('img', { name: /restroom operating logs, FY2027/ }), { key: 'Enter' });
    fireEvent.click(screen.getByText('Record receipt after checking scope and period'));
    view.rerender(<MonitoringPanel caseId="current" topic="bathrooms" />);
    await screen.findByText('current restroom operating logs');
    await act(async () => saved.resolve(null));
    expect(bridge.invoke.mock.calls.filter(([command, args]) => command === 'get_monitoring_status' && args.caseId === 'old')).toHaveLength(1);
    expect(screen.queryByText('old restroom operating logs')).toBeNull();
  });

  it('records receipt with the exact selected citation and case identity', async () => {
    bridge.invoke.mockImplementation((command: string) => Promise.resolve(command === 'save_monitoring_coverage' ? null : fixture('riverbed')));
    render(<MonitoringPanel caseId="riverbed" topic="bathrooms" />);
    await screen.findByText('riverbed restroom operating logs');
    fireEvent.keyDown(screen.getByRole('img', { name: /restroom operating logs, FY2027/ }), { key: 'Enter' });
    fireEvent.click(screen.getByText('Record receipt after checking scope and period'));
    await act(async () => {});
    expect(bridge.invoke).toHaveBeenCalledWith('save_monitoring_coverage', { expectation: expect.objectContaining({ caseId: 'riverbed', state: 'RECEIVED', evidence: [evidence] }) });
  });

  it('outlines an expected record not found as a gap to request, and prints an unmapped status as sent', async () => {
    const missing = fixture('gap');
    missing.coverage[0] = { ...missing.coverage[0], status: 'NOT_LOCATED', candidates: [] };
    missing.coverage.push({ ...missing.coverage[0], expectation: { ...missing.coverage[0].expectation, id: 'other', period: 'FY2028' }, status: 'SOMETHING_NEW' });
    bridge.invoke.mockResolvedValue(missing);
    render(<MonitoringPanel caseId="gap" topic="bathrooms" />);
    const gap = await screen.findByRole('img', { name: /FY2027: expected, not found in the stored corpus/ });
    expect(gap.querySelector('[data-tone="lead"]')).toBeTruthy();
    expect(gap.textContent).toContain('not found');
    const raw = screen.getByRole('img', { name: /FY2028: SOMETHING_NEW/ });
    expect(raw.querySelector('[data-tone="neutral"]')).toBeTruthy();
    expect(screen.getByText('1 of 2 expected records not yet found, across 1 agency.')).toBeTruthy();
  });

  it('preserves zero measurements and never invents a missing coordinate', () => {
    const base: ServiceObservation = { id: 'a', caseId: null, topic: 'bathrooms', kind: 'FACILITY', place: 'Public facility', observedOn: '2026-09-01', latitude: 0, longitude: 0, hoursPerWeek: null, status: 'NOT_RECORDED', evidence, reviewed: false };
    expect(optionalMeasurement('')).toBeNull(); expect(optionalMeasurement('0')).toBe(0);
    expect(observationPoints([base, { ...base, id: 'b', latitude: null }])).toHaveLength(1);
    expect(observationPoints([base])[0]).toMatchObject({ x: 300, y: 150 });
  });
});
