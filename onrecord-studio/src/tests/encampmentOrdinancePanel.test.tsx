// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import type { EncampmentOrdinanceReport } from '../types';

const report: EncampmentOrdinanceReport = {
  nightHours: null,
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
    const row = (await screen.findByText('63.0404(C)')).closest('tr');
    if (!row) throw new Error('section row missing');
    expect(within(row).getAllByText('not measured')).toHaveLength(3);
    expect(within(row).queryByText('0')).toBeTruthy();
    expect(screen.getByText(/fetch SANDAG transit stops/)).toBeTruthy();
  });
});
