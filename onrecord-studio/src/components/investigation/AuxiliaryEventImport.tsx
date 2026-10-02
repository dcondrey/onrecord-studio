import DialogBackdrop from '../ui/DialogBackdrop';
import { useCallback, useEffect, useMemo, useRef, useState, type ChangeEvent, type FormEvent } from 'react';
import { invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { AuxiliaryEventInput, AuxiliaryImportResult, AuxiliaryStreamKind, JsonValue } from '../../types';

/**
 * The operator surface for `import_auxiliary_events`, the only production writer for
 * `auxiliary_events`. The cross-agency correlation queries and the agency profile aggregate read
 * that table, so without this the temporal panel could only ever say it was ready for data that
 * nothing could supply.
 */

const MAXIMUM_SOURCE_BYTES = 64 * 1024 * 1024;
const MAXIMUM_EVENTS = 50_000;
/** `validate_auxiliary_events` rejects on the encoded event payload, not on the source file. */
const MAXIMUM_PAYLOAD_BYTES = 64 * 1024 * 1024;
const PREVIEW_ROWS = 5;

const STREAM_KINDS: { value: AuxiliaryStreamKind; label: string }[] = [
  { value: 'MUNICIPAL_311', label: '311 service requests' },
  { value: 'CODE_ENFORCEMENT', label: 'Code enforcement' },
  { value: 'POLICE_DISPATCH', label: 'Police dispatch' },
  { value: 'SANITATION_OPERATIONS', label: 'Sanitation operations' },
  { value: 'CUSTOM', label: 'Other municipal stream' },
];

/** Each event field, the column names an export commonly uses for it, and whether it is required. */
const FIELDS = [
  { key: 'externalId', label: 'Record identifier', required: true, aliases: ['externalid', 'id', 'caseid', 'casenumber', 'servicerequestid', 'srnumber', 'uniquekey', 'incidentnumber'] },
  { key: 'occurredAt', label: 'Occurred at', required: true, aliases: ['occurredat', 'createddate', 'created', 'createdat', 'requestdate', 'opendate', 'datetimeinit', 'incidentdate', 'timestamp'] },
  { key: 'neighborhoodBlock', label: 'Neighborhood or block', required: true, aliases: ['neighborhoodblock', 'neighborhood', 'block', 'district', 'beat', 'council', 'councildistrict', 'address', 'incidentaddress'] },
  { key: 'eventType', label: 'Event type', required: true, aliases: ['eventtype', 'type', 'complainttype', 'category', 'servicename', 'requesttype', 'calltype'] },
  { key: 'description', label: 'Description', required: false, aliases: ['description', 'descriptor', 'details', 'notes', 'summary', 'comments'] },
  { key: 'latitude', label: 'Latitude', required: false, aliases: ['latitude', 'lat', 'y'] },
  { key: 'longitude', label: 'Longitude', required: false, aliases: ['longitude', 'lon', 'lng', 'long', 'x'] },
] as const;

type FieldKey = (typeof FIELDS)[number]['key'];
export type ColumnMapping = Record<FieldKey, string>;
type ParsedRow = Record<string, JsonValue>;

export interface ParsedFile {
  fileName: string;
  columns: string[];
  rows: ParsedRow[];
  /** Rows the file itself could not yield, so a short file is never mistaken for a clean one. */
  malformedRowCount: number;
}

interface RejectedRow {
  rowNumber: number;
  reason: string;
}

export interface MappedEvents {
  events: AuxiliaryEventInput[];
  rejected: RejectedRow[];
}

/** Splits one RFC 4180 record, honouring doubled quotes inside a quoted field. */
export function splitCsvLine(line: string): string[] {
  const values: string[] = [];
  let current = '';
  let quoted = false;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (quoted) {
      if (character !== '"') { current += character; continue; }
      if (line[index + 1] === '"') { current += '"'; index += 1; continue; }
      quoted = false;
      continue;
    }
    if (character === '"') { quoted = true; continue; }
    if (character === ',') { values.push(current); current = ''; continue; }
    current += character;
  }
  values.push(current);
  return values.map((value) => value.trim());
}

