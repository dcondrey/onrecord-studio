import { useState } from 'react';
import { open as chooseFolder } from '@tauri-apps/plugin-dialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { SharedFolderListing } from '../../types';
import { groupKeyId, type useContributors } from './useContributors';

const FOLDER_KEY = 'onrecord.sharedFolder';
const field = 'analysis-input w-full';

function readSavedFolder(): string {
  try {
    return window.localStorage.getItem(FOLDER_KEY) ?? '';
  } catch {
    return '';
  }
}

function saveFolder(folder: string) {
  try {
    window.localStorage.setItem(FOLDER_KEY, folder);
  } catch {
    // A remembered folder is a convenience; the app works without it.
  }
}

export function packetFileName(title: string, revision: number, keyId: string): string {
  const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 60) || 'case';
  return `${slug}-r${revision}-${keyId.slice(0, 8)}.json`;
}

/** Whether a verified contributor signed a previewed packet; a valid signature alone proves only that the file is unchanged. */
export function SignerTrustNotice({
  signerKeyId,
  contributors,
}: {
  signerKeyId: string;
  contributors: ReturnType<typeof useContributors>;
}) {
  const [label, setLabel] = useState('');
  const [error, setError] = useState('');
  if (contributors.state.status !== 'ready') {
    return <p role="status" className="text-label leading-4 text-slate-400">Checking your contributor list…</p>;
  }
  // The signer id arrives outside the signed bytes and may be in any case; stored keys are lowercase.
  const signer = signerKeyId.toLowerCase();
  const known = contributors.state.contributors.find((entry) => entry.keyId === signer);
  if (known) {
    return <p role="status" className="text-label leading-4 text-emerald-300">Signed by {known.label}, a contributor whose key you verified on {known.trustedAt.slice(0, 10)}.</p>;
  }
  const own = contributors.state.identity?.keyId === signer;
  if (own) return <p role="status" className="text-label leading-4 text-slate-300">Signed by this installation's own key.</p>;
  return (
    <div role="status" className="rounded border border-amber-500/30 p-2 text-label leading-4 text-amber-200">
      <p>This key is not on your contributor list: <code>{groupKeyId(signerKeyId).slice(0, 35)}…</code></p>
      <p className="mt-1">The signature shows the file is unchanged since that key signed it. It does not show who holds the key. Read the key aloud to the sender and compare before you apply their changes.</p>
      <div className="mt-2 flex flex-wrap items-center gap-2">
        <label className="sr-only" htmlFor="trust-label">Who this key belongs to</label>
        <input id="trust-label" className="analysis-input" placeholder="Who this key belongs to" value={label} maxLength={120} onChange={(event) => setLabel(event.target.value)} />
        <button type="button" className="analysis-button" disabled={!label.trim()} onClick={() => void contributors.trust(signerKeyId, label).then(() => setLabel('')).catch((reason) => setError(formatNativeError(reason)))}>I verified this key</button>
      </div>
      {error && <p role="alert" className="mt-1 text-rose-300">{error}</p>}
    </div>
  );
}

