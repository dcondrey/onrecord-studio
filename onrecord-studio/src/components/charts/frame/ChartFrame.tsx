import { useId, useRef, useState, type ReactNode } from 'react';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import { cn } from '../../../utils/cn';
import AnalyticsUnavailable from '../AnalyticsUnavailable';
import { CLAIM_TAG, type Claim } from '../core/claim';
import { absenceLegend, isAbsence, type AbsenceState } from '../core/nulls';
import { ChartMotionContext, useChartMotion } from '../hooks/useChartMotion';
import { useChartSize } from '../hooks/useChartSize';
import type { LegendFocus } from '../hooks/useLegendFocus';
import { Legend, type LegendItem } from './Legend';
import { SourceCaption, type ChartSources } from './SourceCaption';
import { TableTwin, type TwinTable } from './TableTwin';
import { HatchProvider, VizDefs, useSvgId } from './VizDefs';

export type ChartStatus = 'ready' | 'loading' | 'stale' | 'empty' | 'error' | 'scoped-out';

export interface ChartFrameProps {
  /** Names the measure; the visible heading. */
  title: string;
  /** One deterministic sentence from the figure's template; null while loading. */
  headline: string | null;
  claim?: Claim | 'mixed';
  sources: ChartSources;
  /** Screen-reader description: counts, ranges, gaps, excluded counts. */
  summary: string;
  status: ChartStatus;
  emptyText?: string;
  error?: string | null;
  scopedOutText?: string;
  table: TwinTable;
  tableDefault?: 'chart' | 'table';
  legend?: LegendItem[];
  /** False when the figure's own legend already explains each kind of absence it draws. */
  autoAbsenceLegend?: boolean;
  legendFocus?: LegendFocus;
  /** The single filter row, above the plot. */
  controls?: ReactNode;
  minHeight: number;
  /**
   * The plot's height at a measured width, for figures whose rows grow when labels move above
   * their bars at narrow widths. Default: minHeight.
   */
  plotHeight?(width: number): number;
  /**
   * Set when the plot's marks are keyboard-focusable: the plot becomes a named group with this
   * role description. Otherwise the plot is one image labelled by the headline.
   */
  interactive?: { roledescription: string };
  /** Remembers the chart/table choice per figure. Defaults to the title. */
  storageKey?: string;
  className?: string;
  children(size: { width: number; height: number }): ReactNode;
}

const VIEW_STORAGE_PREFIX = 'onrecord-chart-view:';

function readView(key: string, fallback: 'chart' | 'table'): 'chart' | 'table' {
  try {
    const stored = globalThis.localStorage?.getItem(VIEW_STORAGE_PREFIX + key);
    return stored === 'chart' || stored === 'table' ? stored : fallback;
  } catch {
    return fallback;
  }
}

function writeView(key: string, view: 'chart' | 'table') {
  try {
    globalThis.localStorage?.setItem(VIEW_STORAGE_PREFIX + key, view);
  } catch {
    // Storage unavailable: the choice lasts for this render only.
  }
}

function absenceStates(table: TwinTable): AbsenceState[] {
  const states = new Set<AbsenceState>();
  for (const row of table.rows) for (const cell of Object.values(row)) if (isAbsence(cell)) states.add(cell.state);
  return [...states];
}

/** The claim tag beside the title: the strongest claim wears the solid crimson badge. */
function ClaimTags({ claim }: { claim: ChartFrameProps['claim'] }) {
  if (!claim || claim === 'descriptive') return null;
  return (
    <span className="flex flex-wrap gap-1.5">
      {(claim === 'counted' || claim === 'mixed') && <Badge tone="violation">{CLAIM_TAG.counted}</Badge>}
      {(claim === 'statistical' || claim === 'mixed') && <Badge dashed>{CLAIM_TAG.statistical}</Badge>}
    </span>
  );
}

