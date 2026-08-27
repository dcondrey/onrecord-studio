import { beforeEach, describe, expect, it } from 'vitest';
import { useAppStore } from '../store/useAppStore';

// These tests back the run-scoping comment on `setPortalSyncState` in src/store/useAppStore.ts.
// The store is a module-scope singleton with no reset action for sync state, so each case
// restores the slice it touches rather than relying on construction order.

const RESET = { syncingPortalKeys: [] as string[], syncRunIdsByPortal: {}, isSyncing: false };

function state() {
  return useAppStore.getState();
}

describe('setPortalSyncState run scoping', () => {
  beforeEach(() => useAppStore.setState({ ...RESET }));

  it('tracks a single run from start to stop', () => {
    state().setPortalSyncState('portal-a', true, 'run-1');
    expect(state().syncingPortalKeys).toEqual(['portal-a']);
    expect(state().syncRunIdsByPortal).toEqual({ 'portal-a': 'run-1' });
    expect(state().isSyncing).toBe(true);

    state().setPortalSyncState('portal-a', false, 'run-1');
    expect(state().syncingPortalKeys).toEqual([]);
    expect(state().syncRunIdsByPortal).toEqual({});
    expect(state().isSyncing).toBe(false);
  });

  it('keeps portals independent and the key list sorted', () => {
    state().setPortalSyncState('portal-b', true, 'run-b');
    state().setPortalSyncState('portal-a', true, 'run-a');
    expect(state().syncingPortalKeys).toEqual(['portal-a', 'portal-b']);

    state().setPortalSyncState('portal-b', false, 'run-b');
    expect(state().syncingPortalKeys).toEqual(['portal-a']);
    expect(state().isSyncing).toBe(true);
  });

  // The defended case. This is what the guard in the store exists for.
  it('ignores a late stop from a superseded run', () => {
    state().setPortalSyncState('portal-a', true, 'run-1');
    state().setPortalSyncState('portal-a', true, 'run-2');
    expect(state().syncRunIdsByPortal).toEqual({ 'portal-a': 'run-2' });

    // run-1's syncStopped telemetry arrives after run-2 already registered.
    state().setPortalSyncState('portal-a', false, 'run-1');

    expect(state().syncingPortalKeys).toEqual(['portal-a']);
    expect(state().syncRunIdsByPortal).toEqual({ 'portal-a': 'run-2' });
    expect(state().isSyncing).toBe(true);
  });

  // The gap. Pinned as CURRENT behavior, not as desired behavior: a stop with no run id skips
  // the guard entirely, because the condition requires a truthy syncRunId to compare against.
  // Both live stop paths (the Stop button and the runtime poll) omit it, so the guard above
  // cannot fire in production today. See TODO(protocol) in src/store/useAppStore.ts.
  it('DOCUMENTS A GAP: an unscoped stop clears a live replacement run', () => {
    state().setPortalSyncState('portal-a', true, 'run-1');
    state().setPortalSyncState('portal-a', true, 'run-2');

    state().setPortalSyncState('portal-a', false);

    // If this assertion ever flips, the gap has been closed and this test should become the
    // positive assertion (['portal-a'] / { 'portal-a': 'run-2' } / true) instead.
    expect(state().syncingPortalKeys).toEqual([]);
    expect(state().isSyncing).toBe(false);
  });

  // The inversion. A stale syncStarted has no guard at all, so it can rewrite the active run id
  // and then the genuine stop for the live run is rejected as stale, stranding the portal.
  it('DOCUMENTS A GAP: a stale start can strand a portal in the syncing set', () => {
    state().setPortalSyncState('portal-a', true, 'run-2');
    state().setPortalSyncState('portal-a', true, 'run-1');
    expect(state().syncRunIdsByPortal).toEqual({ 'portal-a': 'run-1' });

    state().setPortalSyncState('portal-a', false, 'run-2');

    expect(state().syncingPortalKeys).toEqual(['portal-a']);
    expect(state().isSyncing).toBe(true);
  });

  it('lets a stop with no prior run id through, which is what the runtime poll relies on', () => {
    // The poll is the only writer that legitimately stops a portal it never started, so an
    // unscoped stop must stay effective when no run is registered.
    state().setPortalSyncState('portal-a', false);
    expect(state().syncingPortalKeys).toEqual([]);

    state().setPortalSyncState('portal-a', true, 'run-1');
    useAppStore.setState({ syncRunIdsByPortal: {} });
    state().setPortalSyncState('portal-a', false, 'run-1');
    expect(state().syncingPortalKeys).toEqual([]);
  });

  it('does not register a run id for a start that omits one', () => {
    state().setPortalSyncState('portal-a', true);
    expect(state().syncingPortalKeys).toEqual(['portal-a']);
    expect(state().syncRunIdsByPortal).toEqual({});
  });
});

describe('telemetry counters', () => {
  beforeEach(() => {
    useAppStore.setState({ ...RESET });
    state().resetTelemetryMetrics();
  });

  it('accumulates metrics and clamps negative byte counts', () => {
    state().incrementMetric('pagesProcessed');
    state().incrementMetric('pagesProcessed');
    state().incrementMetric('documentsDownloaded');
    state().addBytesTransferred(1_024);
    state().addBytesTransferred(-5_000);

    expect(state().metrics.pagesProcessed).toBe(2);
    expect(state().metrics.documentsDownloaded).toBe(1);
    expect(state().metrics.bytesTransferred).toBe(1_024);
  });

  // LIMIT: metrics are global, not per-run. resetTelemetryMetrics is the only thing that clears
  // them and nothing calls it on run start, so a new run inherits the previous run's totals.
  it('DOCUMENTS A GAP: metrics survive a stop/start cycle on the same portal', () => {
    state().setPortalSyncState('portal-a', true, 'run-1');
    state().incrementMetric('documentsDownloaded');
    state().setPortalSyncState('portal-a', false, 'run-1');
    state().setPortalSyncState('portal-a', true, 'run-2');

    expect(state().metrics.documentsDownloaded).toBe(1);
  });

  it('resetTelemetryMetrics clears counters without touching sync state', () => {
    state().setPortalSyncState('portal-a', true, 'run-1');
    state().incrementMetric('anomaliesFound');
    state().resetTelemetryMetrics();

    expect(state().metrics.anomaliesFound).toBe(0);
    expect(state().syncLogs).toEqual([]);
    expect(state().syncingPortalKeys).toEqual(['portal-a']);
  });

  it('keeps the sync log newest-first and bounded', () => {
    for (let index = 0; index < 260; index += 1) state().appendSyncLog(`entry-${index}`);
    const logs = state().syncLogs;
    expect(logs.length).toBeLessThanOrEqual(250);
    expect(logs[0]).toContain('entry-259');
    expect(logs.some((line) => line.includes('entry-0'))).toBe(false);
  });
});