export default function CaseSharing({
  activeCase,
  author,
  contributors,
  onLoadPacket,
}: {
  activeCase: { id: string; title: string; revision: number } | null;
  author: string;
  contributors: ReturnType<typeof useContributors>;
  onLoadPacket: (payload: string, fileName: string) => void;
}) {
  const gate = useLatestRequestGate();
  const [folder, setFolder] = useState(readSavedFolder);
  const [listing, setListing] = useState<SharedFolderListing | null>(null);
  const packets = listing?.packets ?? null;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const [newKey, setNewKey] = useState('');
  const [newLabel, setNewLabel] = useState('');
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);

  const run = async <T,>(operation: () => Promise<T>, apply: (value: T) => void) => {
    const request = gate.begin();
    setBusy(true);
    setError('');
    setMessage('');
    try {
      const value = await operation();
      if (gate.isCurrent(request)) apply(value);
    } catch (reason) {
      if (gate.isCurrent(request)) setError(formatNativeError(reason));
    } finally {
      if (gate.isCurrent(request)) setBusy(false);
    }
  };

  const scan = (target: string) =>
    run(() => invokeCommand('list_shared_folder_packets', { folder: target }), setListing);

  const pickFolder = async () => {
    const chosen = await chooseFolder({ directory: true, multiple: false });
    if (typeof chosen !== 'string') return;
    gate.invalidate();
    setFolder(chosen);
    saveFolder(chosen);
    setListing(null);
    await scan(chosen);
  };

  const publish = () =>
    run(
      async () => {
        if (!activeCase) throw new Error('Open a case first.');
        const identity = await invokeCommand('get_local_contributor_identity', {});
        const payload = await invokeCommand('export_casework_packet', { caseId: activeCase.id, contributor: author });
        const fileName = packetFileName(activeCase.title, activeCase.revision, identity.keyId);
        await invokeCommand('publish_packet_to_folder', { folder, fileName, payload });
        return { fileName, listed: await invokeCommand('list_shared_folder_packets', { folder }) };
      },
      ({ fileName, listed }) => {
        setListing(listed);
        setMessage(`Signed packet written to the shared folder as ${fileName}. Anyone with access to the folder can read it.`);
      },
    );

  const load = (fileName: string) =>
    run(() => invokeCommand('read_shared_folder_packet', { folder, fileName }), (payload) => {
      onLoadPacket(payload, fileName);
      setMessage(`${fileName} loaded into "Review a shared case packet" above. Preview it before applying.`);
    });

  const ready = contributors.state.status === 'ready' ? contributors.state : null;

  return (
    <details className="mt-3">
      <summary className="cursor-pointer font-semibold text-slate-200">Collaborators and shared folder</summary>
      <p className="mt-2 text-label leading-4 text-slate-400">Collaborate without a server: put signed case packets in any folder you share (iCloud, Dropbox, a network drive, a USB stick) and review what others put there. Nothing is sent over the network by this app.</p>

      <section aria-label="Your contributor key" className="mt-3">
        <h4 className="text-sm font-semibold text-slate-100">Your key</h4>
        {ready?.identity ? (
          <>
            <p className="mt-1 break-all font-mono text-label leading-4 text-slate-300">{groupKeyId(ready.identity.keyId)}</p>
            <p className="mt-1 text-label leading-4 text-slate-400">Give this to collaborators in person or by phone so they can verify your packets.</p>
          </>
        ) : (
          <p className="mt-1 text-label leading-4 text-slate-400">{contributors.state.status === 'loading' ? 'Reading your key…' : 'Your key could not be read.'}</p>
        )}
      </section>

      <section aria-label="Trusted contributors" className="mt-3">
        <h4 className="text-sm font-semibold text-slate-100">Contributors you verified</h4>
        {contributors.state.status === 'failed' && <p role="alert" className="text-label leading-4 text-rose-300">The list could not be read: {contributors.state.error} <button type="button" className="underline" onClick={contributors.reload}>Retry</button></p>}
        {ready && ready.contributors.length === 0 && <p className="mt-1 text-label leading-4 text-slate-400">Nobody yet. Verify a key when you first receive a packet from someone.</p>}
        {ready && ready.contributors.length > 0 && (
          <ul className="mt-1 space-y-1 text-label leading-4 text-slate-300">
            {ready.contributors.map((entry) => (
              <li key={entry.keyId} className="flex flex-wrap items-center gap-2">
                <strong>{entry.label}</strong>
                <code>{entry.keyId.slice(0, 16)}…</code>
                {confirmRemove === entry.keyId ? (
                  <>
                    <button type="button" className="text-rose-300 underline" onClick={() => void contributors.untrust(entry.keyId).then(() => setConfirmRemove(null)).catch((reason) => setError(formatNativeError(reason)))}>Confirm removal</button>
                    <button type="button" className="underline" onClick={() => setConfirmRemove(null)}>Keep</button>
                  </>
                ) : (
                  <button type="button" className="underline" aria-label={`Remove ${entry.label} from verified contributors`} onClick={() => setConfirmRemove(entry.keyId)}>Remove</button>
                )}
              </li>
            ))}
          </ul>
        )}
        <div className="mt-2 grid gap-2 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_auto]">
          <label className="text-label leading-4 text-slate-300">Key they gave you<input className={`${field} mt-1 font-mono`} value={newKey} maxLength={80} onChange={(event) => setNewKey(event.target.value.replace(/\s+/g, ''))} /></label>
          <label className="text-label leading-4 text-slate-300">Who they are<input className={`${field} mt-1`} value={newLabel} maxLength={120} onChange={(event) => setNewLabel(event.target.value)} /></label>
          <button type="button" className="analysis-button self-end" disabled={!newKey || !newLabel.trim()} onClick={() => void contributors.trust(newKey, newLabel).then(() => { setNewKey(''); setNewLabel(''); setError(''); }).catch((reason) => setError(formatNativeError(reason)))}>Add</button>
        </div>
      </section>

      <section aria-label="Shared folder" className="mt-4">
        <h4 className="text-sm font-semibold text-slate-100">Shared folder</h4>
        <p className="mt-1 break-all text-label leading-4 text-slate-400">{folder || 'No folder chosen.'}</p>
        <div className="mt-2 flex flex-wrap gap-2">
          <button type="button" className="analysis-button" disabled={busy} onClick={() => void pickFolder()}>{folder ? 'Change folder' : 'Choose folder'}</button>
          <button type="button" className="analysis-button" disabled={busy || !folder} onClick={() => void scan(folder)}>Check for new packets</button>
          <button type="button" className="analysis-button" disabled={busy || !folder || !activeCase || !author.trim()} title={!activeCase ? 'Open a case first' : !author.trim() ? 'Enter your name as author first' : undefined} onClick={() => void publish()}>Publish this case's packet</button>
        </div>
        {error && <p role="alert" className="mt-2 text-label leading-4 text-rose-300">{error}</p>}
        {message && <p role="status" className="mt-2 text-label leading-4 text-emerald-300">{message}</p>}
        {listing?.truncated && (
          <p role="status" className="mt-2 text-label leading-4 text-amber-200">This folder holds more packets than are listed. These are the newest; move older ones out of the folder to see them.</p>
        )}
        {packets && packets.length === 0 && <p className="mt-2 text-label leading-4 text-slate-400">No packets in this folder. Files must be signed case packets ending in .json.</p>}
        {packets && packets.length > 0 && (
          <ul aria-label="Packets in the shared folder" className="mt-2 space-y-1 text-label leading-4 text-slate-300">
            {packets.map((packet) => (
              <li key={packet.fileName} className="flex flex-wrap items-center gap-2">
                <code>{packet.fileName}</code>
                <span className="text-slate-400">{(packet.bytes / 1024).toFixed(1)} KB{packet.modifiedAt ? ` · ${packet.modifiedAt.slice(0, 16).replace('T', ' ')}` : ''}</span>
                {packet.oversized ? <span className="text-amber-300">Too large to review (over 8 MB)</span> : <button type="button" className="underline" disabled={busy} onClick={() => void load(packet.fileName)}>Load for review</button>}
              </li>
            ))}
          </ul>
        )}
        <p className="mt-2 text-label leading-4 text-slate-400">A packet in the folder is not applied until you preview it and apply it. Publishing does not revoke access: anyone who can open the folder can read what you put there.</p>
      </section>
    </details>
  );
}
