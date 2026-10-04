import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { PRODUCT_NAME } from '../../config/brand';
import type { StartupNotice, StartupStatus } from '../../types';

export const STARTUP_STATUS_EVENT = 'startup-status';
const POLL_INTERVAL_MS = 1_000;
/**
 * The rewrite call returns at once on a file already in current storage, so the phase alone does
 * not mean a rewrite is happening. Saying "one time" on every launch would be false; past this
 * threshold the call is doing real work.
 */
export const OPTIMIZING_LABEL_AFTER_MS = 1_000;

/** Notices already written to the Activity log in this page session, across remounts. */
const loggedNoticeIds = new Set<string>();

function isTerminal(status: StartupStatus | null): boolean {
  return status?.phase === 'ready' || status?.phase === 'failed';
}

/**
 * Holds the workspace back until the native side has opened the databases, which a one-time
 * storage rewrite can delay by a minute on a large archive. Every command the workspace issues
 * needs state registered only at the end of that sequence.
 */
export default function StartupGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<StartupStatus | null>(null);
  const [pollError, setPollError] = useState<string | null>(null);
  const latest = useRef<StartupStatus | null>(null);
  const appendSyncLog = useAppStore((state) => state.appendSyncLog);

  // Replies from the poll and the event can arrive in either order; only a newer version applies,
  // so a poll begun while optimizing never reverts the screen after Ready has arrived.
  const apply = useCallback((next: StartupStatus) => {
    if (latest.current !== null && latest.current.version >= next.version) return;
    latest.current = next;
    setStatus(next);
  }, []);

  useEffect(() => {
    let active = true;
    let timer: number | null = null;
    let stopListening: (() => void) | null = null;
    const poll = async () => {
      timer = null;
      try {
        const next = await invokeCommand('get_startup_status', {});
        if (!active) return;
        setPollError(null);
        apply(next);
      } catch (error) {
        if (!active) return;
        setPollError(formatNativeError(error));
      }
      if (active && !isTerminal(latest.current)) timer = window.setTimeout(() => void poll(), POLL_INTERVAL_MS);
    };
    // The listener goes first so no change between the fetch and the subscription is missed. The
    // poll does not wait on it: an event bridge that fails must not hold the screen.
    void listen<StartupStatus>(STARTUP_STATUS_EVENT, (event) => {
      if (active) apply(event.payload);
    })
      .then((unlisten) => {
        if (active) stopListening = unlisten;
        else unlisten();
      })
      .catch(() => undefined);
    void poll();
    return () => {
      active = false;
      if (timer !== null) window.clearTimeout(timer);
      stopListening?.();
    };
  }, [apply]);

  useEffect(() => {
    if (status?.phase !== 'ready') return;
    for (const notice of status.notices) {
      if (loggedNoticeIds.has(notice.id)) continue;
      loggedNoticeIds.add(notice.id);
      appendSyncLog(`Startup maintenance, ${notice.file}: ${notice.message}`);
    }
  }, [appendSyncLog, status]);

  if (status?.phase === 'ready') {
    return (
      <>
        {children}
        {status.notices.length > 0 && <StartupNotices notices={status.notices} onAcknowledged={apply} />}
      </>
    );
  }
  return <StartupScreen status={status} pollError={pollError} />;
}

