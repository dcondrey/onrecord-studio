import { useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { EncampmentOrdinanceReport, ObservationSource } from '../../types';
import Icon from '../ui/icons';

const NOT_MEASURED = 'not measured';
const band = (value: number | null) => (value === null ? NOT_MEASURED : value.toLocaleString());
const feet = (value: number | undefined) => (value === undefined ? NOT_MEASURED : `${Math.round(value).toLocaleString()} ft`);

function Sources({ label, sources }: { label: string; sources: ObservationSource[] }) {
  return (
    <p className="mt-1 text-caption text-muted">
      {label}: {sources.map((source) => `${source.displayName} (SHA-256 ${source.sourceSha256.slice(0, 12)}…)`).join('; ')}
    </p>
  );
}

export default function EncampmentOrdinancePanel() {
  const [report, setReport] = useState<EncampmentOrdinanceReport | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
        <div className="mt-4">
          <h3 className="analysis-label">Time of day (63.0405(b): 9:00 p.m. to 5:30 a.m.)</h3>
          <Sources label="Arrest and citation logs" sources={night.sources} />
          <p className="mt-1 text-caption text-muted">
            Logs cover {night.firstActivity ?? NOT_MEASURED} to {night.lastActivity ?? NOT_MEASURED}. Times are as SDPD recorded them. The night-hours limit applies to 63.0404(a); other subsections are shown for comparison. Activities exactly at 9:00 p.m. or 5:30 a.m. are listed apart because the ordinance does not say whether “between” includes them.
          </p>
          <table className="mt-2 w-full text-label">
            <thead><tr className="text-left text-muted"><th>Section</th><th>Type</th><th>Activities</th><th>9 p.m.–5:30 a.m.</th><th>At an endpoint</th><th>Outside</th><th>Time not recorded</th></tr></thead>
            <tbody>
              {night.sections.map((row) => (
                <tr key={`${row.section}:${row.activityType}`}>
                  <td>{row.section}</td><td>{row.activityType || NOT_MEASURED}</td><td>{row.activities}</td><td>{row.insideWindow}</td><td>{row.atWindowBoundary}</td><td>{row.outsideWindow}</td><td>{row.timeNotRecorded}</td>
                </tr>
              ))}
            </tbody>
          </table>
          {night.listedActivities.length > 0 && (
            <ul className="mt-2 space-y-1 text-caption text-fg-secondary">
              {night.listedActivities.map((activity) => (
                <li key={`${activity.activityNumber}:${activity.section}:${activity.recordedAt}`}>
                  {activity.recordedAt} · {activity.activityType} {activity.section} · activity {activity.activityNumber} · {activity.location}{activity.atWindowBoundary ? ' · at an endpoint' : ''}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {locations && (
        <div className="mt-4">
          <h3 className="analysis-label">Distance to schools and transit stations (63.0404(c))</h3>
          <Sources label="NIBRS offenses" sources={locations.incidentSources} />
          {locations.schoolSource && <Sources label="Schools" sources={[locations.schoolSource]} />}
          {locations.stationSource && <Sources label="Transit stations (GTFS location_type 1)" sources={[locations.stationSource]} />}
          <p className="mt-1 text-caption text-muted">
            Bands use the nearer of the closest school or station. 1,320 ft (a quarter mile) stands in for “two blocks”; the ordinance states no distance. Rows are NIBRS offenses; cases counts distinct case numbers.
          </p>
          <table className="mt-2 w-full text-label">
            <thead><tr className="text-left text-muted"><th>Section</th><th>Offenses</th><th>Cases</th><th>Not geocoded</th><th>≤ 1,320 ft</th><th>1,320–2,640 ft</th><th>&gt; 2,640 ft</th></tr></thead>
            <tbody>
              {locations.sections.map((row) => (
                <tr key={row.section}>
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
                  <tr key={`${incident.caseNumber}:${incident.occurredOn}:${incident.blockAddress}`}>
                    <td>{incident.occurredOn ?? NOT_MEASURED}</td><td>{incident.caseNumber}</td><td>{incident.section}</td><td>{incident.blockAddress}</td>
                    <td>{incident.nearestSchool ? `${incident.nearestSchool.name}, ${feet(incident.nearestSchool.distanceFeet)}` : NOT_MEASURED}</td>
                    <td>{incident.nearestStation ? `${incident.nearestStation.name}, ${feet(incident.nearestStation.distanceFeet)}` : NOT_MEASURED}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
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
