import { useEffect, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { useStarterRunStore, type StarterEntryStatus } from '../../store/useStarterRunStore';
import type { StarterEntry, StarterSetsReport } from '../../types';
import { entriesToAdd, entryHost, entryIsSaved } from '../../utils/starterSets';
import MethodNote from '../ui/MethodNote';
import Time from '../ui/Time';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';

/**
 * One-click bundles of public San Diego sources. Choosing a set fetches nothing: it opens a
 * confirmation that names every site that would be contacted, and only the confirming button
 * starts the ordinary add-source path, one source at a time. The run itself lives in
 * `useStarterRunStore` so it outlasts this panel.
 */
export default function StarterSets() {
  const portals = useAppStore((state) => state.portals);
  const runningId = useStarterRunStore((state) => state.runningSetId);
  const statuses = useStarterRunStore((state) => state.statuses);
  const [report, setReport] = useState<StarterSetsReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmingId, setConfirmingId] = useState<string | null>(null);
  const registryGate = useLatestRequestGate();

  useEffect(() => {
    const request = registryGate.begin();
    invokeCommand('list_starter_sets', {})
      .then((value) => {
        if (!registryGate.isCurrent(request)) return;
        if (Array.isArray(value?.sets)) setReport(value);
        else setError('The registry returned an unreadable response.');
      })
      .catch((reason) => {
        if (registryGate.isCurrent(request)) setError(formatNativeError(reason));
      });
    return registryGate.invalidate;
  }, [registryGate]);

  if (error) return <InlineError summary="Starter sets couldn’t be read." detail={error} />;
  if (!report) return <Skeleton variant="lines" lines={3} label="Loading starter sets" />;

  return (
    <section aria-label="Starter sets" className="space-y-3">
      <div>
        <h3 className="text-heading text-fg">Starter sets</h3>
        <div className="mt-1 max-w-prose text-body text-muted">
          Bundles of public San Diego sources and records portals. Nothing is fetched until you confirm a set.{' '}
          <MethodNote>
            A source that does not load is listed as unavailable with the reason, not as an empty source. Registry version{' '}
            {report.registryVersion}. Verification dates record one check, not a promise the page is still there.
          </MethodNote>
        </div>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        {report.sets.map((set) => {
          const pending = entriesToAdd(set, portals);
          return (
            <article key={set.id} aria-label={set.title} className="flex flex-col rounded-lg border border-line p-4">
              <h4 className="text-body font-medium text-fg">{set.title}</h4>
              <p className="mt-1 text-body text-muted">{set.summary}</p>
              <ul className="mt-3 space-y-3 text-caption text-fg-secondary">
                {set.entries.map((entry) => (
                  <EntryRow key={entry.id} entry={entry} saved={entryIsSaved(entry, portals)} status={statuses[entry.id]} />
                ))}
              </ul>
              <div className="mt-auto pt-4">
                {confirmingId === set.id ? (
                  <div role="group" aria-label={`Confirm ${set.title}`} className="space-y-3 rounded-lg bg-raised/60 p-3 text-body text-fg-secondary">
                    <p>
                      This contacts {[...new Set(pending.map(entryHost))].join(', ')} and downloads their public pages
                      and linked files, one source at a time.
                    </p>
                    <div className="flex gap-2">
                      <Button variant="primary" onClick={() => { setConfirmingId(null); void useStarterRunStore.getState().run(set, pending); }}>
                        Fetch {pending.length} source{pending.length === 1 ? '' : 's'}
                      </Button>
                      <Button variant="ghost" onClick={() => setConfirmingId(null)}>Cancel</Button>
                    </div>
                  </div>
                ) : runningId === set.id ? (
                  <Button onClick={() => useStarterRunStore.getState().cancel()}>
                    Adding… stop after the current source
                  </Button>
                ) : pending.length === 0 ? (
                  <span className="text-caption text-muted">Every source in this set is already in your library.</span>
                ) : (
                  <Button disabled={runningId !== null} onClick={() => setConfirmingId(set.id)}>
                    {`Add ${pending.length} source${pending.length === 1 ? '' : 's'}…`}
                  </Button>
                )}
              </div>
            </article>
          );
        })}
      </div>
    </section>
  );
}

function EntryRow({ entry, saved, status }: { entry: StarterEntry; saved: boolean; status: StarterEntryStatus | undefined }) {
  return (
    <li>
      <div className="flex flex-wrap items-baseline gap-x-2">
        <a href={entry.url} target="_blank" rel="noreferrer" className="focus-ring rounded text-body font-medium text-accent underline-offset-2 hover:underline">{entry.title}<span className="sr-only"> (opens in browser)</span></a>
        <span className="text-muted">{entry.kind === 'RECORD_REQUEST_TARGET' ? 'Records request portal' : 'Source'}</span>
        {status === 'adding' ? <span className="text-muted">Adding…</span>
          : status?.state === 'started' && saved ? <span className="text-success">Started</span>
          : status?.state === 'unavailable' ? <span role="alert" className="text-warning">Unavailable: {status.reason}</span>
          : saved ? <span className="text-muted">In your library</span> : null}
      </div>
      <div className="text-muted">{entry.what} {entry.publisher}. Verified <Time value={entry.verifiedAsOf} className="whitespace-nowrap" />.</div>
      <div className="text-muted">{entry.termsNote ?? 'No reuse terms were found; check the publisher before redistributing.'}</div>
    </li>
  );
}