export function parseCsv(text: string, fileName: string): ParsedFile {
  const lines = text.split(/\r?\n/).filter((line) => line.trim().length > 0);
  if (lines.length < 2) {
    throw new Error('A CSV import needs a header row and at least one data row.');
  }
  const columns = splitCsvLine(lines[0]);
  if (new Set(columns).size !== columns.length || columns.some((column) => column.length === 0)) {
    throw new Error('The CSV header must name every column exactly once.');
  }
  const rows: ParsedRow[] = [];
  let malformedRowCount = 0;
  for (const line of lines.slice(1)) {
    const values = splitCsvLine(line);
    // A short or long record is not a row with blanks: its columns cannot be trusted to line up.
    if (values.length !== columns.length) { malformedRowCount += 1; continue; }
    const row: ParsedRow = {};
    columns.forEach((column, index) => { row[column] = values[index]; });
    rows.push(row);
  }
  return { fileName, columns, rows, malformedRowCount };
}

export function parseJson(text: string, fileName: string): ParsedFile {
  const parsed: unknown = JSON.parse(text);
  const candidates = Array.isArray(parsed)
    ? parsed
    : (parsed as { events?: unknown } | null)?.events;
  if (!Array.isArray(candidates)) {
    throw new Error('A JSON import must be an array of event objects, or an object with an "events" array.');
  }
  const rows: ParsedRow[] = [];
  let malformedRowCount = 0;
  for (const candidate of candidates) {
    if (typeof candidate !== 'object' || candidate === null || Array.isArray(candidate)) {
      malformedRowCount += 1;
      continue;
    }
    rows.push(candidate as ParsedRow);
  }
  const columns = [...new Set(rows.flatMap((row) => Object.keys(row)))];
  return { fileName, columns, rows, malformedRowCount };
}

function normalizeColumnName(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]/g, '');
}

export function autoMapColumns(columns: string[]): ColumnMapping {
  const normalized = columns.map((column) => [normalizeColumnName(column), column] as const);
  const mapping = {} as ColumnMapping;
  for (const field of FIELDS) {
    const exact = normalized.find(([name]) => name === normalizeColumnName(field.key));
    const aliased = exact ?? normalized.find(([name]) => (field.aliases as readonly string[]).includes(name));
    mapping[field.key] = aliased?.[1] ?? '';
  }
  return mapping;
}

function cellText(row: ParsedRow, column: string): string {
  if (!column) return '';
  const value = row[column];
  if (value === null || value === undefined) return '';
  if (typeof value === 'string') return value.trim();
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return '';
}

function coordinate(row: ParsedRow, column: string, bound: number): number | null | 'invalid' {
  const text = cellText(row, column);
  if (!text) return null;
  const value = Number(text);
  if (!Number.isFinite(value) || Math.abs(value) > bound) return 'invalid';
  return value;
}

const OFFSET_PATTERN = /(?:Z|[+-]\d{2}:?\d{2})$/i;

/** Fixed offsets, not zone names: a CSV column carries no rule for when a zone changes its offset. */
export const ZONE_OFFSETS = ['+00:00', '-04:00', '-05:00', '-06:00', '-07:00', '-08:00', '-09:00', '-10:00'] as const;

/**
 * Reduces the two shapes municipal exports actually use to ISO date-time components. Anything else
 * is left unrecognized rather than guessed at: `new Date` would accept it and invent a value.
 */
function isoComponents(value: string): string | null {
  const iso = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})(?::(\d{2})(\.\d{1,6})?)?$/.exec(value);
  if (iso) {
    const [, year, month, day, hour, minute, second = '00', fraction = ''] = iso;
    return `${year}-${month}-${day}T${hour}:${minute}:${second}${fraction}`;
  }
  const us = /^(\d{1,2})\/(\d{1,2})\/(\d{4})[ ,]+(\d{1,2}):(\d{2})(?::(\d{2}))?\s*(AM|PM)?$/i.exec(value);
  if (!us) return null;
  const [, month, day, year, rawHour, minute, second = '00', meridiem] = us;
  let hour = Number(rawHour);
  if (meridiem) {
    const isAfternoon = meridiem.toUpperCase() === 'PM';
    if (hour === 12) hour = isAfternoon ? 12 : 0;
    else if (isAfternoon) hour += 12;
  }
  if (hour > 23) return null;
  return `${year}-${month.padStart(2, '0')}-${day.padStart(2, '0')}T${String(hour).padStart(2, '0')}:${minute}:${second}`;
}

export type TimestampResult = { instant: string } | { reason: string };

