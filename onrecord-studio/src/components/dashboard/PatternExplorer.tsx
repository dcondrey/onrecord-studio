import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type {
  CorpusEvidenceReference,
  CorpusIndexStatus,
  CorpusPattern,
  CorpusPatternKind,
  DocumentTextChunk,
} from '../../types';
import { cn } from '../../utils/cn';
import {
  loadPatternRecipes,
  parsePatternRecipeTerms,
  patternRecipeFingerprint,
  persistPatternRecipes,
  type PatternKindFilter,
  type PatternRecipe,
} from '../../utils/patternRecipes';
import Icon from '../ui/icons';
import PageHeader from '../ui/PageHeader';
import Panel from '../ui/Panel';
import Button from '../ui/Button';
import Badge from '../ui/Badge';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import { Field, Input, Select } from '../ui/Field';
import EntityGraph from './EntityGraph';
import ResolutionReview from './ResolutionReview';
import EmptyState from '../ui/EmptyState';
import { openSourceConnector } from './sourceEvents';

const patternLabels: Record<CorpusPatternKind, string> = {
  EXACT_DUPLICATE: 'Exact copies',
  NEAR_DUPLICATE: 'Near copies',
  SHARED_STATUTE: 'Shared authorities',
  SHARED_ENTITY: 'Shared entities',
  SHARED_TERM: 'Recurring language',
  TEMPORAL_CLUSTER: 'Date clusters',
  ENTITY_RELATIONSHIP: 'Entity relationships',
};

