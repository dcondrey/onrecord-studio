import { useState } from 'react';
import type { CorpusEntityEdge, CorpusEntityKind, CorpusEntityNode } from '../../types';
import type { NeighborList } from '../../hooks/useEntityNeighbors';
import { LOAD_ALL_PAGE_GUARD, NEIGHBOR_PAGE_SIZE } from '../../hooks/useEntityNeighbors';
import { csvDocument } from '../../utils/csv';
import { EdgeBasisChip, ValidationGradeChip, gradeLabel, edgeVisual } from '../ui/EvidenceBasis';
import VirtualList from '../ui/VirtualList';

const ROW_HEIGHT = 40;
const COLUMNS = 'grid-cols-[minmax(0,1.6fr)_90px_minmax(0,1fr)_minmax(0,1.2fr)_64px]';

interface Props {
  centre: CorpusEntityNode;
  list: Extract<NeighborList, { status: 'ready' }>;
  kindLabel: (kind: CorpusEntityKind) => string;
  onSelect: (entityId: string) => void;
  onLoadMore: () => void;
  onLoadAll: () => void;
}

const CSV_HEADER = ['Neighbour', 'Entity ID', 'Kind', 'Validation grade', 'Relationship', 'Basis', 'Shared documents', 'Confidence %', 'Valid from', 'Valid until'];

function csvRow(edge: CorpusEntityEdge, kindLabel: Props['kindLabel']) {
  return [
    edge.neighbor.displayValue,
    edge.neighbor.entityId,
    kindLabel(edge.neighbor.kind),
    gradeLabel(edge.neighbor.validationGrade),
    edgeVisual(edge.kind, edge.basis).label,
    edge.basis,
    edge.sharedDocumentCount,
    (edge.confidenceBasisPoints / 100).toFixed(1),
    edge.validFrom ?? '',
    edge.validUntil ?? '',
  ];
}

/**
 * Every neighbour of the centre, strongest first, a page at a time. The status line always states
 * how many are loaded of how many exist, so a partially loaded list is never read as the whole.
 */
export default function EntityNeighborTable({ centre, list, kindLabel, onSelect, onLoadMore, onLoadAll }: Props) {
  const [copied, setCopied] = useState<string | null>(null);
  const { edges, counts, nextCursor } = list;
  const total = counts?.total ?? null;
  const loaded = edges.length.toLocaleString('en-US');
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(csvDocument(CSV_HEADER, edges.map((edge) => csvRow(edge, kindLabel))));
      setCopied(
        total !== null && edges.length < total
          ? `Copied ${loaded} of ${total.toLocaleString('en-US')} neighbours. Load the rest to include them.`
          : `Copied ${loaded} neighbours.`,
      );
    } catch {
      setCopied('The clipboard is unavailable.');
    }
  };

  const status =
    total !== null
      ? `${loaded} of ${total.toLocaleString('en-US')} neighbours loaded`
      : list.countFailed
        ? `${loaded} neighbours loaded; the total could not be counted`
        : `${loaded} neighbours loaded; counting the total`;
  const hidden = counts !== null ? counts.unfilteredTotal - counts.total : 0;
  const drifted = nextCursor === null && total !== null && edges.length !== total;

  return (
    <section aria-label={`All neighbours of ${centre.displayValue}`} className="mt-3 rounded-lg border border-slate-800">
      <div className="flex flex-wrap items-center gap-2 border-b border-slate-800 px-2 py-1.5 text-[9px] text-slate-400">
        <span role="status" className="font-mono text-slate-300">{status}</span>
        {hidden > 0 && <span>{hidden.toLocaleString('en-US')} more are hidden by the filters, not absent.</span>}
        {drifted && <span className="text-amber-200">The index changed while loading, so the list and the count differ.</span>}
        <button type="button" className="focus-ring ml-auto rounded border border-slate-700 px-2 py-0.5 font-semibold text-slate-300 hover:border-cyan-400/50" onClick={() => void copy()}>
          Copy CSV
        </button>
        {copied && <span role="status" className="w-full text-slate-300">{copied}</span>}
      </div>
      <div className={`grid ${COLUMNS} gap-2 border-b border-slate-800 px-2 py-1 text-[8px] font-semibold uppercase tracking-wider text-slate-400`} aria-hidden="true">
        <span>Neighbour</span><span>Kind</span><span>Grade</span><span>Relationship</span><span className="text-right">Shared</span>
      </div>
      <VirtualList
        items={edges}
        rowHeight={ROW_HEIGHT}
        ariaLabel={`Neighbours of ${centre.displayValue}, strongest first`}
        resetKey={centre.entityId}
        getKey={(edge) => edge.neighbor.entityId}
        className="h-[320px]"
        onNearEnd={() => {
          if (nextCursor !== null && !list.loadingMore && list.moreError === null && !list.guardStopped) onLoadMore();
        }}
        renderRow={(edge, { tabIndex }) => (
          <div className={`grid h-full ${COLUMNS} items-center gap-2 border-b border-slate-800/60 px-2 text-[10px]`}>
            <button
              type="button"
              tabIndex={tabIndex}
              onClick={() => onSelect(edge.neighbor.entityId)}
              title={edge.neighbor.displayValue}
              aria-label={`${edge.neighbor.displayValue}, ${kindLabel(edge.neighbor.kind)}. Select to centre the graph on it.`}
              className="focus-ring truncate rounded text-left font-semibold text-slate-100 hover:underline"
            >
              {edge.neighbor.displayValue}
            </button>
            <span className="truncate text-slate-400">{kindLabel(edge.neighbor.kind)}</span>
            <ValidationGradeChip grade={edge.neighbor.validationGrade} />
            <EdgeBasisChip kind={edge.kind} basis={edge.basis} />
            <span className="text-right font-mono text-slate-300">{edge.sharedDocumentCount.toLocaleString('en-US')}</span>
          </div>
        )}
        footer={
          <div className="flex flex-wrap items-center gap-2 px-2 py-2 text-[9px] text-slate-400">
            {list.moreError !== null && (
              <span role="alert" className="text-rose-300">The next page could not be read: {list.moreError}</span>
            )}
            {nextCursor !== null ? (
              <>
                <button type="button" disabled={list.loadingMore} className="focus-ring rounded border border-slate-700 px-2 py-0.5 font-semibold text-slate-200 hover:border-cyan-400/50 disabled:opacity-60" onClick={onLoadMore}>
                  {list.loadingMore ? 'Loading…' : list.moreError !== null ? 'Retry' : `Load ${NEIGHBOR_PAGE_SIZE} more`}
                </button>
                <button type="button" disabled={list.loadingMore} className="focus-ring rounded border border-slate-700 px-2 py-0.5 font-semibold text-slate-200 hover:border-cyan-400/50 disabled:opacity-60" onClick={onLoadAll}>
                  Load all
                </button>
              </>
            ) : (
              <span>End of the list.</span>
            )}
            {list.guardStopped && (
              <span role="status">
                Paused after {LOAD_ALL_PAGE_GUARD} pages to keep the app responsive; {loaded} loaded{total !== null ? ` of ${total.toLocaleString('en-US')}` : ''}. Load all continues from here.
              </span>
            )}
          </div>
        }
      />
    </section>
  );
}
