import { useMemo } from 'react';
import { useAppStore } from '../../../store/useAppStore';
import { useFirstRunDataStore } from '../../../store/useFirstRunDataStore';
import type { AnalyticalDatasetSummary } from '../../../types';
import { rankObservations, type DiscoveryObservation } from '../../../utils/automaticDiscovery';
import LeadFigure from '../../charts/figures/LeadFigure';
import Button from '../../ui/Button';
import FamilyBadge from '../../ui/FamilyBadge';
import Icon from '../../ui/icons';
import Skeleton from '../../ui/Skeleton';
import AutomaticDiscovery, { discoveryLeadSources, discoveryObservations } from '../AutomaticDiscovery';
import { useAutomaticDiscovery } from '../AutomaticDiscoveryProvider';
import CanvasModule from './CanvasModule';

const NEXT_LEADS = 3;

const isSpending = (item: DiscoveryObservation, dataset: AnalyticalDatasetSummary | undefined) =>
  item.family === 'SPENDING_RECORDS' || dataset?.datasetKind === 'OPERATING_BUDGET' || dataset?.datasetKind === 'BUDGET_DIMENSION';

/**
 * The canvas's headline module: the top-ranked lead the table checks raised, led by its own title
 * and drawn with its stored figure, then the next leads in rank order. It expands into every lead,
 * gap and note the checks produced.
 */
export default function LeadsModule({ expanded, onToggle, span }: { expanded: boolean; onToggle(): void; span?: string }) {
  const { controller, state } = useAutomaticDiscovery();
  const setWorkspaceView = useAppStore((app) => app.setWorkspaceView);
  const fetchingCoreData = useFirstRunDataStore((first) => first.running);
  const ranked = useMemo(
    () => (state ? rankObservations(discoveryObservations(state).filter((item) => item.kind === 'lead'), 1 + NEXT_LEADS) : []),
    [state],
  );
  if (!state || !controller) return null;
  const datasets = state.schema?.datasets ?? [];
  const datasetOf = (item: DiscoveryObservation) => datasets.find((dataset) => dataset.snapshotId === item.snapshotId);
  const explore = (item: DiscoveryObservation) => {
    if (!item.query) return;
    controller.inspect(item.query.sql);
    setWorkspaceView('analysis');
  };
  const [top, ...next] = ranked;
  const topDataset = top ? datasetOf(top) : undefined;
  const common = {
    id: 'canvas-leads',
    icon: 'flag' as const,
    expanded,
    onToggle,
    expandLabel: 'Show every lead in your tables',
    collapseLabel: 'Hide every lead in your tables',
    detail: <AutomaticDiscovery embedded />,
    span,
  };

  if (!state.checkedAt) {
    return (
      <CanvasModule {...common} hero title="Top lead in your tables" headline={null}>
        <Skeleton variant="chart" height={180} label="Reading your tables for leads" />
      </CanvasModule>
    );
  }
  if (!top) {
    const empty = state.schema?.datasets.length === 0;
    return (
      <CanvasModule
        {...common}
        title="Leads in your tables"
        headline={empty ? 'No City tables are imported yet, so there is nothing to check.' : 'The completed checks found nothing to raise in these tables.'}
      >
        {empty && (
          <Button size="sm" className="w-fit" disabled={fetchingCoreData} onClick={() => { void useFirstRunDataStore.getState().run(controller); }}>
            Get San Diego’s core public data
          </Button>
        )}
      </CanvasModule>
    );
  }

  return (
    <CanvasModule
      {...common}
      title="Top lead in your tables"
      badges={<FamilyBadge family={isSpending(top, topDataset) ? 'Spending' : 'Records request'} />}
      headline={top.title}
      hero
    >
      <div className="flex min-w-0 flex-1 flex-col gap-4 [&>*]:min-w-0">
        {top.figure && topDataset ? (
          <LeadFigure
            figure={top.figure}
            headline={null}
            size="card"
            sources={discoveryLeadSources(top, topDataset, datasets)}
            onActivate={top.query ? () => explore(top) : undefined}
          />
        ) : (
          <p className="max-w-prose text-body text-fg-secondary">{top.detail}</p>
        )}
        <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
          <p className="min-w-0 flex-1 max-w-prose text-body text-fg-secondary"><span className="font-medium text-lead">Investigate</span> {top.question}</p>
          {top.query && <Button size="sm" onClick={() => explore(top)}>Explore this lead</Button>}
        </div>
        {next.length > 0 && (
          <div className="mt-auto border-t border-line pt-3">
            <h4 className="text-overline uppercase text-muted">Next in rank</h4>
            <ol className="-mx-2 mt-1.5 grid grid-cols-1 gap-px">
              {next.map((item, index) => (
                <li key={item.id}>
                  <button
                    type="button"
                    disabled={!item.query}
                    onClick={() => explore(item)}
                    className="focus-ring group flex w-full items-center gap-3 rounded-md px-2 py-2 text-left transition-colors duration-fast enabled:hover:bg-raised disabled:cursor-default"
                  >
                    <span className="w-5 shrink-0 text-caption tabular-nums text-muted">#{index + 2}</span>
                    <span className="min-w-0 flex-1 truncate text-body text-fg" title={item.title}>{item.title}</span>
                    <span className="hidden shrink-0 truncate text-caption text-muted sm:block sm:max-w-[12rem]">{datasetOf(item)?.displayName}</span>
                    {item.query && <Icon name="arrow" size="sm" className="shrink-0 text-muted transition-colors group-hover:text-fg" />}
                  </button>
                </li>
              ))}
            </ol>
          </div>
        )}
      </div>
    </CanvasModule>
  );
}
