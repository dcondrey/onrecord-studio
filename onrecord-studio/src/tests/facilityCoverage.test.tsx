// @vitest-environment jsdom
import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import type { AnalyticalSchema, FacilityAreaCoverage, FacilityCoverageWorkspace } from '../types';
const bridge = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('../ipc', () => ({ invokeCommand: bridge.invoke }));
import FacilityCoveragePanel from '../components/dashboard/FacilityCoveragePanel';

afterEach(cleanup);
const area = (areaName: string, patch: Partial<FacilityAreaCoverage>): FacilityAreaCoverage => ({
  areaName, boundary: 'box', publicRestrooms: 0, withRecordedHours: 0, hoursUninterpretable: 0, hoursNotRecorded: 0,
  openMinutesCovered: null, minutesWithNoneOpen: null, unmatchedFacilityIds: [], excludedFacilityIds: [], facilities: [], facilitiesOmitted: 0, ...patch,
});
const workspace: FacilityCoverageWorkspace = {
  warnings: [],
  reports: [{
    mapping: { snapshotId: 'fac', nameColumn: 'name', idColumn: null, addressColumn: 'address', latitudeColumn: null, longitudeColumn: null, hoursColumn: 'hours', categoryColumn: null, restroomCategories: [], areas: [] },
    datasetName: 'Restrooms', tableName: 'analysis_fac', sourceUri: null, sourceSha256: 'd'.repeat(64), totalRows: 5, publicRestroomRows: 5, locationNotRecorded: null, limitations: [],
    areas: [
      area('Downtown', { publicRestrooms: 3, withRecordedHours: 2, hoursNotRecorded: 1, openMinutesCovered: 1080, minutesWithNoneOpen: 360 }),
      area('Unrecorded', { publicRestrooms: 2, hoursNotRecorded: 2 }),
    ],
  }],
};
const schema: AnalyticalSchema = { datasets: [], columns: [], templates: [] };

it('renders hours arithmetic from recorded rows and leaves an unmeasured area as not recorded', async () => {
  bridge.invoke.mockResolvedValue(workspace);
  render(<FacilityCoveragePanel schema={schema} />);
  const rows = await screen.findAllByRole('row');
  const downtown = within(rows[1]).getAllByRole('cell').map((cell) => cell.textContent);
  expect(downtown.slice(1)).toEqual(['3', '2', '0', '1', '18 h', '6 h']);
  const unrecorded = within(rows[2]).getAllByRole('cell').map((cell) => cell.textContent);
  expect(unrecorded.slice(1)).toEqual(['2', '0', '0', '2', 'not recorded', 'not recorded']);
});
