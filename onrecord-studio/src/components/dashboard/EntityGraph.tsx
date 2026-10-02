import { useEffect, useRef, useState } from 'react';
import { combineNeighbourhoods, lookalikeNeighbours } from '../../utils/entityEdges';
import { useResolutionClusters } from '../../hooks/useResolutionClusters';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useEntityNeighbors } from '../../hooks/useEntityNeighbors';
import EntityNeighborTable from './EntityNeighborTable';
import type {
  CorpusEntityEdge,
  CorpusEntityKind,
  CorpusEntityNeighborhood,
  CorpusEntityNode,
  EntityBroker,
  EntityNeighborFilter,
  EntityPath,
  RelationshipBasis,
  RelationshipKind,
} from '../../types';
import { cn } from '../../utils/cn';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import { useAffiliationDisclosure } from '../account/useProfile';
import AnalyticsUnavailable from '../charts/AnalyticsUnavailable';
import { ShapeGlyph } from '../ui/shapeGlyph';
import { RegistryAnchorNote } from '../ui/RegistryAnchorNote';
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

/**
 * Neighbours drawn at once. Readability, not a limit: every neighbour stays reachable in the table
 * below, and the kinds the drawing leaves out are named with their counts.
 */
const DRAWING_BUDGET = 18;
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
  EIN: 'Employer ID',
  CONTRACTOR_LICENSE: 'Contractor license',
  PARCEL_NUMBER: 'Parcel',
  SERIES_NUMBER: 'Series number',
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
  EIN: 'var(--viz-emerald)',
  CONTRACTOR_LICENSE: 'var(--viz-purple)',
  PARCEL_NUMBER: 'var(--viz-red)',
  SERIES_NUMBER: 'var(--viz-amber)',
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
const ALL_BASES: RelationshipBasis[] = ['OBSERVED', 'COMPUTED', 'ASSERTED'];
const BASIS_FILTER_LABELS: Record<RelationshipBasis, string> = {
  OBSERVED: 'Stated in text',
  COMPUTED: 'Derived by this app',
  ASSERTED: 'Asserted by a source',
};

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

interface PathRequest {
  fromEntityId: string;
  toEntityId: string;
}