/**
 * Every chart's frame: title, claim tag, headline, the plot or its table twin, legend (with an
 * entry for each kind of absence the data contains), and the source caption. Loading, empty,
 * error and scoped-out states contain text only; no frame ever draws placeholder data.
 */
export default function ChartFrame(props: ChartFrameProps) {
  const { title, headline, claim, sources, summary, status, table, minHeight, interactive, className } = props;
  const titleId = useId();
  const summaryId = useId();
  const hatchId = useSvgId('viz-hatch');
  const plotRef = useRef<HTMLDivElement>(null);
  const { width } = useChartSize(plotRef, { fallbackWidth: 640 });
  const motion = useChartMotion();
  const storageKey = props.storageKey ?? title;
  const [view, setView] = useState<'chart' | 'table'>(() => readView(storageKey, props.tableDefault ?? 'chart'));
  const showsData = status === 'ready' || status === 'stale';
  const legend = [...(props.legend ?? []), ...(props.autoAbsenceLegend === false ? [] : absenceLegend(absenceStates(table)))];

  const toggleView = () => {
    const next = view === 'chart' ? 'table' : 'chart';
    setView(next);
    writeView(storageKey, next);
  };

  let body: ReactNode;
  if (status === 'loading') {
    body = <p className="text-body text-muted">Loading…</p>;
  } else if (status === 'empty') {
    body = <p className="text-body text-muted">{props.emptyText ?? 'Nothing to plot for this selection.'}</p>;
  } else if (status === 'error') {
    body = <AnalyticsUnavailable heading="This chart could not be drawn" detail={props.error ?? undefined} />;
  } else if (status === 'scoped-out') {
    body = <p className="text-body text-muted">{props.scopedOutText ?? 'Hidden while a case scope is active.'}</p>;
  } else if (view === 'table') {
    body = <TableTwin table={table} />;
  } else if (width > 0) {
    const size = { width, height: props.plotHeight ? props.plotHeight(width) : minHeight };
    body = (
      <svg
        width={size.width}
        height={size.height}
        className="block"
        {...(interactive
          ? { role: 'group', 'aria-roledescription': interactive.roledescription, 'aria-label': headline ?? title }
          : { role: 'img', 'aria-label': headline ?? title })}
      >
        <VizDefs hatchId={hatchId} />
        <HatchProvider value={hatchId}>
          <ChartMotionContext.Provider value={motion}>{props.children(size)}</ChartMotionContext.Provider>
        </HatchProvider>
      </svg>
    );
  }

  return (
    <figure aria-labelledby={titleId} aria-describedby={summaryId} className={cn('m-0 grid min-w-0 gap-3', className)}>
      <figcaption className="grid gap-1">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <h3 id={titleId} className="text-heading text-fg">{title}</h3>
            <ClaimTags claim={claim} />
          </div>
          {showsData && (
            <Button size="sm" variant="ghost" className="ml-auto" aria-pressed={view === 'table'} onClick={toggleView}>
              View as table
            </Button>
          )}
        </div>
        {headline && <p className="text-headline text-fg">{headline}</p>}
      </figcaption>
      {props.controls && <div className="flex flex-wrap items-center gap-2">{props.controls}</div>}
      {status === 'stale' && (
        <p role="status" className="text-caption text-warning">Controls changed. The chart still shows the previous result until it is run again.</p>
      )}
      <div
        ref={plotRef}
        className={cn('relative min-w-0', (status === 'loading' || status === 'empty' || status === 'scoped-out') && 'grid place-items-center')}
        style={{ minHeight: view === 'table' && showsData ? undefined : minHeight, background: 'var(--viz-surface)', opacity: status === 'stale' ? 0.5 : undefined }}
      >
        {body}
      </div>
      <p id={summaryId} className="sr-only">{summary}</p>
      {showsData && legend.length > 0 && view === 'chart' && <Legend items={legend} focus={props.legendFocus} />}
      <SourceCaption sources={sources} />
    </figure>
  );
}
