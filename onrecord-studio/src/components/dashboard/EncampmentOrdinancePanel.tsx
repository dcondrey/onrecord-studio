import { useState } from 'react';
import { DistanceStrip, NightHoursStrip, activityKey, incidentKey } from '../charts/figures/OrdinanceStrips';
import { invokeCommand } from '../../ipc';
import type { EncampmentOrdinanceReport, ObservationSource } from '../../types';
import Icon from '../ui/icons';

const NOT_MEASURED = 'not measured';
const anchor = (key: string) => `ordinance-${key.replace(/[^a-zA-Z0-9_-]/g, '_')}`;
const shaName = (source: ObservationSource) => `${source.displayName} (SHA-256 ${source.sourceSha256.slice(0, 12)}…)`;
const band = (value: number | null) => (value === null ? NOT_MEASURED : value.toLocaleString());
const feet = (value: number | undefined) => (value === undefined ? NOT_MEASURED : `${Math.round(value).toLocaleString()} ft`);

export default function EncampmentOrdinancePanel() {
  const [report, setReport] = useState<EncampmentOrdinanceReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [highlight, setHighlight] = useState<string | null>(null);
  const [openRecords, setOpenRecords] = useState<{ night: boolean; locations: boolean }>({ night: false, locations: false });

  /** A dot opens its Records table and moves to its row. */
  const showRecord = (key: string) => {
    setOpenRecords((current) => ({ ...current, [key.startsWith('activity:') ? 'night' : 'locations']: true }));
    setHighlight(key);
    window.requestAnimationFrame(() => {
      const row = document.getElementById(anchor(key));
      row?.scrollIntoView?.({ block: 'nearest', behavior: document.documentElement.dataset.motion === 'full' ? 'smooth' : 'auto' });
      row?.focus({ preventScroll: true });
    });
  };
  const rowClass = (key: string) => (highlight === key ? 'bg-accent/10 outline outline-1 outline-focus' : undefined);

  const load = async () => {
    setLoading(true);
    setError(null);
    try {
      setReport(await invokeCommand('get_encampment_ordinance_report', {}));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  };

  const night = report?.nightHours ?? null;
  const locations = report?.locations ?? null;
  return (
    <section className="app-panel p-4">
      <div className="mb-3 flex items-center gap-2">
        <Icon name="clock" className="h-4 w-4 text-accent" />
        <h2 className="text-sm font-semibold text-fg">Encampment ordinance observations</h2>
      </div>
      <p className="text-label leading-relaxed text-muted">
        Counts from imported tables for SDMC 63.0404 and 63.0405(b). These are observations, not findings that any citation was unlawful: shelter availability, posted signs and park designations are not in the data.
      </p>
      <button type="button" className="analysis-button mt-3" disabled={loading} onClick={() => void load()}>
        {loading ? 'Measuring' : report ? 'Measure again' : 'Measure'}
      </button>
      {error && <p className="mt-2 text-label text-danger" role="alert">{error}</p>}

      {night && (
        <div className="mt-4 grid gap-2">
          <p className="text-caption text-muted">
            Logs cover {night.firstActivity ?? NOT_MEASURED} to {night.lastActivity ?? NOT_MEASURED}. Times are as SDPD recorded them. The night-hours limit applies to 63.0404(a); other subsections are shown for comparison. Activities exactly at 9:00 p.m. or 5:30 a.m. are drawn hollow because the ordinance does not say whether “between” includes them.
          </p>
          <NightHoursStrip night={night} sources={{ tables: ['analytical_rows (SDPD arrest and citation logs)'], note: night.sources.map(shaName).join('; ') }} onRecord={showRecord} />
          <details open={openRecords.night} onToggle={(event) => { const open = event.currentTarget.open; setOpenRecords((current) => (current.night === open ? current : { ...current, night: open })); }} className="rounded-lg border border-line p-3">
            <summary className="focus-ring w-fit cursor-pointer rounded text-control text-fg-secondary hover:text-fg">Records: counts by section and listed activities</summary>
            <table className="mt-2 w-full text-label">
              <thead><tr className="text-left text-muted"><th>Section</th><th>Type</th><th>Activities</th><th>9 p.m.–5:30 a.m.</th><th>At an endpoint</th><th>Outside</th><th>Time not recorded</th></tr></thead>
              <tbody>
                {night.sections.map((row) => (
                  <tr key={`${row.section}:${row.activityType}`} className="tabular-nums">
                    <td>{row.section}</td><td>{row.activityType || NOT_MEASURED}</td><td>{row.activities}</td><td>{row.insideWindow}</td><td>{row.atWindowBoundary}</td><td>{row.outsideWindow}</td><td>{row.timeNotRecorded}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {night.listedActivities.length > 0 && (
              <ul className="mt-2 space-y-1 text-caption text-fg-secondary">
                {night.listedActivities.map((activity) => (
                  <li key={activityKey(activity)} id={anchor(activityKey(activity))} tabIndex={-1} className={rowClass(activityKey(activity))}>
                    {activity.recordedAt} · {activity.activityType} {activity.section} · activity {activity.activityNumber} · {activity.location}{activity.atWindowBoundary ? ' · at an endpoint' : ''}
                  </li>
                ))}
              </ul>
            )}
          </details>
        </div>
      )}

      {locations && (
        <div className="mt-4 grid gap-2">
          <p className="text-caption text-muted">
            Distance uses the nearer of the closest school or station. 1,320 ft (a quarter mile) stands in for “two blocks”; the ordinance states no distance. Rows are NIBRS offenses; cases counts distinct case numbers.
          </p>
          <DistanceStrip
            locations={locations}
            sources={{
              tables: ['analytical_rows (NIBRS offenses, schools, GTFS stations)'],
              note: [...locations.incidentSources, ...(locations.schoolSource ? [locations.schoolSource] : []), ...(locations.stationSource ? [locations.stationSource] : [])].map(shaName).join('; '),
            }}
            onRecord={showRecord}
          />
          <details open={openRecords.locations} onToggle={(event) => { const open = event.currentTarget.open; setOpenRecords((current) => (current.locations === open ? current : { ...current, locations: open })); }} className="rounded-lg border border-line p-3">
            <summary className="focus-ring w-fit cursor-pointer rounded text-control text-fg-secondary hover:text-fg">Records: distance bands and offenses</summary>
            <table className="mt-2 w-full text-label">
              <thead><tr className="text-left text-muted"><th>Section</th><th>Offenses</th><th>Cases</th><th>Not geocoded</th><th>≤ 1,320 ft</th><th>1,320–2,640 ft</th><th>&gt; 2,640 ft</th></tr></thead>
              <tbody>
                {locations.sections.map((row) => (
                  <tr key={row.section} className="tabular-nums">
                    <td>{row.section}</td><td>{row.offenses}</td><td>{row.distinctCases}</td><td>{row.notGeocoded}</td><td>{band(row.withinNearBand)}</td><td>{band(row.withinFarBand)}</td><td>{band(row.beyondFarBand)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            <div className="mt-2 max-h-72 overflow-y-auto">
              <table className="w-full text-caption">
                <thead><tr className="text-left text-muted"><th>Occurred</th><th>Case</th><th>Section</th><th>Block</th><th>Nearest school</th><th>Nearest station</th></tr></thead>
                <tbody>
                  {locations.incidents.map((incident) => (
                    <tr key={incidentKey(incident)} id={anchor(incidentKey(incident))} tabIndex={-1} className={rowClass(incidentKey(incident))}>
                      <td>{incident.occurredOn ?? NOT_MEASURED}</td><td>{incident.caseNumber}</td><td>{incident.section}</td><td>{incident.blockAddress}</td>
                      <td>{incident.nearestSchool ? `${incident.nearestSchool.name}, ${feet(incident.nearestSchool.distanceFeet)}` : NOT_MEASURED}</td>
                      <td>{incident.nearestStation ? `${incident.nearestStation.name}, ${feet(incident.nearestStation.distanceFeet)}` : NOT_MEASURED}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </details>
        </div>
      )}

      {report && report.notMeasured.length > 0 && (
        <div className="mt-4">
          <h3 className="analysis-label">Not measured</h3>
          <ul className="mt-1 list-disc pl-4 text-caption text-muted">
            {report.notMeasured.map((note) => <li key={note}>{note}</li>)}
          </ul>
        </div>
      )}
    </section>
  );
}