/** Same discipline for a traced path: the origin is the centre at submit time, and the result is only a statement about that pair. */
type PathAnswer =
  | { request: PathRequest; kind: 'pending' }
  | { request: PathRequest; kind: 'found'; path: EntityPath }
  | { request: PathRequest; kind: 'notConnected'; maxHops: number }
  | { request: PathRequest; kind: 'unknownEntity'; entityId: string }
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
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [pathTargetId, setPathTargetId] = useState('');
  const [shownBases, setShownBases] = useState<ReadonlySet<RelationshipBasis>>(() => new Set(ALL_BASES));
  const [kindFilter, setKindFilter] = useState<CorpusEntityKind | null>(null);
  const [asOf, setAsOf] = useState('');
  const [includeUndated, setIncludeUndated] = useState(true);
  const [pathAnswer, setPathAnswer] = useState<PathAnswer | null>(null);
  // Entities the reader chose to treat as the same as the centre, for this session only. Keyed by
  // the centre so a result that lands after the reader moved on can never merge into another entity.
  const [combined, setCombined] = useState<{ centreId: string; aliases: Record<string, CorpusEntityNeighborhood | 'loading' | 'failed'> } | null>(null);
  const brokerGate = useLatestRequestGate();
  const [brokers, setBrokers] = useState<EntityBroker[] | null>(null);
  const [brokersFailed, setBrokersFailed] = useState(false);

  useEffect(() => {
    const token = brokerGate.begin();
    invokeCommand('list_entity_brokers', { limit: 12 })
      .then((rows) => {
        if (brokerGate.isCurrent(token)) setBrokers(rows);
      })
      .catch(() => {
        if (brokerGate.isCurrent(token)) setBrokersFailed(true);
      });
  }, [brokerGate]);

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

  // The filter is evaluated by the index over every neighbour, so a filtered drawing and table are
  // complete for the filter rather than limited to whatever was loaded.
  const filter: EntityNeighborFilter = {
    bases: ALL_BASES.filter((basis) => shownBases.has(basis)),
    asOf: asOf || null,
    includeUndated,
    kind: kindFilter,
  };
  const filterKey = JSON.stringify(filter);
  const { list, centre: knownCentre, loadMore, loadAll } = useEntityNeighbors(selectedId, filter);
  const baseNeighborhood: CorpusEntityNeighborhood | null =
    list.status === 'ready' ? { center: list.centre, edges: list.edges.slice(0, DRAWING_BUDGET), truncated: false } : null;
  const aliasState = combined !== null && combined.centreId === selectedId ? combined.aliases : {};
  const loadedAliases = Object.values(aliasState).filter((value): value is CorpusEntityNeighborhood => typeof value === 'object' && value !== null);
  const combinedView = baseNeighborhood && loadedAliases.length > 0 ? combineNeighbourhoods(baseNeighborhood, loadedAliases) : baseNeighborhood;
  const neighborhood = combinedView ? { ...combinedView, edges: combinedView.edges.slice(0, DRAWING_BUDGET) } : null;
  const resolution = useResolutionClusters(selectedId);
  const reviewed = resolution.status === 'ready' && resolution.entityId === selectedId ? resolution.cluster : null;
  const reviewedIds = reviewed ? reviewed.members.filter((member) => member.entityId !== selectedId).map((member) => member.entityId) : [];
  const lookalikes = list.status === 'ready'
    ? lookalikeNeighbours(list.centre, list.edges).filter((alias) => !reviewedIds.includes(alias.entityId))
    : [];
  const combine = (alias: CorpusEntityNode) => {
    const centreId = selectedId;
    if (!centreId) return;
    setCombined((previous) => ({
      centreId,
      aliases: { ...(previous?.centreId === centreId ? previous.aliases : {}), [alias.entityId]: 'loading' },
    }));
    invokeCommand('page_corpus_entity_neighbors', { entityId: alias.entityId, pageSize: DRAWING_BUDGET, filter })
      .then((page) => settleAlias(centreId, alias.entityId, page ? { center: page.center, edges: page.edges, truncated: false } : 'failed'))
      .catch(() => settleAlias(centreId, alias.entityId, 'failed'));
  };
  const settleAlias = (centreId: string, aliasId: string, value: CorpusEntityNeighborhood | 'failed') =>
    setCombined((previous) =>
      previous && previous.centreId === centreId && aliasId in previous.aliases
        ? { ...previous, aliases: { ...previous.aliases, [aliasId]: value } }
        : previous,
    );
  const split = (aliasId: string) =>
    setCombined((previous) => {
      if (!previous) return previous;
      return { ...previous, aliases: Object.fromEntries(Object.entries(previous.aliases).filter(([id]) => id !== aliasId)) };
    });
  const missing = list.status === 'missing';
  const pathView = pathAnswer !== null && pathAnswer.request.fromEntityId === selectedId ? pathAnswer : null;

  // On a narrow screen the drawing scrolls sideways; start with the centre entity in view.
  const centredOn = knownCentre?.entityId ?? null;
  useEffect(() => {
    const el = drawingRef.current;
    if (el) el.scrollLeft = (el.scrollWidth - el.clientWidth) / 2;
  }, [centredOn]);

  // A changed filter describes different relationships, so combined spellings do not carry over.
  useEffect(() => {
    setCombined(null);
  }, [filterKey]);

  // Entities a reviewer judged the same are combined without a click. This runs after the reset
  // above so a filter change re-reads them, and only starts members that have no state yet, so a
  // failed read is shown and retried by the reader rather than looping.
  const reviewedKey = reviewed ? `${reviewed.resolutionId}:${reviewedIds.join(',')}` : '';
  useEffect(() => {
    if (!reviewed || list.status !== 'ready') return;
    for (const entityId of reviewedIds) {
      if (!(entityId in aliasState)) combine({ entityId } as CorpusEntityNode);
    }
  }, [reviewedKey, combined, list.status]);

  if (failed || list.status === 'failed') {
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

  const placed = placeEdges(neighborhood?.edges ?? []);
  const centre = knownCentre;
  const listing = list.status === 'ready' ? list : null;
  const counts = listing?.counts ?? null;
  const hiddenByFilters = counts !== null && counts.total !== counts.unfilteredTotal;
  const drawnOfKind = (kind: CorpusEntityKind) => placed.filter(({ edge }) => edge.neighbor.kind === kind).length;
  const undrawnGroups = (counts?.byKind ?? [])
    .map(({ kind, count }) => ({ kind, undrawn: Math.max(0, count - drawnOfKind(kind)) }))
    .filter(({ undrawn }) => undrawn > 0);

  return (
    <section className="app-panel flex flex-col overflow-hidden p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <div className="app-eyebrow">Entity graph</div>
          <div className="mt-1 text-label text-muted">
            Dotted arcs with a hollow marker are co-occurrence: proximity this app derived, not a
            relationship the documents stated. Labelled arcs with a solid diamond are relations read
            from the text. Distance reads shared documents.
          </div>
        </div>
        <div className="rounded border border-line bg-surface/70 px-2 py-1 font-mono text-caption text-muted">
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
              'focus-ring rounded border px-2 py-1 font-mono text-caption transition-colors',
              isOwn(seed.entityId) && 'ring-1 ring-accent/50',
              dimOthers && !isOwn(seed.entityId) && 'opacity-40',
              seed.entityId === selectedId
                ? 'border-control bg-raised text-fg'
                : 'border-control bg-surface/70 text-muted hover:border-control',
            )}
          >
            {truncateLabel(seed.displayValue)}
            <span className="ml-1.5 text-muted">{seed.documentCount}</span>
          </button>
        ))}
      </div>

      <details className="mt-2 text-caption text-muted">
        <summary className="focus-ring font-semibold text-fg-secondary">Entities that connect the most others</summary>
        {brokersFailed ? (
          <p role="alert" className="mt-1 text-danger">The ranking could not be read. This does not mean no entity bridges others.</p>
        ) : brokers === null ? (
          <p role="status" className="mt-1">Reading the ranking…</p>
        ) : brokers.length === 0 ? (
          <p className="mt-1">No entity has enough connections to rank yet.</p>
        ) : (
          <>
            <p className="mt-1">Ranked by number of connections weighted by shared documents. This is an approximation, not a measure of influence, and a well-connected entity is a lead to check, not a finding of anything improper.</p>
            <ol aria-label="Most connecting entities" className="mt-1 flex flex-col gap-1">
              {brokers.map((broker) => (
                <li key={broker.entity.entityId}>
                  <button type="button" onClick={() => setSelectedId(broker.entity.entityId)} className="focus-ring rounded border border-control px-2 py-1 text-left text-fg-secondary hover:border-control">
                    {truncateLabel(broker.entity.displayValue)} <span className="text-muted">· {broker.distinctNeighbors} connections · score {broker.brokerageScore.toLocaleString()}</span>
                  </button>
                </li>
              ))}
            </ol>
          </>
        )}
      </details>

      {seeds.length === 0 ? (
        <div className="mt-6 grid flex-1 place-items-center text-center">
          <div className="max-w-xs">
            <div className="text-label font-semibold text-fg-secondary">No entities indexed yet</div>
            <div className="mt-1 text-caption leading-relaxed text-muted">
              The graph draws entities the corpus index extracted from ingested documents. Nothing
              has been indexed, so there is nothing here to read.
            </div>
          </div>
        </div>
      ) : missing ? (
        <div className="mt-6 grid flex-1 place-items-center text-center">
          <div className="max-w-xs text-caption leading-relaxed text-muted">
            That entity is no longer in the index. A corpus rebuild discards every entity
            identifier, so this is a stale selection rather than an isolated node.
          </div>
        </div>
      ) : !centre ? (
        <div className="mt-6 grid flex-1 place-items-center text-center" aria-busy="true">
          <div className="max-w-xs text-caption leading-relaxed text-muted">
            Reading the neighbourhood from the corpus index. Nothing is known about this entity's
            relationships until it answers.
          </div>
        </div>
      ) : (
        <>
          {reviewed && reviewedIds.length > 0 && (
            <div role="group" aria-label="Entities combined by review" className="mt-3 rounded border border-success/30 p-2 text-caption text-fg-secondary">
              <p className="font-semibold text-success">
                Combined by your review: {reviewed.members.map((member) => member.displayValue).join(' · ')}
              </p>
              <ul className="mt-1 space-y-0.5 text-muted">
                {reviewed.reasons.map((reason) => (
                  <li key={reason.entryId}>
                    {reason.author} judged "{reason.leftDisplayValue}" and "{reason.rightDisplayValue}" the same on {reason.decidedAt.slice(0, 10)}{reason.reason ? `: ${reason.reason}` : '.'}
                  </li>
                ))}
              </ul>
              <p className="mt-1 text-muted">
                Resolution <code>{reviewed.resolutionId.slice(0, 12)}</code>. Change a judgment in Resolution review; it is recorded there, not undone here. Shared-document counts keep the larger of the combined entities.
              </p>
              {reviewed.contradictions.length > 0 && (
                <p role="alert" className="mt-1 text-warning">Some judgments contradict each other, so those entities are not combined until the conflict is resolved in Resolution review.</p>
              )}
            </div>
          )}
          {lookalikes.length > 0 && (
            <div role="group" aria-label="Possible duplicate entities" className="mt-3 rounded border border-line p-2 text-caption text-muted">
              <p>
                These look like other spellings of {truncateLabel(centre.displayValue)}. A similar name is a lead, not proof they are the same. Combine them to see their relationships together; this changes only this view.
              </p>
              <ul className="mt-1 flex flex-col gap-1">
                {lookalikes.map((alias) => {
                  const state = aliasState[alias.entityId];
                  return (
                    <li key={alias.entityId} className="flex flex-wrap items-center gap-2">
                      <span className="text-fg-secondary">{alias.displayValue}</span>
                      {state === undefined && (
                        <button type="button" className="focus-ring rounded border border-control px-2 py-0.5 font-semibold text-fg-secondary hover:border-accent/60" onClick={() => combine(alias)}>Combine in this view</button>
                      )}
                      {state === 'loading' && <span role="status">Reading its relationships…</span>}
                      {state === 'failed' && (
                        <>
                          <span role="alert" className="text-danger">Its relationships could not be read, so nothing was combined.</span>
                          <button type="button" className="focus-ring underline" onClick={() => combine(alias)}>Retry</button>
                        </>
                      )}
                      {typeof state === 'object' && (
                        <>
                          <span role="status" className="text-success">Combined for this session. Shared-document counts keep the larger of the two, not their sum.</span>
                          <button type="button" className="focus-ring underline" onClick={() => split(alias.entityId)}>Split</button>
                        </>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>
          )}
          <div className="mt-3 flex flex-wrap items-center gap-2 text-caption text-muted">
            <div role="group" aria-label="Show relationships by how they are known" className="flex gap-1">
              {ALL_BASES.map((basis) => (
                <button
                  key={basis}
                  type="button"
                  aria-pressed={shownBases.has(basis)}
                  onClick={() =>
                    setShownBases((current) => {
                      const next = new Set(current);
                      if (next.has(basis)) next.delete(basis);
                      else next.add(basis);
                      return next;
                    })
                  }
                  className={cn('focus-ring rounded border px-2 py-1 font-semibold', shownBases.has(basis) ? 'border-accent/60 text-accent' : 'border-control text-muted')}
                >
                  {BASIS_FILTER_LABELS[basis]}
                </button>
              ))}
            </div>
            <label className="flex items-center gap-1">
              As of
              <input type="date" value={asOf} onChange={(event) => setAsOf(event.target.value)} className="focus-ring rounded border border-control bg-canvas px-1 py-0.5 text-fg" />
            </label>
            {asOf && (
              <>
                <label className="flex items-center gap-1">
                  <input type="checkbox" checked={includeUndated} onChange={(event) => setIncludeUndated(event.target.checked)} />
                  Include undated
                </label>
                <button type="button" className="focus-ring underline" onClick={() => setAsOf('')}>Clear date</button>
              </>
            )}
            <label className="flex items-center gap-1">
              Kind
              <select
                value={kindFilter ?? ''}
                onChange={(event) => setKindFilter((event.target.value || null) as CorpusEntityKind | null)}
                className="focus-ring rounded border border-control bg-canvas px-1 py-0.5 text-fg"
              >
                <option value="">All kinds</option>
                {(Object.keys(kindLabels) as CorpusEntityKind[]).map((kind) => (
                  <option key={kind} value={kind}>{kindLabels[kind]}</option>
                ))}
              </select>
            </label>
            {hiddenByFilters && counts && (
              <span role="status">
                The filters match {counts.total.toLocaleString('en-US')} of {counts.unfilteredTotal.toLocaleString('en-US')} neighbours. Hidden relationships are filtered out, not absent.
                {asOf && !includeUndated ? ' Relationships with no recorded dates are hidden.' : ''}
              </span>
            )}
          </div>
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
                  <g className="text-fg">
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
                    <text x={mx + 6} y={my - 4} className="fill-fg text-caption font-semibold" {...HALO}>
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
                  'focus:outline-none focus-visible:[&>circle:first-of-type]:stroke-white',
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
                <g className="text-fg">
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
                  className="fill-fg-secondary text-caption"
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
            <text x={CENTRE} y={CENTRE + 4} textAnchor="middle" className="fill-fg text-label font-semibold" {...HALO}>
              {truncateLabel(centre.displayValue)}
            </text>
          </svg>
          </div>
          <div className="mt-2 flex flex-wrap items-center gap-2 text-caption text-muted" aria-busy={list.status === 'loading'}>
            {list.status === 'loading' ? (
              <span role="status">Updating the neighbours for this filter…</span>
            ) : (
              <span role="status">
                {counts
                  ? `Drawing the ${placed.length} strongest of ${counts.total.toLocaleString('en-US')} neighbours. All of them are listed in the table below.`
                  : `Drawing the ${placed.length} strongest neighbours. The total is not available yet; the table below lists every neighbour.`}
              </span>
            )}
            {kindFilter === null &&
              undrawnGroups.map(({ kind, undrawn }) => (
                <button
                  key={kind}
                  type="button"
                  className="focus-ring rounded border border-control px-2 py-0.5 font-semibold text-fg-secondary hover:border-accent/60"
                  onClick={() => setKindFilter(kind)}
                >
                  {undrawn.toLocaleString('en-US')} more of {kindLabels[kind]}
                </button>
              ))}
            {kindFilter !== null && (
              <button type="button" className="focus-ring rounded border border-control px-2 py-0.5 font-semibold text-fg-secondary hover:border-accent/60" onClick={() => setKindFilter(null)}>
                Showing {kindLabels[kindFilter]} only. Show all kinds
              </button>
            )}
          </div>

          <ul className="mt-2 flex flex-col gap-1.5" aria-label="Relationships and their evidence basis">
            {placed.map(({ edge }) => (
              <li key={`basis-${edge.neighbor.entityId}`} className="rounded-lg border border-line p-2">
                <div className="flex flex-wrap items-center gap-2 text-label">
                  <span className="font-semibold text-fg">{edge.neighbor.displayValue}</span>
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

          {listing && (
            <>
              <EntityNeighborTable
                centre={centre}
                list={listing}
                kindLabel={(kind) => kindLabels[kind]}
                onSelect={setSelectedId}
                onLoadMore={loadMore}
                onLoadAll={loadAll}
              />
              {loadedAliases.length > 0 && (
                <p className="mt-1 text-caption text-muted">
                  The table lists {centre.displayValue}'s own neighbours. Combined spellings add their strongest neighbours to the drawing only.
                </p>
              )}
            </>
          )}

          <div className="mt-2 flex flex-wrap items-center gap-3 border-t border-line pt-2 text-caption text-muted">
            <span>
              {kindLabels[centre.kind]} · {centre.documentCount} documents
            </span>
            <ValidationGradeChip grade={centre.validationGrade} />
            <span className="font-mono">
              {counts
                ? `${counts.total.toLocaleString('en-US')} neighbours`
                : listing?.countFailed
                  ? 'Neighbour total unavailable'
                  : 'Counting neighbours'}
            </span>
            <span className="ml-auto">
              Node badge shape carries validation grade; hollow badge with dashed outline is a lead.
            </span>
          </div>
          {centre.kind === 'CONTRACTOR_LICENSE' && (
            <div className="mt-2 border-t border-line pt-2">
              <RegistryAnchorNote identifierKind={centre.kind} value={centre.displayValue} />
            </div>
          )}

          <form
            className="mt-2 flex flex-wrap items-center gap-2 border-t border-line pt-2 text-caption"
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
                  setPathAnswer(
                    result.outcome === 'found'
                      ? { request, kind: 'found', path: result.path }
                      : result.outcome === 'notConnected'
                        ? { request, kind: 'notConnected', maxHops: result.maxHops }
                        : { request, kind: 'unknownEntity', entityId: result.entityId },
                  );
                })
                .catch((reason: unknown) => {
                  if (!pathGate.isCurrent(token)) return;
                  setPathAnswer({ request, kind: 'failed', message: formatNativeError(reason) });
                });
            }}
          >
            <label className="font-mono text-muted" htmlFor="entity-path-target">
              Find path to entity id:
            </label>
            <input
              id="entity-path-target"
              type="text"
              value={pathTargetId}
              onChange={(event) => setPathTargetId(event.target.value)}
              className="focus-ring min-w-[220px] flex-1 rounded border border-control bg-surface/70 px-2 py-1 font-mono text-caption text-fg"
              placeholder="paste the neighbour id from another view"
            />
            <button
              type="submit"
              className="focus-ring rounded border border-control bg-raised/70 px-2 py-1 font-mono text-caption text-fg hover:border-control"
            >
              Trace
            </button>
            {pathView?.kind === 'pending' && (
              <span className="text-muted" aria-busy="true">
                Tracing from {centre.displayValue} to {pathView.request.toEntityId}.
              </span>
            )}
            {pathView?.kind === 'failed' && <span className="text-danger">{pathView.message}</span>}
            {pathView?.kind === 'notConnected' && (
              <span className="text-muted">
                No path within {pathView.maxHops} hops from {centre.displayValue} to {pathView.request.toEntityId}:
                the two entities are not connected in this corpus.
              </span>
            )}
            {pathView?.kind === 'unknownEntity' && (
              <span className="text-warning">
                {pathView.entityId} is not in the entity index, so no path from {centre.displayValue} could be
                traced.
              </span>
            )}
            {pathView?.kind === 'found' && (
              <span className="w-full font-mono text-fg-secondary">
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
