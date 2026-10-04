import { invokeCommand } from '../ipc';
import { useAppStore } from '../store/useAppStore';
import type { RefreshFindingsSummary } from '../types';

/**
 * Runs the detector sweep and records which checks did not run. Both callers (the post-ingest
 * hook and the inbox Refresh button) go through here so the store always describes the sweep
 * started last, whichever settles first.
 */
export async function runFindingsRefresh(): Promise<RefreshFindingsSummary> {
  const { beginFindingsRefresh, settleFindingsRefresh } = useAppStore.getState();
  const sequence = beginFindingsRefresh();
  try {
    const summary = await invokeCommand('refresh_findings', {});
    settleFindingsRefresh(sequence, { status: 'completed', notRun: summary.notRun });
    return summary;
  } catch (error) {
    settleFindingsRefresh(sequence, { status: 'failed' });
    throw error;
  }
}
