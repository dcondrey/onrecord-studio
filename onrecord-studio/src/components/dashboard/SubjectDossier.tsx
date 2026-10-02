import { useCallback, useEffect, useMemo, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { ShapeGlyph, type GlyphShape } from '../ui/shapeGlyph';
import type { CorpusEntityNeighborhood, CorpusEntityNode } from '../../types/corpus';
import type { Finding, FindingSeverity } from '../../types';

/**
 * The stitched view for one subject. When the user names an entity or types a name into the
 * palette, this pulls every signal already computed for that subject and lays them out beside
 * each other — findings that mention the subject, the graph of who and what surrounds it, and
 * the raw entity metadata. Each section fetches independently so a slow neighborhood doesn't
 * block the finding list; empty sections say so factually rather than fabricating placeholders.
 *
 * Name resolution: `list_top_corpus_entities({ limit: 500 })` is filtered by display value on
 * the client. If it matches more than one entity we render a picker before any section fetches
 * — a "John Smith" that resolves to three distinct email addresses must not silently choose
 * one. If it matches nothing, we say so and stop. The 500-entity fetch is a real ceiling and
 * we surface it: an unlisted long-tail entity looks the same as a missing one, and the picker
 * says that.
 */
export default function SubjectDossier() {
  const target = useAppStore((state) => state.subjectDossierTarget);
  const clear = useAppStore((state) => state.clearSubjectDossier);
  const open = useAppStore((state) => state.openSubjectDossier);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);

  const [resolution, setResolution] = useState<Resolution>({ status: 'RESOLVING' });
  const [neighborTotal, setNeighborTotal] = useState<number | null>(null);
  const [neighborhood, setNeighborhood] = useState<SectionState<CorpusEntityNeighborhood | null>>({
    status: 'LOADING',
  });
  const [findings, setFindings] = useState<SectionState<Finding[]>>({ status: 'LOADING' });

  const resolveGate = useLatestRequestGate();
  const neighborhoodGate = useLatestRequestGate();
  const findingsGate = useLatestRequestGate();

  const resolveTarget = useCallback(() => {
    if (!target) return;
    if (target.kind === 'entity') {
      setResolution({ status: 'RESOLVED', entityId: target.value, label: null });
      return;
    }
    const request = resolveGate.begin();
    setResolution({ status: 'RESOLVING' });
    invokeCommand('list_top_corpus_entities', { limit: 500 })
      .then((entities) => {
        if (!resolveGate.isCurrent(request)) return;
        const query = target.value.trim().toLocaleLowerCase();
        if (!query) {
          setResolution({ status: 'NONE' });
          return;
        }
        const matches = entities.filter((entity) => matchesName(entity, query));
        if (matches.length === 0) setResolution({ status: 'NONE' });
        else if (matches.length === 1) {
          setResolution({
            status: 'RESOLVED',
            entityId: matches[0].entityId,
            label: matches[0].displayValue,
          });
        } else {
          setResolution({ status: 'AMBIGUOUS', matches });
        }
      })
      .catch((reason) => {
        if (!resolveGate.isCurrent(request)) return;
        setResolution({ status: 'ERROR', message: String(reason) });
      });
  }, [resolveGate, target]);

  useEffect(() => {
    resolveTarget();
    return resolveGate.invalidate;
  }, [resolveGate, resolveTarget]);

  const loadNeighborhood = useCallback(
    (entityId: string) => {
      const request = neighborhoodGate.begin();
      setNeighborhood({ status: 'LOADING' });
      setNeighborTotal(null);
      // The list above is a preview; the total is read separately so "of N" is never the preview's own size.
      invokeCommand('count_corpus_entity_neighbors', { entityId })
        .then((counts) => {
          if (neighborhoodGate.isCurrent(request) && counts) setNeighborTotal(counts.unfilteredTotal);
        })
        .catch(() => {});
      invokeCommand('get_corpus_entity_neighborhood', { entityId, limit: 24 })
        .then((value) => {
          if (!neighborhoodGate.isCurrent(request)) return;
          setNeighborhood({ status: 'READY', value });
          // Direct entity-id navigation (EntityGraph, a finding row) never resolves a label from
          // the name search, so `resolution.label` starts null; backfill it from the neighborhood
          // center once it's confirmed to be this same entity, so loadFindings (keyed on label)
          // gets a real query instead of the empty string it treats as "no query".
          if (value) {
            setResolution((prev) =>
              prev.status === 'RESOLVED' && prev.label === null && prev.entityId === value.center.entityId
                ? { ...prev, label: value.center.displayValue }
                : prev,
            );
          }
        })
        .catch((reason) => {
          if (!neighborhoodGate.isCurrent(request)) return;
          setNeighborhood({ status: 'ERROR', message: String(reason) });
        });
    },
    [neighborhoodGate],
  );

  const loadFindings = useCallback(
    (label: string | null) => {
      const request = findingsGate.begin();
      setFindings({ status: 'LOADING' });
      invokeCommand('list_findings', { filter: null, limit: 200 })
        .then((rows) => {
          if (!findingsGate.isCurrent(request)) return;
          // The Wave 1 filter has no entityId axis, so we surface findings that name the
          // subject textually in an explanation or citation quote. This is looser than a
          // schema-backed join — a namesake match is possible. Label accordingly on screen.
          const query = (label ?? '').trim().toLocaleLowerCase();
          const matches = query.length > 0
            ? rows.filter((row) => mentionsSubject(row, query))
            : [];
          setFindings({ status: 'READY', value: matches });
        })
        .catch((reason) => {
          if (!findingsGate.isCurrent(request)) return;
          setFindings({ status: 'ERROR', message: String(reason) });
        });
    },
    [findingsGate],
  );

  const resolvedEntityId = resolution.status === 'RESOLVED' ? resolution.entityId : null;
  useEffect(() => {
    // Keyed on the entity id alone, not the whole `resolution` object: backfilling `label` below
    // produces a new `resolution` object for the same entity, and re-running this effect on that
    // would invalidate the in-flight request and refire it for no reason.
    if (resolvedEntityId === null) return;
    loadNeighborhood(resolvedEntityId);
    return neighborhoodGate.invalidate;
  }, [loadNeighborhood, neighborhoodGate, resolvedEntityId]);

  useEffect(() => {
    if (resolution.status !== 'RESOLVED') return;
    loadFindings(resolution.label);
    return findingsGate.invalidate;
  }, [findingsGate, loadFindings, resolution]);

  const subjectLabel = useMemo(() => {
    if (resolution.status === 'RESOLVED') return resolution.label ?? resolution.entityId;
    if (target?.kind === 'name') return target.value;
    return null;
  }, [resolution, target]);

  if (!target) return null;

  return (
    <section
      aria-label="Subject dossier"
      className="app-panel space-y-3 overflow-hidden rounded-xl border border-accent/30 bg-canvas/40 p-4"
    >
      <header className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="app-eyebrow text-accent">Subject dossier</div>
          <h2 className="mt-1 truncate text-[15px] font-semibold text-fg">
            {subjectLabel ?? 'Resolving subject…'}
          </h2>
          {target.kind === 'name' && resolution.status === 'RESOLVED' && (
            <p className="mt-0.5 text-caption text-muted">
              Resolved from &ldquo;{target.value}&rdquo;.
            </p>
          )}
        </div>
        <button
          type="button"
          onClick={clear}
          className="focus-ring shrink-0 rounded-lg border border-control px-3 py-1.5 text-label font-semibold text-fg-secondary transition hover:border-control hover:text-fg"
        >
          Close dossier
        </button>
      </header>

      {resolution.status === 'RESOLVING' && (
        <p role="status" className="text-label text-muted">
          Resolving subject in the entity index…
        </p>
      )}

      {resolution.status === 'ERROR' && (
        <p role="alert" className="text-label text-danger">
          Subject resolution failed: {resolution.message}
        </p>
      )}

      {resolution.status === 'NONE' && (
        <p className="text-label text-muted">
          No corpus entity matches &ldquo;{target.value}&rdquo;
          in the 500 most-cited entities. Add data or refine the name to try again.
        </p>
      )}

      {resolution.status === 'AMBIGUOUS' && (
        <div className="rounded-xl border border-warning/30 bg-warning/5 p-3">
          <p className="text-label font-semibold text-warning">
            &ldquo;{target.value}&rdquo; matches {resolution.matches.length} entities. Pick one.
          </p>
          <ul className="mt-2 flex flex-wrap gap-1.5" role="list">
            {resolution.matches.map((entity) => (
              <li key={entity.entityId}>
                <button
                  type="button"
                  onClick={() => open({ kind: 'entity', value: entity.entityId })}
                  className="focus-ring rounded-full border border-warning/30 bg-warning/10 px-2 py-1 text-caption font-semibold text-warning"
                >
                  {entity.displayValue}
                  <span className="ml-1 text-warning">
                    · {entity.kind.toLocaleLowerCase()} · {entity.documentCount} docs
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {resolution.status === 'RESOLVED' && (
        <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
          <DossierSection
            title="Findings that mention this subject"
            state={findings}
            emptyText="No detector output currently mentions this subject."
            render={(items) => (
              <ul className="flex flex-col gap-2" aria-label="Findings for this subject">
                {items.slice(0, 5).map((finding) => (
                  <DossierFindingRow
                    key={finding.id}
                    finding={finding}
                    onOpen={() => {
                      setWorkspaceView('evidence');
                    }}
                  />
                ))}
                {items.length > 5 && (
                  <li className="text-caption text-muted">
                    + {items.length - 5} more finding{items.length - 5 === 1 ? '' : 's'} mention this
                    subject. Open the findings inbox to see them all.
                  </li>
                )}
                <li className="text-caption italic text-muted">
                  Matches are text-based on the subject&rsquo;s display value; a namesake is possible.
                </li>
              </ul>
            )}
          />
          <DossierSection
            title="Corpus neighborhood"
            state={neighborhood}
            emptyText="No corpus edges recorded for this subject yet."
            render={(value) => {
              if (!value) return <p className="text-caption text-muted">Entity has no recorded neighborhood.</p>;
              const edges = value.edges.slice(0, 8);
              return (
                <div className="space-y-2">
                  <p className="text-caption text-muted">
                    Center: <span className="font-mono text-fg-secondary">{value.center.displayValue}</span> · seen
                    in {value.center.documentCount} documents.
                  </p>
                  {edges.length === 0 ? (
                    <p className="text-caption text-muted">No co-occurrence edges.</p>
                  ) : (
                    <ul className="flex flex-wrap gap-1.5">
                      {edges.map((edge) => (
                        <li key={edge.neighbor.entityId}>
                          <button
                            type="button"
                            onClick={() =>
                              open({ kind: 'entity', value: edge.neighbor.entityId })
                            }
                            className="focus-ring rounded-full border border-control bg-canvas/70 px-2 py-1 text-caption font-medium text-fg-secondary transition hover:border-accent/30 hover:text-accent"
                          >
                            {edge.neighbor.displayValue}
                            <span className="ml-1 text-muted">· {edge.sharedDocumentCount} docs</span>
                          </button>
                        </li>
                      ))}
                    </ul>
                  )}
                  {(value.edges.length > 8 || (neighborTotal !== null && neighborTotal > 8)) && (
                    <p className="text-caption text-muted">
                      Showing 8{neighborTotal !== null ? ` of ${neighborTotal.toLocaleString()}` : ''}. Open the entity graph to see every one.
                    </p>
                  )}
                </div>
              );
            }}
          />
        </div>
      )}
    </section>
  );
}

type Resolution =
  | { status: 'RESOLVING' }
  | { status: 'RESOLVED'; entityId: string; label: string | null }
  | { status: 'AMBIGUOUS'; matches: CorpusEntityNode[] }
  | { status: 'NONE' }
  | { status: 'ERROR'; message: string };

type SectionState<T> =
  | { status: 'LOADING' }
  | { status: 'READY'; value: T }
  | { status: 'ERROR'; message: string };

function DossierSection<T>({
  title,
  state,
  emptyText,
  render,
}: {
  title: string;
  state: SectionState<T>;
  emptyText: string;
  render: (value: T) => React.ReactNode;
}) {
  return (
    <section className="rounded-xl border border-line bg-canvas/40 p-3">
      <h3 className="text-label font-semibold uppercase tracking-wider text-fg-secondary">{title}</h3>
      <div className="mt-2">
        {state.status === 'LOADING' && (
          <p role="status" className="text-caption text-muted">
            Loading…
          </p>
        )}
        {state.status === 'ERROR' && (
          <p role="alert" className="text-caption text-danger">
            {state.message}
          </p>
        )}
        {state.status === 'READY' && (Array.isArray(state.value) && state.value.length === 0
          ? <p className="text-caption text-muted">{emptyText}</p>
          : render(state.value))}
      </div>
    </section>
  );
}

function DossierFindingRow({ finding, onOpen }: { finding: Finding; onOpen: () => void }) {
  const [shape, tone] = severityGlyph(finding.severity);
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        className="focus-ring flex w-full items-start gap-2 rounded-lg border border-control bg-canvas/40 p-2 text-left transition hover:border-accent/30 hover:bg-accent/5"
        aria-label={`Open finding: ${finding.headline}`}
      >
        <span aria-hidden="true" className={`mt-0.5 grid h-5 w-5 shrink-0 place-items-center rounded ${tone}`}>
          <svg viewBox="-6 -6 12 12" className="h-3 w-3">
            <ShapeGlyph shape={shape} cx={0} cy={0} radius={4.5} fill="currentColor" />
          </svg>
        </span>
        <span className="min-w-0 flex-1">
          <span className="block text-label font-semibold text-fg">{finding.headline}</span>
          <span className="mt-0.5 block text-caption text-muted">{finding.explanation}</span>
        </span>
      </button>
    </li>
  );
}

function matchesName(entity: CorpusEntityNode, query: string): boolean {
  return entity.displayValue.toLocaleLowerCase().includes(query);
}

function mentionsSubject(finding: Finding, query: string): boolean {
  if (finding.explanation.toLocaleLowerCase().includes(query)) return true;
  if (finding.headline.toLocaleLowerCase().includes(query)) return true;
  return finding.citations.some((citation) => {
    if (citation.quote && citation.quote.toLocaleLowerCase().includes(query)) return true;
    if (citation.label && citation.label.toLocaleLowerCase().includes(query)) return true;
    return false;
  });
}

function severityGlyph(severity: FindingSeverity): [GlyphShape, string] {
  switch (severity) {
    case 'CRITICAL':
      return ['diamond', 'text-danger'];
    case 'SERIOUS':
      return ['triangle', 'text-warning'];
    case 'NOTABLE':
      return ['circle', 'text-accent'];
    case 'INFO':
      return ['ring', 'text-muted'];
  }
}
