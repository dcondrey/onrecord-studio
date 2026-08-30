import { useEffect, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  CorpusEntityEdge,
  CorpusEntityKind,
  CorpusEntityNeighborhood,
  CorpusEntityNode,
} from '../../types';
import { cn } from '../../utils/cn';
import AnalyticsUnavailable from '../charts/AnalyticsUnavailable';

const VIEWPORT = 460;
const CENTRE = VIEWPORT / 2;
const INNER_RADIUS = 74;
const OUTER_RADIUS = 196;

const kindLabels: Record<CorpusEntityKind, string> = {
  STATUTE: 'Authority',
  DATE: 'Date',
  EMAIL_DOMAIN: 'Email domain',
  EMAIL_ADDRESS: 'Correspondent',
  URL_HOST: 'Host',
  ACRONYM: 'Acronym',
  ORGANIZATION: 'Organization',
  MONEY: 'Amount',
};

const kindColors: Record<CorpusEntityKind, string> = {
  STATUTE: 'var(--viz-amber)',
  DATE: 'var(--viz-cyan)',
  EMAIL_DOMAIN: 'var(--viz-indigo)',
  EMAIL_ADDRESS: 'var(--viz-indigo)',
  URL_HOST: 'var(--viz-purple)',
  ACRONYM: 'var(--viz-emerald)',
  ORGANIZATION: 'var(--viz-emerald)',
  MONEY: 'var(--viz-red)',
};

interface PlacedEdge {
  edge: CorpusEntityEdge;
  x: number;
  y: number;
  /** Which side of the node the label sits on, so it never runs off the viewport. */
  anchor: 'start' | 'end';
}

/**
 * Places neighbours around the centre, strongest pulled closest.
 *
 * Hand-rolled rather than force-simulated, and that is the honest layout for an ego network: every
 * arc here shares one endpoint, so a force run would spend its iterations resolving label collisions
 * and produce a picture whose distances mean nothing. Radius is a reading of the data instead --
 * the more documents a neighbour shares with the centre, the closer it sits -- and it is
 * deterministic, so the same corpus draws the same graph twice.
 */
function placeEdges(edges: CorpusEntityEdge[]): PlacedEdge[] {
  const strongest = edges.reduce((most, edge) => Math.max(most, edge.sharedDocumentCount), 1);
  const weakest = edges.reduce((least, edge) => Math.min(least, edge.sharedDocumentCount), strongest);
  const span = Math.max(1, strongest - weakest);
  return edges.map((edge, index) => {
    // Start at the top and walk clockwise. The list arrives ranked, so the strongest neighbour
    // takes the top slot and the ranking is legible without reading a single number.
    const angle = (index / Math.max(1, edges.length)) * Math.PI * 2 - Math.PI / 2;
    const closeness = (edge.sharedDocumentCount - weakest) / span;
    const radius = OUTER_RADIUS - closeness * (OUTER_RADIUS - INNER_RADIUS);
    const x = CENTRE + radius * Math.cos(angle);
    const y = CENTRE + radius * Math.sin(angle);
    return { edge, x, y, anchor: x < CENTRE ? 'end' : 'start' };
  });
}

function nodeRadius(node: CorpusEntityNode) {
  return Math.min(26, 8 + Math.sqrt(node.documentCount) * 2.4);
}

function truncateLabel(value: string) {
  return value.length > 28 ? `${value.slice(0, 27)}…` : value;
}