export function StartupScreen({ status, pollError }: { status: StartupStatus | null; pollError: string | null }) {
  // Re-renders each second; the elapsed time itself is read at render so a status arriving
  // between ticks is never measured against a stale clock.
  const [, setTick] = useState(0);
  const running = !isTerminal(status);
  useEffect(() => {
    if (!running) return;
    const interval = window.setInterval(() => setTick((tick) => tick + 1), 1_000);
    return () => window.clearInterval(interval);
  }, [running]);

  const elapsedMs = status ? Math.max(0, Date.now() - status.phaseStartedAtMs) : 0;
  const optimizing = status?.phase === 'optimizingStorage' && elapsedMs >= OPTIMIZING_LABEL_AFTER_MS;
  const heading =
    status?.phase === 'failed'
      ? `${PRODUCT_NAME} could not open your records`
      : optimizing
        ? 'Optimizing storage (one time)…'
        : status?.phase === 'optimizingStorage'
          ? 'Checking storage format…'
          : 'Opening your records…';

  return (
    <main className="app-theme flex h-screen w-screen items-center justify-center overflow-y-auto bg-canvas px-6 text-fg">
      <section className="my-auto w-full max-w-lg rounded-2xl border border-line-strong bg-surface/70 p-8 shadow-2xl" aria-labelledby="startup-heading">
        <p className="text-label leading-4 font-semibold uppercase tracking-[0.24em] text-accent">{PRODUCT_NAME}</p>
        <h1 id="startup-heading" aria-live="polite" className="mt-3 text-2xl font-semibold">
          {heading}
        </h1>
        {status?.phase === 'failed' ? (
          <>
            <p role="alert" className="mt-4 rounded-lg border border-danger/30 bg-danger/10 px-4 py-3 text-sm text-danger">
              {status.error}
            </p>
            <p className="mt-3 text-label leading-5 text-muted">Quit, resolve the problem described, and open {PRODUCT_NAME} again.</p>
          </>
        ) : (
          <>
            {status?.phase === 'optimizingStorage' && status.file && (
              <p className="mt-4 text-sm text-fg-secondary">
                File: <span className="font-mono">{status.file}</span>
              </p>
            )}
            {optimizing && (
              <>
                <p className="mt-1 text-sm tabular-nums text-fg-secondary">Elapsed: {formatElapsed(elapsedMs)}</p>
                <p className="mt-3 text-label leading-5 text-muted">
                  A database from an earlier version is being rewritten into compressed storage and
                  verified row by row. On a large archive this takes about a minute. Quitting is
                  safe: the original is untouched until the copy is verified, and the next launch
                  starts over.
                </p>
              </>
            )}
            {pollError && (
              <p role="alert" className="mt-4 text-label text-danger">
                Startup progress is unavailable: {pollError}. Retrying.
              </p>
            )}
          </>
        )}
      </section>
    </main>
  );
}

function StartupNotices({
  notices,
  onAcknowledged,
}: {
  notices: StartupNotice[];
  onAcknowledged: (status: StartupStatus) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const dismiss = async () => {
    setBusy(true);
    setError(null);
    try {
      onAcknowledged(await invokeCommand('acknowledge_startup_notices', { ids: notices.map((notice) => notice.id) }));
    } catch (reason) {
      setError(formatNativeError(reason));
    } finally {
      setBusy(false);
    }
  };
  return (
    <aside
      aria-labelledby="startup-notices-heading"
      className="app-theme fixed bottom-4 right-4 z-50 w-[min(28rem,calc(100vw-2rem))] rounded-xl border border-line-strong bg-raised p-4 text-fg shadow-2xl"
    >
      <h2 id="startup-notices-heading" className="text-label font-semibold">
        Startup maintenance
      </h2>
      <ul className="mt-2 flex flex-col gap-2 text-caption leading-relaxed text-fg-secondary">
        {notices.map((notice) => (
          <li key={notice.id} className={notice.kind === 'storageNotRewritten' ? 'text-warning' : undefined}>
            <span className="font-mono">{notice.file}</span>: {notice.message}
          </li>
        ))}
      </ul>
      {error && (
        <p role="alert" className="mt-2 text-caption text-danger">
          Could not dismiss: {error}
        </p>
      )}
      <div className="mt-3 flex justify-end">
        <button
          type="button"
          disabled={busy}
          onClick={() => void dismiss()}
          className="focus-ring rounded-lg border border-control px-3 py-1.5 text-label font-semibold text-fg-secondary transition hover:text-fg disabled:opacity-50"
        >
          Dismiss
        </button>
      </div>
    </aside>
  );
}

function formatElapsed(milliseconds: number): string {
  const seconds = Math.floor(milliseconds / 1_000);
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}`;
}
