import { useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type {
  CorpusEntityEdge,
  CorpusEntityKind,
  CorpusEntityNeighborhood,
  CorpusEntityNode,
  EntityPath,
  RelationshipKind,
} from '../../types';
import { cn } from '../../utils/cn';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import { useAffiliationDisclosure } from '../account/useProfile';
import AnalyticsUnavailable from '../charts/AnalyticsUnavailable';
import { ShapeGlyph } from '../ui/shapeGlyph';
import {
  ConfidenceMeter,
  EdgeBasisChip,
  EvidenceTrace,
  ValidationGradeChip,
  edgeExplanation,
  edgeVisual,
  gradeGlyphShape,
  gradeLabel,
} from '../ui/EvidenceBasis';

const VIEWPORT = 460;
const CENTRE = VIEWPORT / 2;
const INNER_RADIUS = 74;
const OUTER_RADIUS = 196;
/** Side margin inside the viewBox so a label beside an outer node is never cut at the edge. */
const LABEL_ROOM = 120;
/** Relation markers sit past the centre label instead of on top of it. */
const LABEL_POSITION = 0.78;
/** A backing outline keeps a label legible where it crosses an arc or another label. */
const HALO = { stroke: 'var(--chart-center)', strokeWidth: 3, paintOrder: 'stroke', strokeLinejoin: 'round' } as const;

const kindLabels: Record<CorpusEntityKind, string> = {
  STATUTE: 'Authority',
  DATE: 'Date',
  EMAIL_DOMAIN: 'Email domain',
  EMAIL_ADDRESS: 'Correspondent',
  URL_HOST: 'Host',
  ACRONYM: 'Acronym',
  ORGANIZATION: 'Organization',
  MONEY: 'Amount',
  PERSON: 'Person',
  ROLE: 'Role',
  POSITION: 'Position',
  DOCUMENT_CLASS: 'Document class',
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
  PERSON: 'var(--viz-cyan)',
  ROLE: 'var(--viz-indigo)',
  POSITION: 'var(--viz-purple)',
  DOCUMENT_CLASS: 'var(--viz-amber)',
};

const kindDescriptions: Record<RelationshipKind, string> = {
  CO_OCCURRENCE: 'co-occurrence',
  SIGNED_BY: 'signed by',
  APPROVED: 'approved',
  CITES: 'cites',
  PROMISES: 'promises',
  DELIVERED: 'delivered',
  SUPERSEDES: 'supersedes',
  AMENDS: 'amends',
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

/**
 * A neighbourhood answer records the entity it was requested for. The render derives everything
 * from `answer.entityId === selectedId`, so a selection change with no request yet issued, or one
 * whose request is still in flight, reads as pending rather than as the previous entity's verdict.
 * `neighborhood: null` is the index saying the entity is gone; a pending request says nothing.
 */
interface NeighborhoodAnswer {
  entityId: string;
  neighborhood: CorpusEntityNeighborhood | null;
}

interface PathRequest {
  fromEntityId: string;
  toEntityId: string;
}

/** Same discipline for a traced path: the origin is the centre at submit time, and the result is only a statement about that pair. */
type PathAnswer =
  | { request: PathRequest; kind: 'pending' }
  | { request: PathRequest; kind: 'found'; path: EntityPath }
  | { request: PathRequest; kind: 'none' }
  | { request: PathRequest; kind: 'failed'; message: string };

export default function EntityGraph() {
  const gate = useLatestRequestGate();
  const lensEnabled = useProfileLensStore((state) => state.lensEnabled);
  const { disclosure } = useAffiliationDisclosure();
  const isOwn = (entityId: string) => disclosure?.ownEntityIds.includes(entityId) ?? false;
  // Dimming applies only when the lens is on and at least one node is confirmed as yours.
  const dimOthers = lensEnabled && (disclosure?.ownEntityIds.length ?? 0) > 0;
  const pathGate = useLatestRequestGate();
  const drawingRef = useRef<HTMLDivElement>(null);
  const [seeds, setSeeds] = useState<CorpusEntityNode[]>([]);
  const [answer, setAnswer] = useState<NeighborhoodAnswer | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [pathTargetId, setPathTargetId] = useState('');
  const [pathAnswer, setPathAnswer] = useState<PathAnswer | null>(null);

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
    const entityId = selectedId;
    invokeCommand('get_corpus_entity_neighborhood', { entityId, limit: 18 })
      .then((result) => {
        if (!gate.isCurrent(token)) return;
        setAnswer({ entityId, neighborhood: result });
      })
      .catch(() => {
        if (!gate.isCurrent(token)) return;
        setFailed(true);
      });
  }, [selectedId, gate]);

  // Only an answer for the entity now selected is a statement about it. Anything else is pending.
  const current = answer !== null && answer.entityId === selectedId ? answer : null;
  const neighborhood = current?.neighborhood ?? null;
  const missing = current !== null && current.neighborhood === null;
  const pathView = pathAnswer !== null && pathAnswer.request.fromEntityId === selectedId ? pathAnswer : null;

  // On a narrow screen the drawing scrolls sideways; start with the centre entity in view.
  const centredOn = neighborhood?.center.entityId ?? null;
  useEffect(() => {
    const el = drawingRef.current;
    if (el) el.scrollLeft = (el.scrollWidth - el.clientWidth) / 2;
  }, [centredOn]);

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
            Dotted arcs with a hollow marker are co-occurrence: proximity this app derived, not a
            relationship the documents stated. Labelled arcs with a solid diamond are relations read
            from the text. Distance reads shared documents.
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
            data-own-organization={isOwn(seed.entityId) ? 'true' : undefined}
            className={cn(
              'focus-ring rounded border px-2 py-1 font-mono text-[9px] transition-colors',
              isOwn(seed.entityId) && 'ring-1 ring-cyan-400/60',
              dimOthers && !isOwn(seed.entityId) && 'opacity-40',
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
      ) : missing ? (
        <div className="mt-6 grid flex-1 place-items-center text-center">
          <div className="max-w-xs text-[9px] leading-relaxed text-slate-400">
            That entity is no longer in the index. A corpus rebuild discards every entity
            identifier, so this is a stale selection rather than an isolated node.
          </div>
        </div>
      ) : !centre ? (
        <div className="mt-6 grid flex-1 place-items-center text-center" aria-busy="true">
          <div className="max-w-xs text-[9px] leading-relaxed text-slate-400">
            Reading the neighbourhood from the corpus index. Nothing is known about this entity's
            relationships until it answers.
          </div>
        </div>
      ) : (
        <>
          <div
            ref={drawingRef}
            className="focus-ring mt-3 overflow-x-auto"
            tabIndex={0}
            role="region"
            aria-label="Entity graph drawing; scrolls sideways on narrow screens"
          >
          <svg
            viewBox={`${-LABEL_ROOM} 0 ${VIEWPORT + LABEL_ROOM * 2} ${VIEWPORT}`}
            className="h-[430px] w-full min-w-[640px]"
            role="img"
            aria-label={`Entities co-occurring with ${centre.displayValue}`}
          >
            {placed.map(({ edge, x, y }) => {
              const visual = edgeVisual(edge.kind, edge.basis);
              const mx = CENTRE + (x - CENTRE) * LABEL_POSITION;
              const my = CENTRE + (y - CENTRE) * LABEL_POSITION;
              return (
                <g key={`edge-${edge.neighbor.entityId}`} data-edge-family={visual.family}>
                  <line
                    x1={CENTRE}
                    y1={CENTRE}
                    x2={x}
                    y2={y}
                    stroke="var(--chart-grid)"
                    strokeWidth={Math.min(3.5, 0.8 + edge.sharedDocumentCount * 0.35)}
                    strokeDasharray={visual.dash}
                    strokeOpacity={visual.opacity}
                  />
                  <g className="text-slate-200">
                    <ShapeGlyph
                      shape={visual.markerShape}
                      cx={mx}
                      cy={my}
                      radius={3.6}
                      fill={visual.family === 'typed' ? 'currentColor' : 'var(--chart-center)'}
                      stroke="currentColor"
                      strokeWidth={1.2}
                    />
                  </g>
                  {visual.family === 'typed' && (
                    <text x={mx + 6} y={my - 4} className="fill-slate-200 text-[8px] font-semibold" {...HALO}>
                      {visual.label}
                    </text>
                  )}
                </g>
              );
            })}
            {placed.map(({ edge, x, y, anchor }) => (
              <g
                key={`node-${edge.neighbor.entityId}`}
                onClick={() => setSelectedId(edge.neighbor.entityId)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    setSelectedId(edge.neighbor.entityId);
                  }
                }}
                tabIndex={0}
                role="button"
                aria-label={`${edge.neighbor.displayValue}: ${kindLabels[edge.neighbor.kind]}, ${gradeLabel(edge.neighbor.validationGrade)}, ${edgeVisual(edge.kind, edge.basis).label}. Select to centre the graph on it.`}
                className={cn(
                  'cursor-pointer focus:outline-none focus-visible:[&>circle:first-of-type]:stroke-white',
                  dimOthers && !isOwn(edge.neighbor.entityId) && 'opacity-40',
                )}
                data-own-organization={isOwn(edge.neighbor.entityId) ? 'true' : undefined}
              >
                {isOwn(edge.neighbor.entityId) && (
                  <circle cx={x} cy={y} r={nodeRadius(edge.neighbor) + 4} fill="none" stroke="rgb(34 211 238)" strokeWidth={1.5} />
                )}
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
                <g className="text-slate-100">
                  <ShapeGlyph
                    shape={gradeGlyphShape(edge.neighbor.validationGrade)}
                    cx={x + nodeRadius(edge.neighbor) * 0.75}
                    cy={y - nodeRadius(edge.neighbor) * 0.75}
                    radius={4}
                    fill={edge.neighbor.validationGrade === 'PATTERN_ONLY' ? 'var(--chart-center)' : 'currentColor'}
                    stroke="currentColor"
                    strokeWidth={1.2}
                  />
                </g>
                <text
                  x={anchor === 'end' ? x - nodeRadius(edge.neighbor) - 5 : x + nodeRadius(edge.neighbor) + 5}
                  y={y + 3}
                  textAnchor={anchor}
                  className="fill-slate-300 text-[9px]"
                  {...HALO}
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
              strokeWidth={isOwn(centre.entityId) ? 4 : 2}
              data-own-organization={isOwn(centre.entityId) ? 'true' : undefined}
            />
            <text x={CENTRE} y={CENTRE + 4} textAnchor="middle" className="fill-slate-100 text-[10px] font-semibold" {...HALO}>
              {truncateLabel(centre.displayValue)}
            </text>
          </svg>
          </div>

          <ul className="mt-2 flex flex-col gap-1.5" aria-label="Relationships and their evidence basis">
            {placed.map(({ edge }) => (
              <li key={`basis-${edge.neighbor.entityId}`} className="rounded-lg border border-slate-800 p-2">
                <div className="flex flex-wrap items-center gap-2 text-[10px]">
                  <span className="font-semibold text-slate-100">{edge.neighbor.displayValue}</span>
                  <EdgeBasisChip kind={edge.kind} basis={edge.basis} />
                  <ValidationGradeChip grade={edge.neighbor.validationGrade} />
                  <ConfidenceMeter confidence={{ kind: 'MEASURED', basisPoints: edge.confidenceBasisPoints }} />
                </div>
                <div className="mt-1">
                  <EvidenceTrace
                    label={`${centre.displayValue} and ${edge.neighbor.displayValue}`}
                    explanation={edgeExplanation(edge.kind, edge.basis, edge.sharedDocumentCount)}
                    citations={[]}
                    emptyNote="The index stores document counts for this link, not source passages. Search the shared documents in the evidence view to read the text."
                  />
                </div>
              </li>
            ))}
          </ul>

          <div className="mt-2 flex flex-wrap items-center gap-3 border-t border-slate-800 pt-2 text-[9px] text-slate-400">
            <span>
              {kindLabels[centre.kind]} · {centre.documentCount} documents
            </span>
            <ValidationGradeChip grade={centre.validationGrade} />
            <span className="font-mono">
              {edgeCount} neighbours
              {edgesTruncated ? ' (list cut at the limit)' : ''}
            </span>
            <span className="ml-auto">
              Node badge shape carries validation grade; hollow badge with dashed outline is a lead.
            </span>
          </div>

          <form
            className="mt-2 flex flex-wrap items-center gap-2 border-t border-slate-800 pt-2 text-[9px]"
            onSubmit={(event) => {
              event.preventDefault();
              const toEntityId = pathTargetId.trim();
              if (!selectedId || !toEntityId) return;
              const request: PathRequest = { fromEntityId: selectedId, toEntityId };
              const token = pathGate.begin();
              setPathAnswer({ request, kind: 'pending' });
              invokeCommand('find_entity_paths', { ...request, maxHops: 4 })
                .then((result) => {
                  if (!pathGate.isCurrent(token)) return;
                  setPathAnswer(result ? { request, kind: 'found', path: result } : { request, kind: 'none' });
                })
                .catch((reason: unknown) => {
                  if (!pathGate.isCurrent(token)) return;
                  setPathAnswer({ request, kind: 'failed', message: formatNativeError(reason) });
                });
            }}
          >
            <label className="font-mono text-slate-400" htmlFor="entity-path-target">
              Find path to entity id:
            </label>
            <input
              id="entity-path-target"
              type="text"
              value={pathTargetId}
              onChange={(event) => setPathTargetId(event.target.value)}
              className="focus-ring min-w-[220px] flex-1 rounded border border-slate-800 bg-slate-900/60 px-2 py-1 font-mono text-[9px] text-slate-100"
              placeholder="paste the neighbour id from another view"
            />
            <button
              type="submit"
              className="focus-ring rounded border border-slate-700 bg-slate-800/80 px-2 py-1 font-mono text-[9px] text-slate-200 hover:border-slate-500"
            >
              Trace
            </button>
            {pathView?.kind === 'pending' && (
              <span className="text-slate-400" aria-busy="true">
                Tracing from {centre.displayValue} to {pathView.request.toEntityId}.
              </span>
            )}
            {pathView?.kind === 'failed' && <span className="text-rose-400">{pathView.message}</span>}
            {pathView?.kind === 'none' && (
              // The index answers `null` both when an id is not in it and when no path exists, so
              // this cannot be stated as "not connected" alone.
              <span className="text-slate-400">
                No path within 4 hops from {centre.displayValue} to {pathView.request.toEntityId}: either
                that id is not in the index, or the two entities are not connected in this corpus.
              </span>
            )}
            {pathView?.kind === 'found' && (
              <span className="w-full font-mono text-slate-300">
                {pathView.path.hops} hops{' '}
                {pathView.path.weakestConfidenceBasisPoints != null
                  ? `· weakest edge ${(pathView.path.weakestConfidenceBasisPoints / 100).toFixed(1)}%`
                  : ''}{' '}
                ·{' '}
                {pathView.path.steps
                  .map((step, index) =>
                    index === 0
                      ? step.neighbor.displayValue
                      : `→ [${kindDescriptions[step.edgeKind]}] ${step.neighbor.displayValue}`,
                  )
                  .join(' ')}
              </span>
            )}
          </form>
        </>
      )}
    </section>
  );
}
