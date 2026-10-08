import { useEffect, useState, type ReactNode } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { Finding } from '../../types';
import { leadProse, tableLeadSources } from '../../utils/tableLeadSources';
import LeadFigure from '../charts/figures/LeadFigure';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import { useDiscoveryController } from './AutomaticDiscoveryProvider';

/** The stored finding the figure was read with, or why it could not be read. */
type FigureRead = { status: 'pending' } | { status: 'ready'; finding: Finding | null } | { status: 'failed'; message: string };

interface StoredLeadFigureProps {
  findingId: string;
  /** Shown while reading, and kept when the finding stored no figure. */
  fallback: ReactNode;
  /** Reserve the card's height while reading (a row known to be a table lead). */
  reserve?: boolean;
}

/**
 * A stored lead's figure as a card, read by the finding's id (the list omits figures). A lead with
 * a figure leads with it and keeps its prose under 'Figures as text'; a finding without one keeps
 * `fallback`. Every read is gated on the finding it was made for.
 */
export default function StoredLeadFigure({ findingId, fallback, reserve = false }: StoredLeadFigureProps) {
  const gate = useLatestRequestGate();
  const controller = useDiscoveryController();
  const [read, setRead] = useState<FigureRead>({ status: 'pending' });

  useEffect(() => {
    const request = gate.begin();
    setRead({ status: 'pending' });
    invokeCommand('list_findings', { filter: { id: findingId, includeDismissed: true }, limit: 1 })
      .then((found) => {
        if (gate.isCurrent(request)) setRead({ status: 'ready', finding: found.find((row) => row.id === findingId) ?? null });
      })
      .catch((reason: unknown) => {
        if (gate.isCurrent(request)) setRead({ status: 'failed', message: formatNativeError(reason) });
      });
    return gate.invalidate;
  }, [findingId, gate]);

  if (read.status === 'pending') return reserve ? <Skeleton variant="chart" height={160} label="Reading the lead’s figure" /> : <>{fallback}</>;
  if (read.status === 'failed') {
    return (
      <>
        <InlineError summary="The lead's figure couldn’t be read." detail={read.message} />
        {fallback}
      </>
    );
  }
  const finding = read.finding;
  if (!finding?.figure) return <>{fallback}</>;
  const sources = tableLeadSources(finding.explanation);
  const tables = sources?.tables.map((table) => table.parts?.name ?? table.line) ?? [];
  const sql = sources?.sql;
  // A mark opens the lead's own query in Analysis, as its card there does.
  const onActivate = controller && sql ? () => { controller.inspect(sql); useAppStore.getState().setWorkspaceView('analysis'); } : undefined;
  return (
    <div className="flex flex-col gap-2">
      <LeadFigure
        figure={finding.figure}
        headline={null}
        size="card"
        sources={{ tables: tables.length ? tables : ['tables not recorded with this finding'], retrievedAt: sources?.tables[0]?.parts?.retrievedOn }}
        onActivate={onActivate}
      />
      <details className="text-body">
        <summary className="focus-ring w-fit cursor-pointer rounded text-caption text-muted hover:text-fg">Figures as text</summary>
        <p className="mt-1 whitespace-pre-line break-words text-muted">{leadProse(finding.explanation)}</p>
      </details>
    </div>
  );
}
