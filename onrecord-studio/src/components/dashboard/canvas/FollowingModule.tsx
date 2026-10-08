import { invokeCommand } from '../../../ipc';
import { useGatedLoad } from '../../../hooks/useGatedLoad';
import { useAppStore } from '../../../store/useAppStore';
import { formatShortDate } from '../../objects/labels';
import Badge from '../../ui/Badge';
import Icon, { type IconName } from '../../ui/icons';
import Skeleton from '../../ui/Skeleton';
import StandingSearchPanel from '../StandingSearchPanel';
import WatchlistPanel from '../WatchlistPanel';
import CanvasModule from './CanvasModule';

const ROWS = 4;

interface FollowRow {
  key: string;
  icon: IconName;
  label: string;
  meta: string;
  due?: boolean;
}

const plural = (n: number, one: string, many = `${one}s`) => `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`;

/**
 * What the reader follows: watches on stored records and saved searches re-run on corpus change.
 * The compact face lists the latest of each with when it last ran; it expands into both panels.
 * A watch never checked says so; it is not shown as "no changes".
 */
export default function FollowingModule({ expanded, onToggle, span }: { expanded: boolean; onToggle(): void; span?: string }) {
  const refreshes = useAppStore((state) => state.findingsRefreshRequests);
  const watches = useGatedLoad(`watches:${refreshes}`, () => invokeCommand('list_watches', {}));
  const searches = useGatedLoad(`searches:${refreshes}`, () => invokeCommand('list_standing_queries', {}));
  const common = {
    id: 'canvas-following',
    icon: 'search' as const,
    title: 'Following',
    expanded,
    onToggle,
    expandLabel: 'Show watches and standing searches',
    collapseLabel: 'Hide watches and standing searches',
    detail: (
      <section aria-label="Following" className="grid grid-cols-1 items-start gap-4 2xl:grid-cols-2">
        <WatchlistPanel />
        <StandingSearchPanel />
      </section>
    ),
    span,
  };

  if (watches.status === 'pending' || searches.status === 'pending') {
    return (
      <CanvasModule {...common} headline={null}>
        <Skeleton variant="lines" lines={4} label="Reading what you follow" />
      </CanvasModule>
    );
  }
  if (watches.status === 'failed' && searches.status === 'failed') {
    return <CanvasModule {...common} headline="Watches and standing searches couldn’t be read." />;
  }

  const watchList = watches.status === 'ready' ? watches.data : [];
  const searchList = searches.status === 'ready' ? searches.data : [];
  const due = searchList.filter((summary) => summary.due).length;
  const rows: FollowRow[] = [
    ...searchList.map((summary): FollowRow => ({
      key: `search:${summary.query.id}`,
      icon: 'search',
      label: summary.query.label,
      meta: summary.lastRun ? `ran ${formatShortDate(summary.lastRun.ranAt)}` : 'not run yet',
      due: summary.due,
    })),
    ...watchList.map((summary): FollowRow => {
      const changes = summary.lastDimensions.reduce((sum, dimension) => sum + dimension.changeCount, 0);
      return {
        key: `watch:${summary.watch.id}`,
        icon: 'activity',
        label: summary.watch.label,
        meta: summary.lastCheckedAt ? `${plural(changes, 'change')} · ${formatShortDate(summary.lastCheckedAt)}` : 'not checked yet',
      };
    }),
  ];
  const parts = [
    watches.status === 'ready' ? plural(watchList.length, 'watch', 'watches') : null,
    searches.status === 'ready' ? plural(searchList.length, 'standing search', 'standing searches') : null,
  ].filter(Boolean);
  const headline = rows.length === 0
    ? 'You follow nothing yet. Watch a record or save a search to have changes surface here.'
    : `${parts.join(' and ')}${due > 0 ? `; ${plural(due, 'search')} ${due === 1 ? 'is' : 'are'} due to run` : ''}.`;

  return (
    <CanvasModule {...common} headline={headline}>
      {rows.length > 0 && (
        <ul className="grid grid-cols-1 gap-2.5">
          {rows.slice(0, ROWS).map((row) => (
            <li key={row.key} className="flex min-w-0 items-start gap-2.5">
              <span className="canvas-module-icon mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-md"><Icon name={row.icon} size="xs" /></span>
              <span className="grid min-w-0 flex-1">
                <span className="truncate text-body text-fg" title={row.label}>{row.label}</span>
                <span className="text-caption tabular-nums text-muted">{row.icon === 'search' ? 'Standing search' : 'Watch'} · {row.meta}</span>
              </span>
              {row.due && <Badge tone="neutral" className="mt-0.5">Due</Badge>}
            </li>
          ))}
        </ul>
      )}
      {rows.length > ROWS && <p className="mt-1 text-caption tabular-nums text-muted">{(rows.length - ROWS).toLocaleString('en-US')} more</p>}
      {(watches.status === 'failed' || searches.status === 'failed') && (
        <p className="mt-2 text-caption text-warning">{watches.status === 'failed' ? 'Watches' : 'Standing searches'} couldn’t be read.</p>
      )}
    </CanvasModule>
  );
}
