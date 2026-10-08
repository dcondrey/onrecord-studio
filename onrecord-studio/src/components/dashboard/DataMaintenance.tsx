import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import type { DataMaintenancePolicy } from '../../types';
import Button from '../ui/Button';
import { Field, Input, Select } from '../ui/Field';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';

const gib = 1024 ** 3;
/** Binary units with the short labels Activity uses, so one size reads the same everywhere. */
const size = (bytes: number) => bytes < gib ? `${(bytes / 1024 ** 2).toFixed(1)} MB` : `${(bytes / gib).toFixed(2)} GB`;
const EVENT_TIME = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
const RECENT_EVENTS = 5;

function eventTime(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : EVENT_TIME.format(parsed);
}

export default function DataMaintenance() {
  const { controller, state } = useAutomaticDiscovery();
  const status = state?.maintenance;
  const [draft, setDraft] = useState<DataMaintenancePolicy | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!controller) return;
    let active = true;
    void invokeCommand('get_data_maintenance_status', {}).then((result) => {
      if (active && result?.policy) { controller.setMaintenance(result); setDraft(result.policy); }
    }).catch((reason) => { if (active) setError(formatNativeError(reason)); });
    return () => { active = false; };
  }, [controller]);
  const edit = (patch: Partial<DataMaintenancePolicy>) => { setSaved(false); setDraft((current) => current && { ...current, ...patch }); };
  const save = async () => {
    if (!draft || !controller) return;
    setSaving(true); setError(null); setSaved(false);
    const release = controller.hold();
    try { const result = await invokeCommand('set_data_maintenance_policy', { policy: draft }); controller.setMaintenance(result); setDraft(result.policy); setSaved(true); }
    catch (reason) { setError(formatNativeError(reason)); }
    finally { release(); setSaving(false); }
  };
  return (
    <details className="group border-t border-line pt-4 text-body" aria-label="Data upkeep and storage">
      <summary className="focus-ring w-fit cursor-pointer rounded text-control text-fg-secondary hover:text-fg">
        Storage{status ? ` · ${status.policy.enabled ? 'Automatic upkeep' : 'Upkeep paused'} · ${size(status.retainedFileBytes)} managed` : ''}
      </summary>
      <div className="mt-4 max-w-prose space-y-4 text-muted">
        <p>While the app is open, it imports acquired tables, retries recoverable failures and rechecks active sources within their saved scope.</p>
        {draft && (
          <fieldset disabled={saving} className="space-y-4">
            <div className="flex flex-wrap gap-x-5 gap-y-2 text-fg-secondary">
              <label className="flex items-center gap-2"><input type="checkbox" className="accent-[rgb(var(--accent))]" checked={draft.enabled} onChange={(event) => edit({ enabled: event.target.checked })} />Automatic upkeep</label>
              <label className="flex items-center gap-2"><input type="checkbox" className="accent-[rgb(var(--accent))]" checked={draft.autoImport} onChange={(event) => edit({ autoImport: event.target.checked })} />Import acquired tables</label>
              <label className="flex items-center gap-2"><input type="checkbox" className="accent-[rgb(var(--accent))]" checked={draft.autoRefresh} onChange={(event) => edit({ autoRefresh: event.target.checked })} />Recheck sources</label>
            </div>
            <div className="flex flex-wrap gap-3">
              <Select label="Recheck interval" value={draft.refreshHours} onChange={(event) => edit({ refreshHours: Number(event.target.value) })} className="w-40">
                <option value={6}>Every 6 hours</option><option value={24}>Daily</option><option value={168}>Weekly</option>
              </Select>
              <Field label="Managed-file budget (GB)" help="New downloads pause at this limit.">
                <Input className="w-28 tabular-nums" type="number" min={0.1} max={1024} step={0.1} value={draft.retainedFileBudgetBytes / gib} onChange={(event) => edit({ retainedFileBudgetBytes: Math.round(Number(event.target.value) * gib) })} />
              </Field>
            </div>
            <Select label="Original download retention" value={draft.retentionMode} onChange={(event) => edit({ retentionMode: event.target.value as DataMaintenancePolicy['retentionMode'] })}>
              <option value="ORIGINALS">Keep original files and derived data</option>
              <option value="REFERENCE_ONLY">Keep extracted text, tables, and source references</option>
            </Select>
            {draft.retentionMode === 'REFERENCE_ONLY' && (
              <p className="rounded-lg border border-line p-3">Searchable text, tables, source URLs and SHA-256 hashes stay; processed downloads are released after validated extraction. Original-file layout and forensic checks require the original bytes; a publisher’s later copy may differ.</p>
            )}
            <Button size="sm" onClick={() => void save()}>{saving ? 'Saving…' : 'Save upkeep settings'}</Button>
          </fieldset>
        )}
        {saved && <p role="status" className="text-success">Upkeep settings saved.</p>}
        {error && <p role="alert" className="text-danger">{error}</p>}
        {status && (
          <>
            <p className="tabular-nums">{status.referencedDocuments.toLocaleString()} documents retain references · {size(status.reclaimedFileBytes)} of originals released.</p>
            {status.events.length > 0 && (
              <div>
                <h5 className="text-control text-fg-secondary">Recent upkeep activity</h5>
                <ul className="mt-2 space-y-1.5">
                  {status.events.slice(0, RECENT_EVENTS).map((event, index) => (
                    <li key={`${event.recordedAt}:${index}`}><time dateTime={event.recordedAt} className="tabular-nums">{eventTime(event.recordedAt)}</time> · {event.detail}</li>
                  ))}
                </ul>
              </div>
            )}
          </>
        )}
      </div>
    </details>
  );
}
