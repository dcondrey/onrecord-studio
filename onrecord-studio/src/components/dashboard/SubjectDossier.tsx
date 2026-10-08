import { useCallback, useEffect, useMemo, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import NeighbourBars from '../charts/figures/NeighbourBars';
import { cn } from '../../utils/cn';
import type { CorpusEntityNeighborhood, CorpusEntityNode } from '../../types/corpus';
import type { Finding } from '../../types';
import SeverityGlyph from './SeverityGlyph';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';

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
  const inspect = useAppStore((state) => state.inspect);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);

  const [resolution, setResolution] = useState<Resolution>({ status: 'RESOLVING' });
  // `unknown` is a count that was asked for and failed, which is not the same as one not yet read.
  const [neighborTotal, setNeighborTotal] = useState<number | 'unknown' | null>(null);
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
        setResolution({ status: 'ERROR', message: formatNativeError(reason) });
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
        .catch(() => {
          if (neighborhoodGate.isCurrent(request)) setNeighborTotal('unknown');
        });
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
          setNeighborhood({ status: 'ERROR', message: formatNativeError(reason) });
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
          // The findings filter has no entity axis, so a finding matches when it names the subject
          // in its text or a citation. A namesake can match; the section says so.
          const query = (label ?? '').trim().toLocaleLowerCase();
          const matches = query.length > 0
            ? rows.filter((row) => mentionsSubject(row, query))
            : [];
          setFindings({ status: 'READY', value: matches });
        })
        .catch((reason) => {
          if (!findingsGate.isCurrent(request)) return;
          setFindings({ status: 'ERROR', message: formatNativeError(reason) });
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

  const neighborhoodPending = neighborhood.status === 'LOADING';
  useEffect(() => {
    if (resolution.status !== 'RESOLVED') return;
    // Findings are matched by name. Without one, an empty match is not a measurement, so the
    // section says it cannot match rather than that nothing mentions the subject. While the
    // neighborhood may still supply the name, the section stays loading.
    if (!resolution.label?.trim()) {
      findingsGate.invalidate();
      setFindings(neighborhoodPending ? { status: 'LOADING' } : { status: 'UNMATCHABLE' });
      return;
    }
    loadFindings(resolution.label);
    return findingsGate.invalidate;
  }, [findingsGate, loadFindings, neighborhoodPending, resolution]);

  const subjectLabel = useMemo(() => {
    if (resolution.status === 'RESOLVED') return resolution.label;
    if (target?.kind === 'name') return target.value;
    return null;
  }, [resolution, target]);

  if (!target) return null;

  const entityId = resolution.status === 'RESOLVED' ? resolution.entityId : undefined;
  const center = neighborhood.status === 'READY' && neighborhood.value?.center.entityId === entityId ? neighborhood.value?.center : null;
  const findingsEmpty = findings.status === 'UNMATCHABLE' || (findings.status === 'READY' && findings.value.length === 0);
  const unnamed = resolution.status === 'RESOLVED' && subjectLabel === null && !neighborhoodPending;

  return (
    <section aria-label="Subject dossier" className="app-panel space-y-4 p-5">
      <header className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className={cn('truncate text-title', unnamed ? 'text-muted' : 'text-fg')} title={entityId}>
            {subjectLabel ?? (unnamed ? 'Unnamed subject' : 'Resolving subject…')}
          </h2>
          <p className="mt-1 text-caption text-muted">
            {unnamed
              ? 'Name not yet read from the entity index'
              : center
                ? `Seen in ${documentCount(center.documentCount)}`
                : target.kind === 'name' && resolution.status === 'RESOLVED'
                  ? <>Resolved from &ldquo;{target.value}&rdquo;</>
                  : null}
          </p>
        </div>
        <Button size="sm" onClick={clear}>Close dossier</Button>
      </header>

      {resolution.status === 'RESOLVING' && <Skeleton variant="lines" lines={2} label="Resolving subject in the entity index" />}

      {resolution.status === 'ERROR' && (
        <InlineError summary="This subject couldn’t be looked up in the entity index." detail={resolution.message} onRetry={resolveTarget} />
      )}

      {resolution.status === 'NONE' && (
        <p className="text-body text-muted">
          No entity matches &ldquo;{target.value}&rdquo;
          among the 500 most-cited entities. Add data or refine the name to try again.
        </p>
      )}

      {resolution.status === 'AMBIGUOUS' && (
        <div className="rounded-lg border border-warning/30 bg-warning/10 p-3">
          <p className="text-body font-medium text-warning">
            &ldquo;{target.value}&rdquo; matches {resolution.matches.length} entities. Pick one.
          </p>
          <ul className="mt-2 flex flex-wrap gap-1.5" role="list">
            {resolution.matches.map((entity) => (
              <li key={entity.entityId}>
                <Button size="sm" onClick={() => open({ kind: 'entity', value: entity.entityId })}>
                  {entity.displayValue}
                  <span className="text-caption tabular-nums text-muted">
                    · {entity.kind.toLocaleLowerCase()} · {documentCount(entity.documentCount)}
                  </span>
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {resolution.status === 'RESOLVED' && (
        <>
          {findingsEmpty && (
            <p className="text-body text-muted">
              {findings.status === 'UNMATCHABLE'
                ? 'Findings can’t be matched until this subject’s name is read.'
                : 'No finding mentions this subject by name.'}
            </p>
          )}
          <div className={cn('grid grid-cols-1 gap-4', !findingsEmpty && 'lg:grid-cols-2')}>
            {!findingsEmpty && (
              <DossierSection
                title="Findings"
                state={findings}
                render={(items) => (
                  <ul className="flex flex-col gap-2" aria-label="Findings for this subject">
                    {items.slice(0, 5).map((finding) => (
                      <DossierFindingRow
                        key={finding.id}
                        finding={finding}
                        onOpen={() => inspect({ kind: 'finding', id: finding.id, label: finding.headline })}
                      />
                    ))}
                    {items.length > 5 && (
                      <li className="text-caption text-muted">
                        {items.length - 5} more finding{items.length - 5 === 1 ? '' : 's'} mention this subject.
                      </li>
                    )}
                    <li className="text-caption text-muted">
                      Matched on the subject&rsquo;s name in the finding text; a namesake is possible.
                    </li>
                  </ul>
                )}
              />
            )}
            <DossierSection
              title="Related names"
              state={neighborhood}
              render={(value) => {
                if (!value) return <p className="text-caption text-muted">No neighborhood is recorded for this subject.</p>;
                const total = typeof neighborTotal === 'number' ? neighborTotal : null;
                return (
                  <div className="space-y-2">
                    <NeighbourBars
                      neighborhood={value}
                      total={total}
                      storageKey="subject-dossier-neighbours"
                      onOpen={(edge, { shift }) => {
                        if (shift) {
                          setSearchFilters(edge.neighbor.displayValue, null, null);
                          setWorkspaceView('evidence');
                          return;
                        }
                        open({ kind: 'entity', value: edge.neighbor.entityId });
                      }}
                    />
                    {(neighborTotal === 'unknown' || (total !== null && total > value.edges.length)) && (
                      <p className="text-caption tabular-nums text-muted">
                        {neighborTotal === 'unknown'
                          ? `Showing ${value.edges.length}; total not available.`
                          : `Showing ${value.edges.length} of ${(total ?? 0).toLocaleString()}. Open the entity graph to see every one.`}
                      </p>
                    )}
                  </div>
                );
              }}
            />
          </div>
        </>
      )}
    </section>
  );
}

function documentCount(count: number) {
  return `${count.toLocaleString()} document${count === 1 ? '' : 's'}`;
}

type Resolution =
  | { status: 'RESOLVING' }
  | { status: 'RESOLVED'; entityId: string; label: string | null }
  | { status: 'AMBIGUOUS'; matches: CorpusEntityNode[] }
  | { status: 'NONE' }
  | { status: 'ERROR'; message: string };

type SectionState<T> =
  | { status: 'LOADING' }
  | { status: 'UNMATCHABLE' }
  | { status: 'READY'; value: T }
  | { status: 'ERROR'; message: string };

function DossierSection<T>({
  title,
  state,
  render,
}: {
  title: string;
  state: SectionState<T>;
  render: (value: T) => React.ReactNode;
}) {
  return (
    <section aria-label={title} className="rounded-lg border border-line bg-canvas p-4">
      <h3 className="text-heading text-fg">{title}</h3>
      <div className="mt-3">
        {(state.status === 'LOADING' || state.status === 'UNMATCHABLE') && <Skeleton variant="lines" lines={2} label={`Loading ${title.toLowerCase()}`} />}
        {state.status === 'ERROR' && <InlineError summary={`${title} couldn’t be read.`} detail={state.message} />}
        {state.status === 'READY' && render(state.value)}
      </div>
    </section>
  );
}

function DossierFindingRow({ finding, onOpen }: { finding: Finding; onOpen: () => void }) {
  return (
    <li>
      <button
        type="button"
        onClick={onOpen}
        className="focus-ring group flex w-full items-start gap-2 rounded-md p-2 text-left transition-colors duration-fast hover:bg-raised"
        aria-label={`Open finding: ${finding.headline}`}
      >
        <SeverityGlyph severity={finding.severity} className="mt-0.5 h-5 w-5 rounded" />
        <span className="min-w-0 flex-1">
          <span className="block text-body font-medium text-fg group-hover:underline">{finding.headline}</span>
          <span className="mt-0.5 line-clamp-2 text-caption text-muted">{finding.explanation}</span>
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