/**
 * The backend requires an RFC 3339 instant and rejects anything else, counting it only as a
 * rejection. Converting here means an operator sees which rows will not land, and why, before the
 * import is committed rather than as an unexplained shortfall afterwards.
 *
 * A value carrying no offset is not an instant. `new Date` would read it as this machine's local
 * time and `toISOString` would then label the operator's wall clock as UTC, which is exactly the
 * defect 60300c9 removed from the storage layer. The offset therefore has to be stated, and rows
 * are refused until it is.
 */
export function toRfc3339(value: string, zoneOffset: string): TimestampResult {
  const text = value.trim();
  if (!text) return { reason: 'no timestamp' };
  if (OFFSET_PATTERN.test(text)) {
    const parsed = new Date(text);
    if (Number.isNaN(parsed.getTime())) return { reason: 'timestamp is not a recognizable date' };
    return { instant: parsed.toISOString() };
  }
  const components = isoComponents(text);
  if (!components) return { reason: 'timestamp is not a recognizable date' };
  if (!zoneOffset) return { reason: 'timestamp carries no time zone; state the export offset above' };
  const parsed = new Date(`${components}${zoneOffset}`);
  if (Number.isNaN(parsed.getTime())) return { reason: 'timestamp is not a recognizable date' };
  return { instant: parsed.toISOString() };
}

export function mapEvents(rows: ParsedRow[], mapping: ColumnMapping, zoneOffset: string): MappedEvents {
  const events: AuxiliaryEventInput[] = [];
  const rejected: RejectedRow[] = [];
  const seen = new Set<string>();
  rows.forEach((row, index) => {
    const rowNumber = index + 1;
    const externalId = cellText(row, mapping.externalId);
    const neighborhoodBlock = cellText(row, mapping.neighborhoodBlock);
    const eventType = cellText(row, mapping.eventType);
    const timestamp = toRfc3339(cellText(row, mapping.occurredAt), zoneOffset);
    if (!externalId) { rejected.push({ rowNumber, reason: 'no record identifier' }); return; }
    if (!('instant' in timestamp)) { rejected.push({ rowNumber, reason: timestamp.reason }); return; }
    const occurredAt = timestamp.instant;
    if (!neighborhoodBlock) { rejected.push({ rowNumber, reason: 'no neighborhood or block' }); return; }
    if (!eventType) { rejected.push({ rowNumber, reason: 'no event type' }); return; }
    if (seen.has(externalId)) { rejected.push({ rowNumber, reason: `duplicate identifier ${externalId}` }); return; }
    const latitude = coordinate(row, mapping.latitude, 90);
    const longitude = coordinate(row, mapping.longitude, 180);
    if (latitude === 'invalid' || longitude === 'invalid') {
      rejected.push({ rowNumber, reason: 'coordinates are outside valid bounds' });
      return;
    }
    seen.add(externalId);
    events.push({
      externalId,
      occurredAt,
      neighborhoodBlock,
      eventType,
      description: cellText(row, mapping.description),
      latitude,
      longitude,
      // The unmapped columns are the rest of the record. Keeping the row verbatim is what lets a
      // later question be answered from the import instead of from the operator's memory of it.
      sourcePayload: row,
    });
  });
  return { events, rejected };
}

interface AuxiliaryEventImportProps {
  isOpen: boolean;
  onClose: () => void;
}

