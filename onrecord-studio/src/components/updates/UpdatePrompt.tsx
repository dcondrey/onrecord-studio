import { useUpdateCheck } from './useUpdateCheck';

/** Non-blocking notice for a signed update. Renders nothing outside the desktop app, when the
 * app is current, or after "Later" for that version. */
export default function UpdatePrompt() {
  const { state, dismissed, install, later } = useUpdateCheck();
  if (state.phase === 'idle') return null;
  const version = state.update?.version ?? '';
  if (state.phase !== 'installing' && dismissed !== null && dismissed === version) return null;
  const installing = state.phase === 'installing';

  return (
    <div
      role="status"
      aria-live="polite"
      className="fixed right-4 top-16 z-40 w-72 rounded-xl border border-accent/30 bg-surface p-3 text-label leading-4 text-fg shadow-lg"
    >
      {state.phase === 'error' && (
        <p role="alert" className="mb-2 text-danger">
          Update failed: {state.message}
        </p>
      )}
      {state.update && (
        <>
          <p className="font-semibold">Update available: {state.update.version}</p>
          {state.update.notes && <p className="mt-1 whitespace-pre-line text-muted">{state.update.notes}</p>}
          <p className="mt-1 text-muted">Your license and account are not affected.</p>
        </>
      )}
      <div className="mt-2 flex gap-2">
        {state.update && (
          <button
            type="button"
            disabled={installing}
            onClick={() => void install()}
            className="focus-ring rounded-lg border border-accent/30 bg-accent/10 px-2.5 py-1 text-accent disabled:opacity-60"
          >
            {installing ? 'Installing...' : state.phase === 'error' ? 'Retry install' : 'Install and restart'}
          </button>
        )}
        <button
          type="button"
          disabled={installing}
          onClick={later}
          className="focus-ring rounded-lg border border-control px-2.5 py-1 text-fg-secondary disabled:opacity-60"
        >
          {state.phase === 'error' && !state.update ? 'Dismiss' : 'Later'}
        </button>
      </div>
    </div>
  );
}
