import { useMemo, type ReactNode } from 'react';
import { useAppStore } from '../../store/useAppStore';

const DATELINE = new Intl.DateTimeFormat(undefined, { weekday: 'long', month: 'long', day: 'numeric', year: 'numeric' });

/** The reader's calendar date as YYYY-MM-DD, matching the dateline's words rather than the UTC date. */
function localDate(date: Date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}-${String(date.getDate()).padStart(2, '0')}`;
}

/** The active source's name, or every source when none is chosen: what the page's figures cover. */
export function useScopeName(): string {
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const portals = useAppStore((state) => state.portals);
  return useMemo(
    () => (activePortalKey ? portals.find((saved) => saved.registration.portalKey === activePortalKey)?.registration.displayName ?? activePortalKey : 'All saved sources'),
    [activePortalKey, portals],
  );
}

/** Today in the reader's zone, as the dateline prints it. */
export function Dateline() {
  const today = new Date();
  return <time dateTime={localDate(today)}>{DATELINE.format(today)}</time>;
}

/**
 * A page's nameplate: an overline (the dateline or desk name, then the scope), the title with the
 * page's tabs beside it, the page's actions at the right, and the heavy rule that closes it.
 */
export default function Masthead({ overline, title, aside, tabs, actions }: {
  overline: ReactNode;
  title: string;
  /** Set right after the title, such as a method note. */
  aside?: ReactNode;
  tabs?: ReactNode;
  actions?: ReactNode;
}) {
  const scope = useScopeName();
  return (
    // The actions share the dateline's row, so the title and its tabs keep one line at every width
    // the app is laid out for. The row keeps a control's height when a state has no actions, so the
    // masthead stays one height across loading, empty and error.
    <header className="border-b-2 border-fg/85 px-1 pb-3">
      <div className="flex min-h-8 flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <p className="text-overline uppercase text-muted">
          {overline}
          <span aria-hidden="true"> · </span>
          {scope}
        </p>
        {actions && <div className="flex flex-wrap items-center gap-2">{actions}</div>}
      </div>
      <div className="mt-1 flex flex-wrap items-baseline gap-x-4 gap-y-2">
        <h2 className="text-title text-fg">{title}</h2>
        {aside}
        {tabs}
      </div>
    </header>
  );
}
