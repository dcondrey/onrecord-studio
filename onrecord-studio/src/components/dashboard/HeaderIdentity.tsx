import type { PortalRegistration } from '../../types';
import { useAppStore } from '../../store/useAppStore';
import { viewLabel } from './NavigationControls';

/** The view's name, then the source it is scoped to, separated by a rule rather than a chevron so it never reads as a path. */
export function HeaderIdentity({ activePortal }: { activePortal: PortalRegistration | null }) {
  const view = useAppStore((state) => state.workspaceView);
  const source = activePortal?.displayName ?? 'All saved sources';
  return (
    <div className="flex min-w-0 items-center gap-3">
      <h1 className="shrink-0 text-heading text-fg">{viewLabel(view)}</h1>
      <span aria-hidden="true" className="h-4 w-px shrink-0 bg-line-strong" />
      <p className="min-w-0 truncate text-control text-fg-secondary" title={source}>
        <span className="sr-only">Source: </span>{source}
      </p>
    </div>
  );
}