export default function AuxiliaryEventImport({ isOpen, onClose }: AuxiliaryEventImportProps) {
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const requestAnalyticsRefresh = useAppStore((state) => state.requestAnalyticsRefresh);
  const fileInput = useRef<HTMLInputElement>(null);
  const [parsed, setParsed] = useState<ParsedFile | null>(null);
  const [mapping, setMapping] = useState<ColumnMapping>(() => autoMapColumns([]));
  const [sourceKind, setSourceKind] = useState<AuxiliaryStreamKind>('MUNICIPAL_311');
  const [displayName, setDisplayName] = useState('');
  const [sourceUri, setSourceUri] = useState('');
  const [zoneOffset, setZoneOffset] = useState('');
  const [scopeToActivePortal, setScopeToActivePortal] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<AuxiliaryImportResult | null>(null);
  const [isImporting, setIsImporting] = useState(false);
  const importGate = useLatestRequestGate();

  const reset = useCallback(() => {
    setParsed(null);
    setMapping(autoMapColumns([]));
    setDisplayName('');
    setSourceUri('');
    setZoneOffset('');
    setError(null);
    setResult(null);
    setIsImporting(false);
    importGate.invalidate();
    if (fileInput.current) fileInput.current.value = '';
  }, [importGate]);

  useEffect(() => {
    if (!isOpen) reset();
  }, [isOpen, reset]);

  const mapped = useMemo(
    () => (parsed ? mapEvents(parsed.rows, mapping, zoneOffset) : { events: [], rejected: [] }),
    [mapping, parsed, zoneOffset],
  );

  // The backend measures the encoded events, not the file: every row carries its whole original
  // record plus the mapped fields, and CSV column names repeat per row. Measuring the same thing
  // here means the limit is reported before the import rather than as a late rejection.
  const payloadBytes = useMemo(
    () => (mapped.events.length === 0 ? 0 : new TextEncoder().encode(JSON.stringify(mapped.events)).byteLength),
    [mapped.events],
  );

  const missingRequired = FIELDS.filter((field) => field.required && !mapping[field.key]);
  const overCapacity = mapped.events.length > MAXIMUM_EVENTS || payloadBytes > MAXIMUM_PAYLOAD_BYTES;
  const canImport =
    !isImporting && mapped.events.length > 0 && !overCapacity && missingRequired.length === 0 && displayName.trim().length > 0;

  const loadFile = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    if (!file) return;
    setError(null);
    setResult(null);
    if (file.size > MAXIMUM_SOURCE_BYTES) {
      setError(`Auxiliary exports must be no larger than ${MAXIMUM_SOURCE_BYTES / (1024 * 1024)} MiB.`);
      event.target.value = '';
      return;
    }
    try {
      const text = await file.text();
      const next = file.name.toLowerCase().endsWith('.json')
        ? parseJson(text, file.name)
        : parseCsv(text, file.name);
      if (next.rows.length === 0) throw new Error('The file contained no readable rows.');
      setParsed(next);
      setMapping(autoMapColumns(next.columns));
      if (!displayName.trim()) setDisplayName(file.name.replace(/\.[^.]+$/, '').split(/[_-]/).join(' '));
    } catch (reason) {
      setParsed(null);
      setError(`Could not read the selected export: ${reason instanceof Error ? reason.message : String(reason)}`);
      event.target.value = '';
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!canImport) return;
    const request = importGate.begin();
    setIsImporting(true);
    setError(null);
    setResult(null);
    try {
      const imported = await invokeCommand('import_auxiliary_events', {
        portalKey: scopeToActivePortal ? activePortalKey : null,
        sourceKind,
        displayName: displayName.trim(),
        sourceUri: sourceUri.trim() || null,
        events: mapped.events,
      });
      if (!importGate.isCurrent(request)) return;
      setResult(imported);
      // The correlation and profile panels read `auxiliary_events`; leaving them on the pre-import
      // aggregate would show a coverage answer the database no longer supports.
      requestAnalyticsRefresh();
    } catch (reason) {
      if (importGate.isCurrent(request)) setError(String(reason));
    } finally {
      if (importGate.isCurrent(request)) setIsImporting(false);
    }
  };

  if (!isOpen) return null;

  const previewRows = parsed ? parsed.rows.slice(0, PREVIEW_ROWS) : [];

  return (
    <DialogBackdrop labelledBy="auxiliary-import-title" onClose={onClose} className="app-backdrop fixed inset-0 z-[70] grid place-items-center p-6 backdrop-blur-sm">
      <form onSubmit={(event) => void submit(event)} className="flex h-[min(820px,92vh)] w-[min(1080px,96vw)] flex-col overflow-hidden rounded-2xl border border-slate-700 bg-slate-950 shadow-2xl shadow-black/60">
        <div className="flex items-start justify-between gap-4 border-b border-slate-800 px-5 py-4">
          <div>
            <div className="text-caption font-bold uppercase tracking-[0.2em] text-cyan-300/70">Auxiliary municipal stream</div>
            <h2 id="auxiliary-import-title" className="mt-1 text-base font-semibold text-slate-100">Import 311 and enforcement events</h2>
            <p className="mt-1 max-w-2xl text-label leading-relaxed text-slate-400">
              A CSV or JSON export becomes the correlation baseline the temporal panel reads. Each row is stored verbatim
              alongside the mapped fields, and the whole import is content-addressed so re-importing the same export updates
              rather than duplicates it.
            </p>
          </div>
          <button type="button" onClick={onClose} className="focus-ring rounded-lg px-3 py-2 text-caption font-semibold text-slate-400 hover:text-slate-300">Close</button>
        </div>

        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto px-5 py-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="block text-caption text-slate-400">
              Source name
              <input value={displayName} onChange={(event) => setDisplayName(event.target.value)} required maxLength={256} placeholder="City 311 service requests, 2026" className="mt-1 w-full rounded-lg border border-slate-800 bg-slate-950 px-2.5 py-2 text-label text-slate-200" />
            </label>
            <label className="block text-caption text-slate-400">
              Stream kind
              <select value={sourceKind} onChange={(event) => setSourceKind(event.target.value as AuxiliaryStreamKind)} className="mt-1 w-full rounded-lg border border-slate-800 bg-slate-950 px-2.5 py-2 text-label text-slate-200">
                {STREAM_KINDS.map((kind) => <option key={kind.value} value={kind.value}>{kind.label}</option>)}
              </select>
            </label>
            <label className="block text-caption text-slate-400">
              Source URI <span className="text-slate-400">(optional, recorded as provenance)</span>
              <input value={sourceUri} onChange={(event) => setSourceUri(event.target.value)} maxLength={2048} placeholder="https://data.example.gov/311.csv" className="mt-1 w-full rounded-lg border border-slate-800 bg-slate-950 px-2.5 py-2 font-mono text-caption text-slate-200" />
            </label>
            <label className="block text-caption text-slate-400">
              Offset for timestamps that carry none
              <select value={zoneOffset} onChange={(event) => setZoneOffset(event.target.value)} className="mt-1 w-full rounded-lg border border-slate-800 bg-slate-950 px-2.5 py-2 font-mono text-label text-slate-200">
                <option value="">— not stated —</option>
                {ZONE_OFFSETS.map((offset) => <option key={offset} value={offset}>UTC{offset}</option>)}
              </select>
            </label>
            <label className="flex items-center gap-2 self-end pb-2 text-caption text-slate-400">
              <input type="checkbox" checked={scopeToActivePortal && Boolean(activePortalKey)} disabled={!activePortalKey} onChange={(event) => setScopeToActivePortal(event.target.checked)} className="h-3 w-3" />
              {activePortalKey ? `Scope to the active source (${activePortalKey})` : 'No active source selected; this import will be global'}
            </label>
          </div>

          <div className="rounded-xl border border-slate-800 bg-slate-900/30 p-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <div className="text-label font-semibold text-slate-300">Export file</div>
                <div className="mt-0.5 text-caption text-slate-400">{parsed ? `${parsed.fileName} · ${parsed.rows.length.toLocaleString()} rows · ${parsed.columns.length} columns` : 'CSV with a header row, or a JSON array of event objects.'}</div>
              </div>
              <button type="button" onClick={() => fileInput.current?.click()} className="focus-ring rounded-lg border border-slate-700 bg-slate-950 px-3 py-2 text-caption font-semibold text-slate-300">{parsed ? 'Choose a different file' : 'Choose a file'}</button>
              <input ref={fileInput} type="file" accept=".csv,.json,text/csv,application/json" onChange={(event) => void loadFile(event)} className="sr-only" />
            </div>
            {parsed && parsed.malformedRowCount > 0 && (
              <p className="mt-2 text-caption text-amber-300">{parsed.malformedRowCount.toLocaleString()} record{parsed.malformedRowCount === 1 ? '' : 's'} could not be read as a row and are not counted below.</p>
            )}
          </div>

          {parsed && (
            <div className="rounded-xl border border-slate-800 bg-slate-900/30 p-4">
              <div className="text-label font-semibold text-slate-300">Column mapping</div>
              <div className="mt-0.5 text-caption text-slate-400">Detected from the header where the names matched. Correct anything that is wrong before importing.</div>
              <div className="mt-3 grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                {FIELDS.map((field) => (
                  <label key={field.key} className="block text-caption text-slate-400">
                    {field.label}{field.required && <span className="text-rose-300"> *</span>}
                    <select
                      value={mapping[field.key]}
                      onChange={(event) => setMapping((current) => ({ ...current, [field.key]: event.target.value }))}
                      className="mt-1 w-full rounded-lg border border-slate-800 bg-slate-950 px-2 py-1.5 font-mono text-caption text-slate-200"
                    >
                      <option value="">{field.required ? '— not mapped —' : '— not recorded —'}</option>
                      {parsed.columns.map((column) => <option key={column} value={column}>{column}</option>)}
                    </select>
                  </label>
                ))}
              </div>
              {previewRows.length > 0 && (
                <div className="mt-3 overflow-x-auto rounded-lg border border-slate-800">
                  <table className="w-full min-w-[640px] border-collapse text-left font-mono text-caption">
                    <thead className="bg-slate-950/80 text-slate-400">
                      <tr>{FIELDS.map((field) => <th key={field.key} className="px-2 py-1.5 font-semibold">{field.label}</th>)}</tr>
                    </thead>
                    <tbody className="text-slate-300">
                      {previewRows.map((row, index) => (
                        <tr key={`${cellText(row, mapping.externalId)}-${index}`} className="border-t border-slate-800/70">
                          {FIELDS.map((field) => (
                            <td key={field.key} className="max-w-40 truncate px-2 py-1.5">
                              {mapping[field.key] ? cellText(row, mapping[field.key]) || '—' : <span className="italic text-slate-400">not mapped</span>}
                            </td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </div>
          )}

          {parsed && (
            <div className="rounded-xl border border-slate-800 bg-slate-900/30 p-4">
              <div className="text-label font-semibold text-slate-300">What this import will write</div>
              {missingRequired.length > 0 ? (
                <p className="mt-2 text-caption text-amber-300">Map {missingRequired.map((field) => field.label.toLowerCase()).join(', ')} before this export can be imported.</p>
              ) : (
                <>
                  <div className="mt-2 flex flex-wrap gap-4 font-mono text-caption">
                    <span className="text-cyan-300">{mapped.events.length.toLocaleString()} events accepted</span>
                    <span className={mapped.rejected.length > 0 ? 'text-amber-300' : 'text-slate-400'}>{mapped.rejected.length.toLocaleString()} rows refused</span>
                  </div>
                  {overCapacity && (
                    <p className="mt-2 text-caption text-rose-300">
                      One import may carry at most {MAXIMUM_EVENTS.toLocaleString()} events and {MAXIMUM_PAYLOAD_BYTES / (1024 * 1024)} MiB of encoded records; this one is {mapped.events.length.toLocaleString()} events and {(payloadBytes / (1024 * 1024)).toFixed(1)} MiB. Split the export by date range and import each part as its own source.
                    </p>
                  )}
                  {mapped.rejected.length > 0 && (
                    <ul className="mt-2 space-y-0.5 font-mono text-caption text-slate-400">
                      {mapped.rejected.slice(0, PREVIEW_ROWS).map((rejection) => (
                        <li key={rejection.rowNumber}>Row {rejection.rowNumber.toLocaleString()}: {rejection.reason}</li>
                      ))}
                      {mapped.rejected.length > PREVIEW_ROWS && <li>…and {(mapped.rejected.length - PREVIEW_ROWS).toLocaleString()} more.</li>}
                    </ul>
                  )}
                </>
              )}
            </div>
          )}

          {error && <p role="alert" className="rounded-lg border border-rose-500/30 bg-rose-500/10 px-3 py-2 text-caption text-rose-200">{error}</p>}
          {result && (
            <div role="status" className="rounded-lg border border-cyan-400/25 bg-cyan-500/10 px-3 py-2 text-caption text-cyan-100">
              <div className="font-semibold">Imported as source {result.sourceId}</div>
              <div className="mt-1 font-mono text-caption text-cyan-200/80">
                {result.importedCount.toLocaleString()} new · {result.updatedCount.toLocaleString()} updated · {result.rejectedCount.toLocaleString()} refused by validation · SHA-256 {result.sourceSha256}
              </div>
            </div>
          )}
        </div>

        <div className="flex justify-end gap-2 border-t border-slate-800 px-5 py-3">
          <button type="button" onClick={onClose} className="focus-ring rounded-lg px-3 py-2 text-caption font-semibold text-slate-400 hover:text-slate-300">Cancel</button>
          <button type="submit" disabled={!canImport} className="focus-ring rounded-lg border border-cyan-400/25 bg-cyan-500/10 px-4 py-2 text-caption font-semibold text-cyan-200 disabled:opacity-40">
            {isImporting ? 'Importing…' : `Import ${mapped.events.length.toLocaleString()} events`}
          </button>
        </div>
      </form>
    </DialogBackdrop>
  );
}
