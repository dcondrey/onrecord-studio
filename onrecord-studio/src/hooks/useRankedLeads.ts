import { useMemo, useRef } from 'react';
import { invokeCommand } from '../ipc';
import { useSinceInstant } from '../components/dashboard/SinceControl';
import { useAppStore } from '../store/useAppStore';
import type { StoryLead } from '../types';
import { useCaseScope } from './useCaseScope';
import { useGatedLoad } from './useGatedLoad';

export interface RankedLeads {
  status: 'pending' | 'ready' | 'failed';
  message?: string;
  /** The leads in the Since window and the active case, in rank order. */
  leads: StoryLead[];
  /** Every ranked lead, unfiltered: a scale drawn over it does not change when a filter does. */
  all: StoryLead[];
  /** How many leads the ranker returned before the window and case filters. */
  total: number;
  /** A reload is running while the previous ranking stays on screen. */
  refreshing: boolean;
  /** 1-based rank within `leads`; undefined for a lead outside them. */
  rankOf(findingId: string): number | undefined;
}

const NO_LEADS: StoryLead[] = [];

/**
 * The one ranking every lead surface shows: the Story desk's attention ranking, cut to the Since
 * window and the active case. Overview, Story desk and the command bar read it here, so "#3" means
 * the same lead wherever it is printed. A refresh keeps the last ranking on screen until the new
 * one lands, so the queue never collapses to a skeleton under the reader. Disabled, it reads
 * nothing.
 */
export function useRankedLeads(enabled = true): RankedLeads {
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const since = useSinceInstant();
  const caseScope = useCaseScope();
  const load = useGatedLoad(enabled ? `story:${findingsRefreshRequests}` : 'off', () => (enabled ? invokeCommand('list_story_leads', { limit: 100 }) : Promise.resolve(NO_LEADS)));
  // The last ranking read while enabled; a disabled read's empty list never stands for one.
  const held = useRef<StoryLead[] | null>(null);
  if (!enabled || load.status === 'failed') held.current = null;
  else if (load.status === 'ready') held.current = load.data;
  const all = enabled ? held.current : null;

  return useMemo(() => {
    const leads = (all ?? NO_LEADS).filter((lead) => (!since || lead.firstSeenAt > since) && (caseScope.status !== 'ready' || caseScope.value.findingIds.has(lead.findingId)));
    const ranks = new Map(leads.map((lead, index) => [lead.findingId, index + 1]));
    return {
      status: all ? 'ready' : load.status === 'failed' ? 'failed' : 'pending',
      message: load.status === 'failed' ? load.message : undefined,
      leads,
      all: all ?? NO_LEADS,
      total: all?.length ?? 0,
      refreshing: load.status === 'pending' && all !== null,
      rankOf: (findingId: string) => ranks.get(findingId),
    };
  }, [all, caseScope, load, since]);
}
