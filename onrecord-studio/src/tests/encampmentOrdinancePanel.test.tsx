// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import type { EncampmentOrdinanceReport } from '../types';

const report: EncampmentOrdinanceReport = {
  nightHours: {
    sources: [{ snapshotId: 'arrests', displayName: 'SDPD arrests 2025', sourceSha256: 'c'.repeat(64) }],
    firstActivity: '2025-01-01',
    lastActivity: '2025-06-30',
    sections: [{ section: '63.0404(a)', activityType: 'Citation', activities: 41, insideWindow: 12, atWindowBoundary: 3, outsideWindow: 22, timeNotRecorded: 4 }],
    listedActivities: [
      { activityNumber: 'A1', activityType: 'Citation', section: '63.0404(a)', recordedAt: '2025-03-14 22:15:00', atWindowBoundary: false, location: '1400 block Commercial St' },
      { activityNumber: 'A2', activityType: 'Citation', section: '63.0404(a)', recordedAt: '2025-03-15 21:00', atWindowBoundary: true, location: '1500 block Imperial Ave' },
    ],
  },
  locations: {
    incidentSources: [{ snapshotId: 'nibrs', displayName: 'SDPD NIBRS offenses 2025', sourceSha256: 'a'.repeat(64) }],
    schoolSource: { snapshotId: 'schools', displayName: 'SANDAG schools', sourceSha256: 'b'.repeat(64) },
    stationSource: null,
    sections: [{ section: '63.0404(C)', offenses: 3, distinctCases: 2, notGeocoded: 0, withinNearBand: null, withinFarBand: null, beyondFarBand: null }],
    incidents: [],
  },
  notMeasured: ['Distance to transit stations: fetch SANDAG transit stops to measure it.'],
};
vi.mock('@tauri-apps/api/core', () => ({ invoke: async () => report }));
import EncampmentOrdinancePanel from '../components/dashboard/EncampmentOrdinancePanel';
import { DistanceStrip, NightHoursStrip } from '../components/charts/figures/OrdinanceStrips';

afterEach(cleanup);

describe('encampment ordinance observations', () => {
  it('shows a band computed without both layers as not measured, never as zero', async () => {
    render(<EncampmentOrdinancePanel />);
    fireEvent.click(screen.getByRole('button', { name: 'Measure' }));
    const row = (await screen.findAllByText('63.0404(C)')).map((cell) => cell.closest('tr')).find(Boolean);
    if (!row) throw new Error('section row missing');
    expect(within(row).getAllByText('not measured')).toHaveLength(3);
    expect(within(row).queryByText('0')).toBeTruthy();
    expect(screen.getByText(/fetch SANDAG transit stops/)).toBeTruthy();
  });

  it('plots timed activities on the clock, counts untimed and ungeocoded records beside it, and uses no accent colour', async () => {
    render(<EncampmentOrdinancePanel />);
    fireEvent.click(screen.getByRole('button', { name: 'Measure' }));
    expect(await screen.findByText('12 of 41 activities under 63.0404(a) were recorded between 9 p.m. and 5:30 a.m.; 4 have no recorded time.')).toBeTruthy();
    expect(screen.getByRole('img', { name: /activity A2: recorded 2025-03-15 21:00, at an endpoint/ })).toBeTruthy();
    expect(document.querySelector('[data-absence="excluded"]')?.textContent).toBe('4 time not recorded');
    expect(screen.getByText(/Distance to schools and stations is not measured for every section; 0 offenses not geocoded/)).toBeTruthy();
    expect(screen.getByText('1,320 ft stand-in (reference)')).toBeTruthy();
    expect(document.body.innerHTML).not.toMatch(/--viz-(counted|lead|positive)/);
  });

  it('steps through dots in clock order, piles overflow as a count, and announces the focused dot', () => {
    const night = report.nightHours as NonNullable<typeof report.nightHours>;
    const activity = (n: string, recordedAt: string) => ({ activityNumber: n, activityType: 'Citation', section: '63.0404(a)', recordedAt, atWindowBoundary: false, location: 'x' });
    // Backend order is by date: the 02:00 activity comes first, but on the noon-to-noon clock it is last.
    const listed = [activity('early', '2025-03-01 02:00'), activity('late', '2025-03-02 23:30'), activity('mid', '2025-03-03 22:00'), ...Array.from({ length: 12 }, (_, i) => activity(`pile${i}`, '2025-03-04 00:30'))];
    render(<NightHoursStrip night={{ ...night, listedActivities: listed }} sources={{ tables: ['t'] }} onRecord={() => {}} />);
    const first = screen.getByRole('img', { name: /activity mid:/ });
    expect(first.getAttribute('tabindex')).toBe('0');
    fireEvent.focus(first);
    fireEvent.keyDown(first, { key: 'ArrowRight' });
    expect(document.activeElement).toBe(screen.getByRole('img', { name: /activity late:/ }));
    expect(document.querySelector('[aria-live="polite"]')?.textContent).toMatch(/activity late:/);
    // Seven rows fit at one time; the other five are a '+5' marker, not dots drawn over each other.
    expect(screen.getAllByRole('img', { name: /activity pile/ })).toHaveLength(7);
    expect(screen.getByText('+5')).toBeTruthy();
    expect(screen.getByText(/22 outside.*not placed/)).toBeTruthy();
  });

  it('counts offenses beyond the axis and offenses with no measured distance instead of placing them', () => {
    const locations = report.locations as NonNullable<typeof report.locations>;
    const incident = (caseNumber: string, feet: number | null) => ({ caseNumber, occurredOn: '2025-02-01', section: '63.0404(C)', blockAddress: 'b', latitude: 32.7, longitude: -117.1, nearestSchool: feet === null ? null : { name: 'S', distanceFeet: feet }, nearestStation: null });
    render(<DistanceStrip locations={{ ...locations, incidents: [incident('near', 800), incident('far', 6200), incident('none', null)] }} sources={{ tables: ['t'] }} onRecord={() => {}} />);
    expect(screen.getAllByRole('img', { name: /^Case / })).toHaveLength(1);
    expect(screen.getByText(/1 distance not measured/)).toBeTruthy();
    expect(document.body.textContent).toMatch(/1 >5,000 ft/);
  });
});
