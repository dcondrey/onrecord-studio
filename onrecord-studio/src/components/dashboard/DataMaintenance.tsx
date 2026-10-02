import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { DataMaintenancePolicy } from '../../types';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';

const gib = 1024 ** 3;
const size = (bytes: number) => bytes < gib ? `${(bytes / 1024 ** 2).toFixed(1)} MiB` : `${(bytes / gib).toFixed(2)} GiB`;

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
    }).catch((reason) => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, [controller]);
  const edit = (patch: Partial<DataMaintenancePolicy>) => { setSaved(false); setDraft((current) => current && { ...current, ...patch }); };
  const save = async () => {
    if (!draft || !controller) return;
    setSaving(true); setError(null); setSaved(false);
    const release = controller.hold();
    try { const result = await invokeCommand('set_data_maintenance_policy', { policy: draft }); controller.setMaintenance(result); setDraft(result.policy); setSaved(true); }
    catch (reason) { setError(String(reason)); }
    finally { release(); setSaving(false); }
  };
  return <details className="border-t border-line pt-3 text-label leading-4" aria-label="Data upkeep and storage">
    <summary className="cursor-pointer text-fg-secondary">Data upkeep &amp; storage{status ? ` · ${status.policy.enabled ? 'Automatic' : 'Paused'} · ${size(status.retainedFileBytes)} managed files` : ''}</summary>
    <div className="mt-3 space-y-3 text-muted">
      <p>While the app is open, import acquired tables, retry recoverable failures, and recheck active sources within their saved scope. Paused and removed sources stay paused.</p>
      {draft && <fieldset disabled={saving} className="space-y-3">
        <div className="flex flex-wrap gap-4">
          <label><input type="checkbox" checked={draft.enabled} onChange={(event) => edit({ enabled: event.target.checked })} /> Automatic upkeep</label>
          <label><input type="checkbox" checked={draft.autoImport} onChange={(event) => edit({ autoImport: event.target.checked })} /> Import acquired tables</label>
          <label><input type="checkbox" checked={draft.autoRefresh} onChange={(event) => edit({ autoRefresh: event.target.checked })} /> Recheck sources</label>
        </div>
        <div className="flex flex-wrap gap-4">
          <label className="space-x-2"><span>Recheck interval</span><select className="analysis-input" value={draft.refreshHours} onChange={(event) => edit({ refreshHours: Number(event.target.value) })}><option value={6}>6 hours</option><option value={24}>Daily</option><option value={168}>Weekly</option></select></label>
          <label className="space-x-2"><span>Managed-file budget (GiB)</span><input className="analysis-input w-24" type="number" min={0.1} max={1024} step={0.1} value={draft.retainedFileBudgetBytes / gib} onChange={(event) => edit({ retainedFileBudgetBytes: Math.round(Number(event.target.value) * gib) })} /></label>
        </div>
        <label className="block space-y-1"><span>Original download retention</span><select className="analysis-input block max-w-full" value={draft.retentionMode} onChange={(event) => edit({ retentionMode: event.target.value as DataMaintenancePolicy['retentionMode'] })}><option value="ORIGINALS">Keep original files and derived data</option><option value="REFERENCE_ONLY">Keep extracted text, tables, and source references</option></select></label>
        {draft.retentionMode === 'REFERENCE_ONLY' && <p className="rounded border border-line-strong p-3">Processed public downloads are released after validated extraction and complete table import. Searchable text, analytical tables, original URLs, and SHA-256 hashes remain in the compressed database; research dossiers use Zstandard compression and compact evidence excerpts. Local originals, workbooks with unprocessed sheets, incomplete extractions, and downloads without stable source links are retained. Original-file layout and forensic checks require the original bytes; a publisher’s later copy may differ.</p>}
        <p>The budget pauses new downloads when managed files reach the limit. Disk usage is cached for up to five seconds; in-flight downloads and the database can exceed this budget. Large sources still follow crawl and per-file import limits.</p>
        <button type="button" className="analysis-button" onClick={() => void save()}>{saving ? 'Saving…' : 'Save upkeep settings'}</button>
      </fieldset>}
      {saved && <p role="status" className="text-success">Upkeep settings saved.</p>}
      {error && <p role="alert" className="text-warning">{error}</p>}
      {status && <><p>{status.referencedDocuments.toLocaleString()} documents retain references · {size(status.reclaimedFileBytes)} original bytes released.</p>
        {status.events.length > 0 && <details><summary>Recent upkeep activity</summary><ul className="mt-2 space-y-2">{status.events.map((event, index) => <li key={`${event.recordedAt}:${index}`}><time>{new Date(event.recordedAt).toLocaleString()}</time> · {event.detail}</li>)}</ul></details>}</>}
    </div>
  </details>;
}
