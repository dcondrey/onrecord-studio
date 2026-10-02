import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AnalyticalSchema, FacilityArea, FacilityCoverageMapping, FacilityCoverageReport, FacilityCoverageWorkspace } from '../../types';

type ColumnRole = 'nameColumn' | 'idColumn' | 'addressColumn' | 'latitudeColumn' | 'longitudeColumn' | 'hoursColumn' | 'categoryColumn';
const columnRoles: [ColumnRole, string][] = [
  ['nameColumn', 'Facility name *'], ['idColumn', 'Facility id'], ['addressColumn', 'Address'], ['latitudeColumn', 'Latitude'],
  ['longitudeColumn', 'Longitude'], ['hoursColumn', 'Daily open hours'], ['categoryColumn', 'Category'],
];
interface AreaDraft { name: string; kind: 'bbox' | 'ids'; bbox: [string, string, string, string]; ids: string }
interface Draft { mapping: FacilityCoverageMapping; categories: string; areas: AreaDraft[] }

const NOT_RECORDED = 'not recorded';
function formatMinutesAsHours(minutes: number | null): string {
  if (minutes === null) return NOT_RECORDED;
  return `${Number((minutes / 60).toFixed(2))} h`;
}
const lines = (text: string) => text.split('\n').map((line) => line.trim()).filter(Boolean);
const emptyArea = (): AreaDraft => ({ name: '', kind: 'bbox', bbox: ['', '', '', ''], ids: '' });

function newDraft(snapshotId: string): Draft {
  return {
    mapping: { snapshotId, nameColumn: '', idColumn: null, addressColumn: null, latitudeColumn: null, longitudeColumn: null, hoursColumn: null, categoryColumn: null, restroomCategories: [], areas: [] },
    categories: '', areas: [emptyArea()],
  };
}
function draftFrom(mapping: FacilityCoverageMapping): Draft {
  return {
    mapping, categories: mapping.restroomCategories.join('\n'),
    areas: mapping.areas.map((area) => area.boundingBox
      ? { name: area.name, kind: 'bbox', ids: '', bbox: [area.boundingBox.minLatitude, area.boundingBox.minLongitude, area.boundingBox.maxLatitude, area.boundingBox.maxLongitude].map(String) as AreaDraft['bbox'] }
      : { name: area.name, kind: 'ids', bbox: ['', '', '', ''], ids: area.facilityIds.join('\n') }),
  };
}
function toArea(area: AreaDraft): FacilityArea {
  if (area.kind === 'ids') return { name: area.name.trim(), boundingBox: null, facilityIds: lines(area.ids) };
  const [minLatitude, minLongitude, maxLatitude, maxLongitude] = area.bbox.map((value) => (value.trim() === '' ? Number.NaN : Number(value)));
  if ([minLatitude, minLongitude, maxLatitude, maxLongitude].some((value) => !Number.isFinite(value))) {
    throw new Error(`Area ${area.name || '(unnamed)'} needs four numeric bounding-box values.`);
  }
  return { name: area.name.trim(), boundingBox: { minLatitude, minLongitude, maxLatitude, maxLongitude }, facilityIds: [] };
}

