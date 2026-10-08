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
});