export default function EntityGraph() {
  const gate = useLatestRequestGate();
  const [seeds, setSeeds] = useState<CorpusEntityNode[]>([]);
  const [neighborhood, setNeighborhood] = useState<CorpusEntityNeighborhood | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [missing, setMissing] = useState(false);

  useEffect(() => {
    const token = gate.begin();
    invokeCommand('list_top_corpus_entities', { limit: 24 })
      .then((entities) => {
        if (!gate.isCurrent(token)) return;
        setSeeds(entities);
        setSelectedId((current) => current ?? entities[0]?.entityId ?? null);
      })
      .catch(() => {
        if (!gate.isCurrent(token)) return;
        setFailed(true);
      });
  }, [gate]);

  useEffect(() => {
    if (!selectedId) return;
    // The token is captured for the entity this request was made under. A neighbourhood that
    // resolves after the investigator has clicked through to another node would otherwise be
    // published as that node's relationships, which is a false statement about the corpus.
    const token = gate.begin();
    invokeCommand('get_corpus_entity_neighborhood', { entityId: selectedId, limit: 18 })
      .then((result) => {
        if (!gate.isCurrent(token)) return;
        setMissing(result === null);
        setNeighborhood(result);
      })
      .catch(() => {
        if (!gate.isCurrent(token)) return;
        setFailed(true);
      });
  }, [selectedId, gate]);

  if (failed) {
    return (
      <section className="app-panel flex min-h-[420px] flex-col overflow-hidden p-5">
        <div className="app-eyebrow">Entity graph</div>
        <AnalyticsUnavailable
          className="mt-4 flex-1"
          heading="Entity graph unavailable"
          detail="The corpus index did not answer, so this panel has nothing to report either way. It is not a statement that the corpus holds no relationships."
        />
      </section>
    );
  }

  const placed = neighborhood ? placeEdges(neighborhood.edges) : [];
  const centre = neighborhood?.center ?? null;
  const edgeCount = neighborhood?.edges.length ?? 0;
  const edgesTruncated = neighborhood?.truncated ?? false;

  return (
    <section className="app-panel flex flex-col overflow-hidden p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="app-eyebrow">Entity graph</div>
          <div className="mt-1 text-[10px] text-slate-400">
            Every arc here is co-occurrence in one segment: a derivation this app performed and can
            re-derive, not a relationship the documents asserted. Distance reads shared documents.
          </div>
        </div>
        <div className="rounded border border-slate-800 bg-slate-900/60 px-2 py-1 font-mono text-[9px] text-slate-400">
          {seeds.length} entities indexed
        </div>
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5">
        {seeds.map((seed) => (
          <button
            key={seed.entityId}
            type="button"
            onClick={() => setSelectedId(seed.entityId)}
            className={cn(
              'rounded border px-2 py-1 font-mono text-[9px] transition-colors',
              seed.entityId === selectedId
                ? 'border-slate-500 bg-slate-800 text-slate-100'
                : 'border-slate-800 bg-slate-900/60 text-slate-400 hover:border-slate-600',
            )}
          >
            {truncateLabel(seed.displayValue)}
            <span className="ml-1.5 text-slate-400">{seed.documentCount}</span>
          </button>
        ))}
      </div>

      {seeds.length === 0 ? (
        <div className="mt-6 grid flex-1 place-items-center text-center">
          <div className="max-w-xs">
            <div className="text-[10px] font-semibold text-slate-300">No entities indexed yet</div>
            <div className="mt-1 text-[9px] leading-relaxed text-slate-400">
              The graph draws entities the corpus index extracted from ingested documents. Nothing
              has been indexed, so there is nothing here to read.
            </div>
          </div>
        </div>
      ) : missing || !centre ? (
        <div className="mt-6 grid flex-1 place-items-center text-center">
          <div className="max-w-xs text-[9px] leading-relaxed text-slate-400">
            That entity is no longer in the index. A corpus rebuild discards every entity
            identifier, so this is a stale selection rather than an isolated node.
          </div>
        </div>
      ) : (
        <>
          <svg
            viewBox={`0 0 ${VIEWPORT} ${VIEWPORT}`}
            className="mt-3 h-[430px] w-full"
            role="img"
            aria-label={`Entities co-occurring with ${centre.displayValue}`}
          >
            {placed.map(({ edge, x, y }) => (
              <line
                key={`edge-${edge.neighbor.entityId}`}
                x1={CENTRE}
                y1={CENTRE}
                x2={x}
                y2={y}
                stroke="var(--chart-grid)"
                strokeWidth={Math.min(3.5, 0.8 + edge.sharedDocumentCount * 0.35)}
                // Dashed is not decoration. A computed edge and an observed one must never read
                // the same, and every edge the index writes today is computed.
                strokeDasharray={edge.basis === 'COMPUTED' ? '4 4' : undefined}
                strokeOpacity={0.55}
              />
            ))}
            {placed.map(({ edge, x, y, anchor }) => (
              <g
                key={`node-${edge.neighbor.entityId}`}
                onClick={() => setSelectedId(edge.neighbor.entityId)}
                className="cursor-pointer"
              >
                <circle
                  cx={x}
                  cy={y}
                  r={nodeRadius(edge.neighbor)}
                  fill={kindColors[edge.neighbor.kind]}
                  fillOpacity={0.22}
                  stroke={kindColors[edge.neighbor.kind]}
                  // An unverified node is drawn as one. `PATTERN_ONLY` is a lead, and the reader
                  // must not present a lead the way it presents a parsed value.
                  strokeDasharray={edge.neighbor.validationGrade === 'PATTERN_ONLY' ? '3 3' : undefined}
                  strokeWidth={1.4}
                />
                <text
                  x={anchor === 'end' ? x - nodeRadius(edge.neighbor) - 5 : x + nodeRadius(edge.neighbor) + 5}
                  y={y + 3}
                  textAnchor={anchor}
                  className="fill-slate-300 text-[9px]"
                >
                  {truncateLabel(edge.neighbor.displayValue)}
                </text>
              </g>
            ))}
            <circle
              cx={CENTRE}
              cy={CENTRE}
              r={nodeRadius(centre) + 4}
              fill="var(--chart-center)"
              stroke={kindColors[centre.kind]}
              strokeWidth={2}
            />
            <text x={CENTRE} y={CENTRE + 4} textAnchor="middle" className="fill-slate-100 text-[10px] font-semibold">
              {truncateLabel(centre.displayValue)}
            </text>
          </svg>

          <div className="mt-2 flex flex-wrap items-center gap-3 border-t border-slate-800 pt-2 text-[9px] text-slate-400">
            <span>
              {kindLabels[centre.kind]} · {centre.documentCount} documents ·{' '}
              {centre.validationGrade === 'PATTERN_ONLY'
                ? 'pattern only, a lead'
                : centre.validationGrade.toLowerCase().replace(/_/g, ' ')}
            </span>
            <span className="font-mono">
              {edgeCount} neighbours
              {edgesTruncated ? ' (list cut at the limit)' : ''}
            </span>
            <span className="ml-auto">Dashed arc: computed. Dashed outline: unverified.</span>
          </div>
        </>
      )}
    </section>
  );
}