export default function FacilityCoveragePanel({ schema }: { schema: AnalyticalSchema }) {
  const [workspace, setWorkspace] = useState<FacilityCoverageWorkspace | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const gate = useLatestRequestGate();

  useEffect(() => {
    const token = gate.begin();
    void invokeCommand('get_facility_coverage', {})
      .then((next) => {
        if (!gate.isCurrent(token)) return;
        if (!Array.isArray(next?.reports)) throw new Error('The native facility coverage response is invalid.');
        setWorkspace(next);
      })
      .catch((reason) => { if (gate.isCurrent(token)) setError(String(reason)); });
    return gate.invalidate;
  }, [gate]);

  const save = async () => {
    if (!draft) return;
    let mapping: FacilityCoverageMapping;
    try {
      mapping = { ...draft.mapping, restroomCategories: lines(draft.categories), areas: draft.areas.map(toArea) };
    } catch (reason) { setError(String(reason)); return; }
    const token = gate.begin();
    setSaving(true); setError('');
    try {
      const next = await invokeCommand('save_facility_coverage_mapping', { mapping });
      if (!gate.isCurrent(token)) return;
      setWorkspace(next); setDraft(null);
    } catch (reason) {
      if (gate.isCurrent(token)) setError(String(reason));
    } finally {
      if (gate.isCurrent(token)) setSaving(false);
    }
  };

  const draftTable = schema.datasets.find((dataset) => dataset.snapshotId === draft?.mapping.snapshotId)?.tableName;
  const draftColumns = schema.columns.filter((column) => column.tableName === draftTable).map((column) => column.columnName);
  const setArea = (index: number, patch: Partial<AreaDraft>) => draft && setDraft({ ...draft, areas: draft.areas.map((area, i) => (i === index ? { ...area, ...patch } : area)) });

  return (
    <div className="space-y-4" aria-label="Facility coverage">
      <p className="text-label text-muted">Descriptive counts from an imported facility list, per area you define. Only rows with recorded, interpretable daily hours contribute to open-hours arithmetic; blank hours stay {NOT_RECORDED}. This view does not raise findings.</p>
      {error && <p role="alert" className="text-label leading-4 text-danger">{error}</p>}
      {workspace?.warnings.map((warning) => <p key={warning} className="text-label leading-4 text-warning">{warning}</p>)}
      <div className="flex flex-wrap items-end gap-2">
        <label className="text-label leading-4 text-fg-secondary">Facility list
          <select aria-label="Facility list snapshot" className="analysis-input ml-2" disabled={saving} value={draft?.mapping.snapshotId ?? ''} onChange={(event) => {
            const snapshotId = event.target.value;
            const saved = workspace?.reports.find((report) => report.mapping.snapshotId === snapshotId)?.mapping;
            setDraft(snapshotId ? (saved ? draftFrom(saved) : newDraft(snapshotId)) : null);
          }}>
            <option value="">Choose an imported table</option>
            {schema.datasets.map((dataset) => <option key={dataset.snapshotId} value={dataset.snapshotId}>{dataset.displayName}</option>)}
          </select>
        </label>
      </div>
      {draft && (
        <div className="space-y-3 rounded border border-line p-3">
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
            {columnRoles.map(([role, label]) => (
              <label key={role} className="text-label text-fg-secondary">{label}
                <select aria-label={`Facility ${label.replace(' *', '').toLowerCase()} column`} className="analysis-input block w-full" disabled={saving} value={draft.mapping[role] ?? ''} onChange={(event) => setDraft({ ...draft, mapping: { ...draft.mapping, [role]: role === 'nameColumn' ? event.target.value : event.target.value || null } })}>
                  <option value="">{role === 'nameColumn' ? 'Choose column' : NOT_RECORDED}</option>
                  {draftColumns.map((column) => <option key={column}>{column}</option>)}
                </select>
              </label>
            ))}
          </div>
          {draft.mapping.categoryColumn && (
            <label className="block text-label text-fg-secondary">Category values that mean a public restroom (one per line, exact match ignoring case)
              <textarea aria-label="Restroom category values" className="analysis-input block w-full" rows={2} disabled={saving} value={draft.categories} onChange={(event) => setDraft({ ...draft, categories: event.target.value })} />
            </label>
          )}
          <div className="space-y-2">
            <div className="analysis-label">Areas</div>
            {draft.areas.map((area, index) => (
              <div key={index} className="flex flex-wrap items-end gap-2 rounded border border-line p-2">
                <label className="text-label text-fg-secondary">Name<input aria-label={`Area ${index + 1} name`} className="analysis-input block" maxLength={200} disabled={saving} value={area.name} onChange={(event) => setArea(index, { name: event.target.value })} /></label>
                <label className="text-label text-fg-secondary">Boundary
                  <select aria-label={`Area ${index + 1} boundary type`} className="analysis-input block" disabled={saving} value={area.kind} onChange={(event) => setArea(index, { kind: event.target.value as AreaDraft['kind'] })}>
                    <option value="bbox">Latitude/longitude box</option>
                    <option value="ids">Listed facility ids</option>
                  </select>
                </label>
                {area.kind === 'bbox'
                  ? (['Min latitude', 'Min longitude', 'Max latitude', 'Max longitude'] as const).map((label, bound) => (
                    <label key={label} className="text-label text-fg-secondary">{label}<input aria-label={`Area ${index + 1} ${label.toLowerCase()}`} inputMode="decimal" className="analysis-input block w-28" disabled={saving} value={area.bbox[bound]} onChange={(event) => { const bbox = [...area.bbox] as AreaDraft['bbox']; bbox[bound] = event.target.value; setArea(index, { bbox }); }} /></label>
                  ))
                  : <label className="text-label text-fg-secondary">Facility ids (one per line)<textarea aria-label={`Area ${index + 1} facility ids`} className="analysis-input block" rows={3} disabled={saving} value={area.ids} onChange={(event) => setArea(index, { ids: event.target.value })} /></label>}
                <button type="button" className="text-label text-muted underline" disabled={saving || draft.areas.length === 1} onClick={() => setDraft({ ...draft, areas: draft.areas.filter((_, i) => i !== index) })}>Remove area</button>
              </div>
            ))}
            <button type="button" className="text-label text-accent underline" disabled={saving} onClick={() => setDraft({ ...draft, areas: [...draft.areas, emptyArea()] })}>Add area</button>
          </div>
          <div className="flex gap-3">
            <button type="button" className="analysis-button" disabled={saving || !draft.mapping.nameColumn} onClick={() => void save()}>{saving ? 'Validating columns and areas…' : 'Save facility mapping'}</button>
            <button type="button" className="text-label leading-4 text-fg-secondary" disabled={saving} onClick={() => setDraft(null)}>Cancel</button>
          </div>
        </div>
      )}
      {workspace?.reports.map((report) => <CoverageReport key={report.mapping.snapshotId} report={report} />)}
      {workspace && workspace.reports.length === 0 && <p className="text-label text-muted">No facility list is mapped. Import the agency's facility list above, then choose it here and define the areas to summarize.</p>}
    </div>
  );
}

function CoverageReport({ report }: { report: FacilityCoverageReport }) {
  return (
    <section aria-label={`Facility coverage for ${report.datasetName}`} className="space-y-2 rounded border border-line p-3">
      <h3 className="text-label leading-4 font-semibold text-fg">{report.datasetName}</h3>
      <p className="font-mono text-caption text-muted">{report.tableName} · SHA-256 {report.sourceSha256.slice(0, 16)}… · {report.totalRows} rows, {report.publicRestroomRows} counted as public restrooms{report.locationNotRecorded !== null && ` · ${report.locationNotRecorded} without coordinates`}</p>
      <div className="overflow-x-auto">
        <table className="w-full text-left text-label">
          <thead>
            <tr>{['Area', 'Public restrooms', 'With recorded hours', 'Hours unreadable', 'Hours not recorded', 'Recorded open coverage (of 24 h)', 'Time with none of the recorded open'].map((heading) => <th key={heading} className="p-1.5 text-fg-secondary">{heading}</th>)}</tr>
          </thead>
          <tbody>
            {report.areas.map((area) => (
              <tr key={area.areaName} className="border-t border-line text-fg">
                <td className="p-1.5">{area.areaName}<div className="text-caption text-muted">{area.boundary}</div>{area.unmatchedFacilityIds.length > 0 && <div className="text-caption text-warning">Not found: {area.unmatchedFacilityIds.join(', ')}</div>}</td>
                <td className="p-1.5">{area.publicRestrooms}</td>
                <td className="p-1.5">{area.withRecordedHours}</td>
                <td className="p-1.5">{area.hoursUninterpretable}</td>
                <td className="p-1.5">{area.hoursNotRecorded}</td>
                <td className="p-1.5">{formatMinutesAsHours(area.openMinutesCovered)}</td>
                <td className="p-1.5">{formatMinutesAsHours(area.minutesWithNoneOpen)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <details className="text-label text-fg-secondary">
        <summary className="cursor-pointer text-accent">Facilities per area</summary>
        {report.areas.map((area) => (
          <div key={area.areaName} className="mt-2">
            <div className="font-semibold">{area.areaName}</div>
            {area.facilities.map((facility) => (
              <p key={facility.tableRow} className="text-label text-muted">Row {facility.tableRow}: {facility.name ?? 'name not recorded'}{facility.address ? ` · ${facility.address}` : ''} · hours: {facility.hoursText ?? NOT_RECORDED}{facility.hoursStatus === 'UNINTERPRETABLE' && ' (not interpretable as daily hours)'}</p>
            ))}
            {area.facilitiesOmitted > 0 && <p className="text-label text-muted">{area.facilitiesOmitted} more not listed.</p>}
          </div>
        ))}
      </details>
      <ul className="list-disc pl-4 text-label text-muted">{report.limitations.map((limitation) => <li key={limitation}>{limitation}</li>)}</ul>
    </section>
  );
}