export default function PatternExplorer() {
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const patternEvidenceSelection = useAppStore((state) => state.patternEvidenceSelection);
  const setPatternEvidenceSelection = useAppStore((state) => state.setPatternEvidenceSelection);
  const [patterns, setPatterns] = useState<CorpusPattern[]>([]);
  const [status, setStatus] = useState<CorpusIndexStatus | null>(null);
  const [kindFilter, setKindFilter] = useState<PatternKindFilter>('ALL');
  const [minimumEvidenceCount, setMinimumEvidenceCount] = useState(2);
  const [minimumConfidence, setMinimumConfidence] = useState(0);
  const [indexedFrom, setIndexedFrom] = useState('');
  const [indexedTo, setIndexedTo] = useState('');
  const [requiredTerms, setRequiredTerms] = useState('');
  const [recipeName, setRecipeName] = useState('');
  const [recipeFormOpen, setRecipeFormOpen] = useState(false);
  const [savedRecipes, setSavedRecipes] = useState<PatternRecipe[]>(() =>
    typeof window === 'undefined' ? [] : loadPatternRecipes(window.localStorage),
  );
  const [lastPreviewFingerprint, setLastPreviewFingerprint] = useState<string | null>(null);
  const [selectedPatternId, setSelectedPatternId] = useState<string | null>(patternEvidenceSelection?.patternId ?? null);
  const [excerpt, setExcerpt] = useState<{
    evidence: CorpusEvidenceReference;
    chunk: DocumentTextChunk;
  } | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [excerptLoadingKey, setExcerptLoadingKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [listFailed, setListFailed] = useState(false);
  const refreshGate = useLatestRequestGate();
  const evidenceGate = useLatestRequestGate();
  const restoredEvidenceKey = useRef<string | null>(null);
  const lastGatedPatternId = useRef<string | null>(null);

  const currentRecipeControls = useCallback(() => ({
    kindFilter,
    minimumEvidenceCount,
    minimumConfidence,
    indexedFrom,
    indexedTo,
    requiredTerms,
  }), [indexedFrom, indexedTo, kindFilter, minimumConfidence, minimumEvidenceCount, requiredTerms]);

  const refresh = useCallback(async (markAsRecipePreview = false) => {
    // Controls stay live during a refresh, so refreshes overlap; only the latest may publish, or
    // patterns for one portal or threshold would render under another's controls.
    const request = refreshGate.begin();
    setIsLoading(true);
    setError(null);
    try {
      const [nextPatterns, nextStatus] = await Promise.all([
        invokeCommand('query_corpus_patterns', {
          query: {
            portalKey: activePortalKey,
            minimumEvidenceCount,
            minimumConfidenceBasisPoints: minimumConfidence,
            indexedFrom: toRfc3339(indexedFrom),
            indexedTo: toRfc3339(indexedTo, true),
            // Every kind, always: the per-kind counts come from this list and the kind filter is
            // applied client-side, so filtering here would zero every other kind's count.
            patternKinds: [],
            requiredTerms: parsePatternRecipeTerms(requiredTerms),
            resultLimit: 250,
          },
        }),
        invokeCommand('get_corpus_index_status', {}),
      ]);
      if (!refreshGate.isCurrent(request)) return;
      setPatterns(nextPatterns);
      setStatus(nextStatus);
      setListFailed(false);
      if (markAsRecipePreview) {
        setLastPreviewFingerprint(patternRecipeFingerprint(currentRecipeControls()));
      }
      setSelectedPatternId((current) =>
        current && nextPatterns.some((pattern) => pattern.patternId === current)
          ? current
          : nextPatterns[0]?.patternId ?? null,
      );
    } catch (reason) {
      if (!refreshGate.isCurrent(request)) return;
      // The previous list answered other controls; keeping it would present it as this query's.
      setPatterns([]);
      setListFailed(true);
      setError(formatNativeError(reason));
    } finally {
      if (refreshGate.isCurrent(request)) setIsLoading(false);
    }
  }, [activePortalKey, currentRecipeControls, indexedFrom, indexedTo, minimumConfidence, minimumEvidenceCount, refreshGate, requiredTerms]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const filteredPatterns = useMemo(
    () =>
      kindFilter === 'ALL'
        ? patterns
        : patterns.filter((pattern) => pattern.kind === kindFilter),
    [kindFilter, patterns],
  );
  const selectedPattern =
    filteredPatterns.find((pattern) => pattern.patternId === selectedPatternId)
    ?? filteredPatterns[0]
    ?? null;

  // The displayed pattern also changes when a refresh or the kind filter drops the selected one, and
  // its evidence must not survive under the next. The first null-to-pattern transition is skipped
  // so it does not race the evidence restore below, which begins its own request for it.
  useEffect(() => {
    const nextPatternId = selectedPattern?.patternId ?? null;
    const previousPatternId = lastGatedPatternId.current;
    lastGatedPatternId.current = nextPatternId;
    if (previousPatternId !== null && previousPatternId !== nextPatternId) {
      evidenceGate.invalidate();
      setExcerpt(null);
      setExcerptLoadingKey(null);
    }
  }, [selectedPattern?.patternId, evidenceGate]);

  const recipeIsPreviewed = lastPreviewFingerprint === patternRecipeFingerprint(currentRecipeControls());

  const saveRecipe = useCallback(() => {
    const name = recipeName.trim();
    if (!name || !recipeIsPreviewed) return;
    const nextRecipe: PatternRecipe = {
      id: globalThis.crypto?.randomUUID?.() ?? `pattern-recipe-${Date.now()}`,
      name: name.slice(0, 80),
      ...currentRecipeControls(),
      createdAt: new Date().toISOString(),
    };
    const nextRecipes = [nextRecipe, ...savedRecipes].slice(0, 50);
    try {
      persistPatternRecipes(window.localStorage, nextRecipes);
      setSavedRecipes(nextRecipes);
      setRecipeName('');
      setRecipeFormOpen(false);
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  }, [currentRecipeControls, recipeIsPreviewed, recipeName, savedRecipes]);

  const loadRecipe = useCallback((recipe: PatternRecipe) => {
    setKindFilter(recipe.kindFilter);
    setMinimumEvidenceCount(recipe.minimumEvidenceCount);
    setMinimumConfidence(recipe.minimumConfidence);
    setIndexedFrom(recipe.indexedFrom.slice(0, 10));
    setIndexedTo(recipe.indexedTo.slice(0, 10));
    setRequiredTerms(recipe.requiredTerms);
    setLastPreviewFingerprint(null);
  }, []);

  const deleteRecipe = useCallback((id: string) => {
    const nextRecipes = savedRecipes.filter((recipe) => recipe.id !== id);
    try {
      persistPatternRecipes(window.localStorage, nextRecipes);
      setSavedRecipes(nextRecipes);
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  }, [savedRecipes]);

  const openEvidence = useCallback(async (evidence: CorpusEvidenceReference) => {
    const key = evidenceKey(evidence);
    restoredEvidenceKey.current = key;
    setPatternEvidenceSelection({
      patternId: selectedPattern?.patternId ?? patternEvidenceSelection?.patternId ?? '',
      portalKey: evidence.portalKey,
      documentId: evidence.documentId,
      characterStart: evidence.characterStart,
    });
    const request = evidenceGate.begin();
    setExcerptLoadingKey(key);
    setError(null);
    const sourceOffset = evidence.characterStart ?? 0;
    try {
      const chunk = await invokeCommand('get_document_text_chunk', {
        documentId: evidence.documentId,
        portalKey: evidence.portalKey,
        characterOffset: Math.max(0, sourceOffset - 500),
        characterLimit: 3_000,
      });
      if (!evidenceGate.isCurrent(request)) return;
      setExcerpt({ evidence, chunk });
    } catch (reason) {
      if (!evidenceGate.isCurrent(request)) return;
      setError(formatNativeError(reason));
    } finally {
      if (evidenceGate.isCurrent(request)) setExcerptLoadingKey(null);
    }
  }, [evidenceGate, patternEvidenceSelection?.patternId, selectedPattern?.patternId, setPatternEvidenceSelection]);

  useEffect(() => {
    if (!patternEvidenceSelection || selectedPattern?.patternId !== patternEvidenceSelection.patternId) return;
    if (patternEvidenceSelection.portalKey === null || patternEvidenceSelection.documentId === null) return;
    const evidence = selectedPattern.evidence.find((candidate) =>
      candidate.portalKey === patternEvidenceSelection.portalKey
      && candidate.documentId === patternEvidenceSelection.documentId
      && candidate.characterStart === patternEvidenceSelection.characterStart
    );
    if (!evidence || restoredEvidenceKey.current === evidenceKey(evidence)) return;
    void openEvidence(evidence);
  }, [openEvidence, patternEvidenceSelection, selectedPattern]);

  const portals = useAppStore((state) => state.portals);
  const portalName = (portalKey: string) => portals.find(({ registration }) => registration.portalKey === portalKey)?.registration.displayName ?? portalKey;
  const visibleKinds = (Object.keys(patternLabels) as CorpusPatternKind[])
    .map((kind) => ({ kind, count: patterns.filter((pattern) => pattern.kind === kind).length }))
    .filter(({ kind, count }) => count > 0 || kindFilter === kind);

  // An empty corpus has nothing for the filters to narrow, so it points at the step that fills it.
  const corpusEmpty = status !== null && status.indexedDocumentCount === 0 && !listFailed && patterns.length === 0;

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-content flex-col gap-6 px-4 pb-10 pt-6 md:px-6">
        <PageHeader
          title="Patterns"
          summary={status
            ? <span className="tabular-nums">{status.indexedDocumentCount.toLocaleString()} documents · {status.indexedRelationshipCount.toLocaleString()} relationships indexed.</span>
            : 'Signals matched across extracted files, each linked to its source.'}
          method="Deterministic fingerprints, normalized authorities, recurring language and dates are matched across extracted files. Every signal links back to a document and a source offset."
        />

        {corpusEmpty ? (
          <EmptyState
            icon="link"
            title="No documents to connect yet"
            body="Patterns appear once a source is collected and its files are extracted. Every signal links back to a document and a source offset."
            actions={<Button variant="primary" onClick={openSourceConnector}>Add a source</Button>}
          />
        ) : (
          <>
            <Panel aria-label="Pattern filters" className="p-5">
              <div className="grid items-end gap-3 [grid-template-columns:repeat(auto-fit,minmax(10rem,1fr))]">
                <Field label="Minimum evidence">
                  <Select value={minimumEvidenceCount} onChange={(event) => setMinimumEvidenceCount(Number(event.target.value))} className="tabular-nums">
                    {[2, 3, 5, 10].map((value) => <option key={value} value={value}>{value} files</option>)}
                  </Select>
                </Field>
                <Field label="Confidence">
                  <Select value={minimumConfidence} onChange={(event) => setMinimumConfidence(Number(event.target.value))} className="tabular-nums">
                    <option value={0}>Any</option>
                    <option value={7000}>70% or more</option>
                    <option value={8000}>80% or more</option>
                    <option value={9000}>90% or more</option>
                    <option value={9500}>95% or more</option>
                  </Select>
                </Field>
                <Field label="Indexed from">
                  <Input type="date" value={indexedFrom} max={indexedTo || undefined} onChange={(event) => setIndexedFrom(event.target.value)} className="tabular-nums" />
                </Field>
                <Field label="Indexed to">
                  <Input type="date" value={indexedTo} min={indexedFrom || undefined} onChange={(event) => setIndexedTo(event.target.value)} className="tabular-nums" />
                </Field>
                <Field label="Required terms">
                  <Input value={requiredTerms} onChange={(event) => setRequiredTerms(event.target.value)} placeholder="deadline, response" />
                </Field>
                <div className="flex items-center gap-2">
                  <Button variant="primary" onClick={() => void refresh(true)} disabled={isLoading}>{isLoading ? 'Applying…' : 'Apply'}</Button>
                  <Button onClick={() => setRecipeFormOpen((open) => !open)} aria-expanded={recipeFormOpen} aria-controls="pattern-recipe-form">Save as recipe…</Button>
                </div>
              </div>
              {recipeFormOpen && (
                <form
                  id="pattern-recipe-form"
                  className="motion-reveal mt-4 flex flex-wrap items-end gap-3 border-t border-line pt-4"
                  onSubmit={(event) => { event.preventDefault(); saveRecipe(); }}
                >
                  <Field label="Recipe name" help={recipeIsPreviewed ? 'Saved on this device. A recipe never alters source evidence.' : 'Apply these filters first, so the recipe saves what you have seen.'} className="w-72">
                    <Input value={recipeName} maxLength={80} onChange={(event) => setRecipeName(event.target.value)} autoFocus />
                  </Field>
                  <Button type="submit" disabled={!recipeName.trim() || !recipeIsPreviewed} className="mb-6">Save recipe</Button>
                </form>
              )}
              {savedRecipes.length > 0 && (
                <div className="mt-4 flex flex-wrap items-center gap-2" aria-label="Saved pattern recipes">
                  <span className="text-caption text-muted">Recipes</span>
                  {savedRecipes.map((recipe) => (
                    <span key={recipe.id} className="inline-flex h-7 items-center overflow-hidden rounded-md border border-line-strong">
                      <button type="button" onClick={() => loadRecipe(recipe)} className="focus-ring h-full px-2.5 text-control text-fg-secondary hover:bg-raised hover:text-fg">
                        {recipe.name}
                      </button>
                      <button type="button" onClick={() => deleteRecipe(recipe.id)} aria-label={`Delete ${recipe.name}`} className="focus-ring grid h-full w-7 place-items-center border-l border-line text-muted hover:bg-raised hover:text-danger">
                        <Icon name="x" size="xs" />
                      </button>
                    </span>
                  ))}
                </div>
              )}
            </Panel>

            <div className="flex flex-wrap gap-1.5" role="group" aria-label="Pattern type">
              <FilterButton selected={kindFilter === 'ALL'} onClick={() => setKindFilter('ALL')} count={listFailed ? null : patterns.length}>All signals</FilterButton>
              {visibleKinds.map(({ kind, count }) => (
                <FilterButton key={kind} selected={kindFilter === kind} onClick={() => setKindFilter(kind)} count={listFailed ? null : count}>{patternLabels[kind]}</FilterButton>
              ))}
            </div>

            {error && (
              <InlineError
                summary={listFailed ? 'Signals couldn’t be read for these filters.' : 'The excerpt couldn’t be opened.'}
                detail={error}
                onRetry={listFailed ? () => void refresh() : undefined}
              />
            )}

            <div className="grid min-h-[500px] grid-cols-1 gap-4 xl:grid-cols-[minmax(360px,0.8fr)_minmax(0,1.2fr)]">
              <section aria-labelledby="pattern-list-title" className="app-panel min-h-0 min-w-0 overflow-hidden">
                <h3 id="pattern-list-title" className="border-b border-line px-4 py-3 text-heading text-fg">
                  {listFailed
                    ? 'Signals'
                    : <><span className="tabular-nums">{filteredPatterns.length.toLocaleString()}</span> signal{filteredPatterns.length === 1 ? '' : 's'}</>}
                </h3>
                <div className="max-h-[650px] overflow-y-auto">
                  {isLoading && patterns.length === 0 ? (
                    <div className="space-y-px">
                      <Skeleton variant="lines" lines={4} label="Analyzing the corpus index" className="p-4" />
                    </div>
                  ) : listFailed ? (
                    <EmptyMessage title="No signals to show until the query succeeds." />
                  ) : filteredPatterns.length === 0 ? (
                    <EmptyMessage title="No pattern has enough evidence for these filters yet." />
                  ) : filteredPatterns.map((pattern) => (
                    <button
                      key={pattern.patternId}
                      type="button"
                      aria-current={selectedPattern?.patternId === pattern.patternId ? 'true' : undefined}
                      onClick={() => {
                        setSelectedPatternId(pattern.patternId);
                        setPatternEvidenceSelection({ patternId: pattern.patternId, portalKey: null, documentId: null, characterStart: null });
                        setExcerpt(null);
                        // Invalidated here as well as in the effect: the effect runs after paint, and an
                        // excerpt for the previous pattern can resolve in between.
                        evidenceGate.invalidate();
                        setExcerptLoadingKey(null);
                      }}
                      className={cn(
                        'focus-ring block w-full border-b border-line px-4 py-3 text-left transition-colors duration-fast hover:bg-raised/60 focus-visible:ring-inset',
                        selectedPattern?.patternId === pattern.patternId && 'bg-accent/5 shadow-[inset_2px_0_0_rgb(var(--accent))]',
                      )}
                    >
                      <span className="flex items-center justify-between gap-3">
                        <Badge>{patternLabels[pattern.kind]}</Badge>
                        <span className="text-caption tabular-nums text-muted">
                          {(pattern.confidenceBasisPoints / 100).toFixed(0)}% match · {pattern.evidence.length} file{pattern.evidence.length === 1 ? '' : 's'}
                        </span>
                      </span>
                      <span className="mt-2 block text-body font-medium text-fg">{pattern.title}</span>
                      <span className="mt-1 line-clamp-2 text-caption text-muted">{pattern.explanation}</span>
                    </button>
                  ))}
                </div>
              </section>

              <section aria-label="Signal evidence" className="app-panel min-h-0 min-w-0 overflow-hidden">
                {selectedPattern ? (
                  <div className="flex h-full min-h-0 flex-col">
                    <div className="border-b border-line p-5">
                      <Badge title={`Algorithm ${selectedPattern.algorithmVersion}`}>{patternLabels[selectedPattern.kind]}</Badge>
                      <h3 className="mt-2 text-heading text-fg">{selectedPattern.title}</h3>
                      <p className="mt-1 text-body text-muted">{selectedPattern.explanation}</p>
                    </div>
                    <div className="min-h-0 flex-1 overflow-y-auto p-5">
                      <h4 className="mb-2 text-body font-medium text-fg">Source evidence</h4>
                      <ul className="space-y-2">
                        {selectedPattern.evidence.map((evidence) => {
                          const key = evidenceKey(evidence);
                          return (
                            <li key={key}>
                              <button
                                type="button"
                                onClick={() => void openEvidence(evidence)}
                                className="focus-ring group w-full rounded-lg border border-line bg-canvas p-3 text-left transition-colors duration-fast hover:border-line-strong hover:bg-raised/60"
                              >
                                <span className="flex items-center justify-between gap-3">
                                  <span className="min-w-0 truncate text-body text-fg-secondary">
                                    {portalName(evidence.portalKey)} · Request <span className="tabular-nums">{evidence.requestId}</span>
                                  </span>
                                  <span className="shrink-0 text-caption text-accent group-hover:underline">
                                    {excerptLoadingKey === key ? 'Opening…' : 'Open excerpt'}
                                  </span>
                                </span>
                                <span className="mt-1 block text-caption text-muted">{evidence.evidenceLabel}</span>
                                <span className="mt-1 block truncate font-mono text-code text-muted" title={`Document ${evidence.documentId}${evidence.characterStart !== null ? `, characters ${evidence.characterStart}–${evidence.characterEnd}` : ''}; SHA-256 ${evidence.contentSha256}`}>
                                  sha256 {evidence.contentSha256.slice(0, 16)}…
                                </span>
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                      {excerpt && (
                        <div className="mt-4 rounded-lg border border-line bg-canvas p-4">
                          <div className="flex items-start justify-between gap-3">
                            <div className="min-w-0">
                              <p className="text-body font-medium text-fg">Masked source excerpt</p>
                              <p className="mt-0.5 text-caption text-muted">
                                {portalName(excerpt.chunk.portalKey)} · document <span className="tabular-nums">{excerpt.chunk.documentId}</span>
                              </p>
                            </div>
                            <Button variant="ghost" size="sm" iconOnly icon="x" aria-label="Close excerpt" onClick={() => setExcerpt(null)} />
                          </div>
                          <pre className="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words font-mono text-code text-fg-secondary">
                            {excerpt.chunk.maskedText}
                          </pre>
                          <p className="mt-2 text-caption text-muted">
                            {excerpt.chunk.maskedFindingCount} sensitive value{excerpt.chunk.maskedFindingCount === 1 ? '' : 's'} masked locally.
                          </p>
                        </div>
                      )}
                    </div>
                  </div>
                ) : (
                  <EmptyMessage title="Select a signal to inspect its evidence." />
                )}
              </section>
            </div>

            <CollapsibleSection title="Entity graph"><EntityGraph /></CollapsibleSection>
            <CollapsibleSection title="Resolve entities"><ResolutionReview /></CollapsibleSection>
          </>
        )}
      </div>
    </div>
  );
}

function evidenceKey(evidence: CorpusEvidenceReference) {
  return `${evidence.portalKey}:${evidence.documentId}:${evidence.characterStart ?? 'document'}`;
}

/** A date control yields `YYYY-MM-DD`; "to" covers that whole day. Older recipes hold a local date-time. */
function toRfc3339(value: string, endOfDay = false) {
  if (!value) return null;
  const date = new Date(value.includes('T') ? value : `${value}T00:00`);
  if (Number.isNaN(date.getTime())) return null;
  if (endOfDay && !value.includes('T')) date.setDate(date.getDate() + 1);
  return date.toISOString();
}

/** Mounted on first open and kept, so its own filters survive closing it. */
function CollapsibleSection({ title, children }: { title: string; children: ReactNode }) {
  const [everOpened, setEverOpened] = useState(false);
  return (
    <details className="group app-panel" onToggle={(event) => { if (event.currentTarget.open) setEverOpened(true); }}>
      <summary className="focus-ring flex h-14 cursor-pointer list-none items-center gap-2 rounded-xl px-5 text-heading text-fg [&::-webkit-details-marker]:hidden">
        {title}
      </summary>
      {everOpened && <div className="border-t border-line p-5">{children}</div>}
    </details>
  );
}

/** `count` is null when the query failed: an unknown count is not zero. */
function FilterButton({ selected, onClick, count, children }: { selected: boolean; onClick: () => void; count: number | null; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        'focus-ring inline-flex h-7 items-center gap-1.5 rounded-md border px-2.5 text-control transition-colors duration-fast',
        selected ? 'border-accent/30 bg-accent/10 text-accent' : 'border-line-strong text-fg-secondary hover:bg-raised hover:text-fg',
      )}
    >
      {children}
      {count === null
        ? <span className="text-muted"><span aria-hidden="true">—</span><span className="sr-only">count unavailable</span></span>
        : <span className="tabular-nums text-muted">{count}</span>}
    </button>
  );
}

function EmptyMessage({ title }: { title: string }) {
  return <p className="grid min-h-48 place-items-center p-8 text-center text-body text-muted">{title}</p>;
}
