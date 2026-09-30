import Icon from '../ui/icons';
import type { PortalRegistration } from '../../types';
import type { WorkspaceView } from '../../store/useAppStore';

const viewLabels: Record<WorkspaceView, string> = {
  overview: 'San Diego research desk',
  story: 'Story desk',
  patterns: 'Corpus connections',
  analysis: 'Analysis workbench',
  evidence: 'Evidence workspace',
  activity: 'Ingestion activity',
  briefing: 'Presentation briefing',
  meetings: 'Public meetings',
  compare: 'Agency comparison',
};

/** The breadcrumb and title block at the left of the header: current view, active source, and its
 * verified badge. */
export function HeaderIdentity({ workspaceView, activePortal }: { workspaceView: WorkspaceView; activePortal: PortalRegistration | null }) {
  return (
    <div className="min-w-0 flex-1 lg:w-[210px] lg:flex-none xl:w-[270px]">
      <div className="flex items-center gap-2 text-[9px] font-medium text-slate-400">
        <span>{viewLabels[workspaceView]}</span>
        <Icon name="chevron" className="h-2.5 w-2.5" />
        <span className="truncate text-slate-400">{activePortal?.displayName ?? 'All saved sources'}</span>
      </div>
      <div className="mt-1 flex min-w-0 items-center gap-2">
        <h1 className="truncate text-[15px] font-semibold tracking-[-0.015em] text-slate-100">
          {activePortal?.displayName ?? 'City of San Diego'}
        </h1>
        {activePortal && (
          <span className="hidden shrink-0 items-center gap-1 rounded-full border border-emerald-500/20 bg-emerald-500/[0.08] px-2 py-0.5 text-[8px] font-semibold uppercase tracking-wider text-emerald-300 sm:flex">
            <Icon name="shield" className="h-2.5 w-2.5" /> Verified
          </span>
        )}
      </div>
    </div>
  );
}
