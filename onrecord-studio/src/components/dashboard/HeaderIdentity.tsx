import type { PortalRegistration } from '../../types';

/** The active source's name. The sidebar already names the view, so the header does not repeat it. */
export function HeaderIdentity({ activePortal }: { activePortal: PortalRegistration | null }) {
  return (
    <div className="min-w-0">
      <h1 className="truncate text-heading text-fg" title={activePortal?.displayName}>
        {activePortal?.displayName ?? 'All saved sources'}
      </h1>
    </div>
  );
}
