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
import EntityGraph from './EntityGraph';
import ResolutionReview from './ResolutionReview';

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
    // The "Minimum evidence" select is not disabled while a refresh runs, so refreshes are already
    // concurrent. Without a sequence the slower one wins: patterns supported by three documents
    // render under a control reading "5", and portal A's cross-document patterns render under
    // portal B. Same guard `SearchFilter` uses.
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
            indexedTo: toRfc3339(indexedTo),
            // Always fetch every kind, unfiltered: the per-kind filter-button counts below are
            // computed from `patterns`, and the UI narrows to `kindFilter` client-side via
            // `filteredPatterns`. Filtering this query by `kindFilter` would make every other
            // kind's count read 0 once a kind filter was active.
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

  // The pattern actually shown in the "Source evidence" panel can change for reasons besides the
  // pattern-list `onClick` below: `refresh()` reselects silently when the previously selected
  // pattern drops out of a new result set, and a `kindFilter` change can drop it out of
  // `filteredPatterns` before that. Whenever the *displayed* pattern changes for any reason, any
  // evidence fetched or in flight for the previous one must not survive under the new one. Skips
  // the initial null-to-first-pattern transition so it never races the mount-time evidence restore
  // effect below, which begins its own request for that same transition.
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
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  }, [currentRecipeControls, recipeIsPreviewed, recipeName, savedRecipes]);

  const loadRecipe = useCallback((recipe: PatternRecipe) => {
    setKindFilter(recipe.kindFilter);
    setMinimumEvidenceCount(recipe.minimumEvidenceCount);
    setMinimumConfidence(recipe.minimumConfidence);
    setIndexedFrom(recipe.indexedFrom);
    setIndexedTo(recipe.indexedTo);
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
    const key = `${evidence.portalKey}:${evidence.documentId}:${evidence.characterStart ?? 'document'}`;
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
    if (!evidence) return;
    const key = `${evidence.portalKey}:${evidence.documentId}:${evidence.characterStart ?? 'document'}`;
    if (restoredEvidenceKey.current === key) return;
    void openEvidence(evidence);
  }, [openEvidence, patternEvidenceSelection, selectedPattern]);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex min-h-full w-full max-w-[1800px] flex-col gap-4 p-4 lg:p-6">
        <header className="onboarding-panel flex flex-col gap-4 rounded-2xl border p-5 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="app-eyebrow flex items-center gap-2 text-accent">
              <Icon name="sparkles" className="h-3.5 w-3.5" /> Corpus intelligence
            </div>
            <h2 className="mt-2 text-[26px] font-semibold tracking-[-0.035em] text-fg sm:text-[30px]">
              Connections backed by source evidence.
            </h2>
            <p className="mt-1 max-w-3xl text-label leading-relaxed text-muted">
              Deterministic fingerprints, normalized authorities, recurring language, and dates are
              matched across extracted files. Every signal links back to a document and source offset.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-caption text-muted">
              Minimum evidence
              <select
                value={minimumEvidenceCount}
                onChange={(event) => setMinimumEvidenceCount(Number(event.target.value))}
                className="bg-transparent font-mono text-fg outline-none"
              >
                {[2, 3, 5, 10].map((value) => <option key={value} value={value}>{value}</option>)}
              </select>
            </label>
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-caption text-muted">
              Confidence
              <select
                value={minimumConfidence}
                onChange={(event) => setMinimumConfidence(Number(event.target.value))}
                className="bg-transparent font-mono text-fg outline-none"
              >
                <option value={0}>Any</option>
                <option value={7000}>70%+</option>
                <option value={8000}>80%+</option>
                <option value={9000}>90%+</option>
                <option value={9500}>95%+</option>
              </select>
            </label>
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-caption text-muted">
              Indexed after
              <input type="datetime-local" value={indexedFrom} max={indexedTo || undefined} onChange={(event) => setIndexedFrom(event.target.value)} className="bg-transparent font-mono text-fg outline-none" />
            </label>
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-caption text-muted">
              Indexed before
              <input type="datetime-local" value={indexedTo} min={indexedFrom || undefined} onChange={(event) => setIndexedTo(event.target.value)} className="bg-transparent font-mono text-fg outline-none" />
            </label>
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-caption text-muted">
              Required terms
              <input
                value={requiredTerms}
                onChange={(event) => setRequiredTerms(event.target.value)}
                placeholder="deadline, response"
                aria-label="Required pattern terms"
                className="w-36 bg-transparent font-mono text-fg outline-none"
              />
            </label>
            <button
              type="button"
              onClick={() => void refresh(true)}
              disabled={isLoading}
              className="primary-action focus-ring flex items-center gap-2 rounded-xl border border-transparent px-3 py-2 text-caption font-semibold text-white transition hover:brightness-105 disabled:opacity-50"
            >
              <Icon name="sync" className={cn('h-3.5 w-3.5', isLoading && 'animate-spin')} />
              Preview results
            </button>
          </div>
        </header>

        <section className="app-panel rounded-2xl border border-line p-4">
          <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <div>
              <div className="app-eyebrow">Reusable investigation recipe</div>
              <p className="mt-1 text-caption text-muted">
                Preview the exact controls above before saving. Recipes stay on this device and never alter source evidence.
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <input
                value={recipeName}
                maxLength={80}
                onChange={(event) => setRecipeName(event.target.value)}
                placeholder="Recipe name"
                aria-label="Recipe name"
                className="focus-ring h-9 rounded-lg border border-control bg-canvas/70 px-3 text-caption text-fg"
              />
              <button
                type="button"
                onClick={saveRecipe}
                disabled={!recipeName.trim() || !recipeIsPreviewed}
                className="focus-ring h-9 rounded-lg border border-accent/30 bg-accent/10 px-3 text-caption font-semibold text-accent disabled:opacity-35"
              >
                Save previewed recipe
              </button>
            </div>
          </div>
          {savedRecipes.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-2" aria-label="Saved pattern recipes">
              {savedRecipes.map((recipe) => (
                <div key={recipe.id} className="flex items-center overflow-hidden rounded-lg border border-line bg-canvas/40">
                  <button type="button" onClick={() => loadRecipe(recipe)} className="focus-ring px-3 py-2 text-caption text-fg-secondary hover:text-accent">
                    {recipe.name}
                  </button>
                  <button type="button" onClick={() => deleteRecipe(recipe.id)} aria-label={`Delete ${recipe.name}`} className="focus-ring border-l border-control px-2 py-2 text-muted hover:text-danger">
                    <Icon name="x" className="h-3 w-3" />
                  </button>
                </div>
              ))}
            </div>
          )}
        </section>

        <div className="instrument-strip app-panel grid grid-cols-2 overflow-hidden sm:grid-cols-5">
          <IndexMetric label="Documents" value={status?.indexedDocumentCount} />
          <IndexMetric label="Segments" value={status?.indexedSegmentCount} />
          <IndexMetric label="Term postings" value={status?.indexedTermCount} />
          <IndexMetric label="Relationships" value={status?.indexedRelationshipCount} />
          <IndexMetric label="Writer queue" value={status?.availableWriteCapacity} suffix=" free" />
        </div>

        <EntityGraph />

        <ResolutionReview />

        <div className="flex flex-wrap gap-1.5" role="group" aria-label="Pattern type">
          <FilterButton selected={kindFilter === 'ALL'} onClick={() => setKindFilter('ALL')}>
            All signals <span>{patterns.length}</span>
          </FilterButton>
          {(Object.keys(patternLabels) as CorpusPatternKind[]).map((kind) => (
            <FilterButton key={kind} selected={kindFilter === kind} onClick={() => setKindFilter(kind)}>
              {patternLabels[kind]}
              <span>{patterns.filter((pattern) => pattern.kind === kind).length}</span>
            </FilterButton>
          ))}
        </div>

        {error && (
          <div role="alert" className="rounded-xl border border-danger/30 bg-danger/10 px-4 py-3 text-label text-danger">
            {error}
          </div>
        )}

        <div className="grid min-h-[500px] flex-1 grid-cols-1 gap-4 xl:grid-cols-[minmax(360px,0.8fr)_minmax(0,1.2fr)]">
          <section className="app-panel min-h-0 overflow-hidden">
            <div className="border-b border-line px-4 py-3 text-label font-semibold text-fg-secondary">
              {filteredPatterns.length.toLocaleString()} explainable signals
            </div>
            <div className="max-h-[650px] overflow-y-auto">
              {isLoading && patterns.length === 0 ? (
                <EmptyState title="Analyzing corpus index…" />
              ) : filteredPatterns.length === 0 ? (
                <EmptyState title="No matching pattern has enough evidence yet." />
              ) : filteredPatterns.map((pattern) => (
                <button
                  key={pattern.patternId}
                  type="button"
                  onClick={() => {
                    setSelectedPatternId(pattern.patternId);
                    setPatternEvidenceSelection({ patternId: pattern.patternId, portalKey: null, documentId: null, characterStart: null });
                    setExcerpt(null);
                    // An evidence fetch for the previously selected pattern may still be in
                    // flight. Without invalidating it, it would still pass `isCurrent` and
                    // publish its excerpt under this newly selected pattern.
                    evidenceGate.invalidate();
                    setExcerptLoadingKey(null);
                  }}
                  className={cn(
                    'focus-ring block w-full border-b border-control px-4 py-3 text-left transition hover:bg-surface/70',
                    selectedPattern?.patternId === pattern.patternId && 'bg-accent/5 ring-1 ring-inset ring-accent/30',
                  )}
                >
                  <div className="flex items-center justify-between gap-3">
                    <span className="rounded-md border border-line-strong bg-canvas px-1.5 py-0.5 text-caption font-semibold uppercase tracking-wider text-muted">
                      {patternLabels[pattern.kind]}
                    </span>
                    <span className="font-mono text-caption text-accent">
                      {(pattern.confidenceBasisPoints / 100).toFixed(0)}% · {pattern.evidence.length} files
                    </span>
                  </div>
                  <h3 className="mt-2 text-label font-semibold leading-snug text-fg">{pattern.title}</h3>
                  <p className="mt-1 line-clamp-2 text-caption leading-relaxed text-muted">{pattern.explanation}</p>
                </button>
              ))}
            </div>
          </section>

          <section className="app-panel min-h-0 overflow-hidden">
            {selectedPattern ? (
              <div className="flex h-full min-h-0 flex-col">
                <div className="border-b border-line p-4">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <span className="app-eyebrow text-highlight">{patternLabels[selectedPattern.kind]}</span>
                    <span className="font-mono text-caption text-muted">{selectedPattern.algorithmVersion}</span>
                  </div>
                  <h2 className="mt-2 text-base font-semibold text-fg">{selectedPattern.title}</h2>
                  <p className="mt-1 text-label leading-relaxed text-muted">{selectedPattern.explanation}</p>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto p-4">
                  <div className="mb-2 text-caption font-bold uppercase tracking-[0.16em] text-muted">Source evidence</div>
                  <div className="space-y-2">
                    {selectedPattern.evidence.map((evidence) => {
                      const key = `${evidence.portalKey}:${evidence.documentId}:${evidence.characterStart ?? 'document'}`;
                      return (
                        <button
                          key={`${key}:${evidence.characterStart ?? 'document'}`}
                          type="button"
                          onClick={() => void openEvidence(evidence)}
                          className="focus-ring w-full rounded-xl border border-control bg-canvas/40 p-3 text-left transition hover:border-highlight/30 hover:bg-highlight/5"
                        >
                          <div className="flex items-center justify-between gap-3">
                            <span className="font-mono text-caption text-fg-secondary">
                              {evidence.portalKey} / request {evidence.requestId} / document {evidence.documentId}
                            </span>
                            <span className="text-caption font-semibold text-highlight">
                              {excerptLoadingKey === key ? 'Loading…' : 'Open excerpt'}
                            </span>
                          </div>
                          <p className="mt-1.5 text-caption text-muted">{evidence.evidenceLabel}</p>
                          <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 font-mono text-caption text-muted">
                            {evidence.characterStart !== null && <span>chars {evidence.characterStart}–{evidence.characterEnd}</span>}
                            <span title={evidence.contentSha256}>sha256 {evidence.contentSha256.slice(0, 16)}…</span>
                          </div>
                        </button>
                      );
                    })}
                  </div>
                  {excerpt && (
                    <div className="mt-4 rounded-xl border border-accent/30 bg-canvas/70 p-4">
                      <div className="flex items-start justify-between gap-3">
                        <div>
                          <div className="text-caption font-semibold text-accent">Masked source excerpt</div>
                          <div className="mt-0.5 font-mono text-caption text-muted">
                            {excerpt.chunk.portalKey} / document {excerpt.chunk.documentId} · offset {excerpt.chunk.characterOffset}
                          </div>
                        </div>
                        <button type="button" onClick={() => setExcerpt(null)} aria-label="Close excerpt" className="text-muted hover:text-fg-secondary">
                          <Icon name="x" className="h-3.5 w-3.5" />
                        </button>
                      </div>
                      <pre className="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words font-mono text-caption leading-relaxed text-muted">
                        {excerpt.chunk.maskedText}
                      </pre>
                      <div className="mt-2 text-caption text-muted">
                        {excerpt.chunk.maskedFindingCount} sensitive value{excerpt.chunk.maskedFindingCount === 1 ? '' : 's'} masked locally.
                      </div>
                    </div>
                  )}
                </div>
              </div>
            ) : (
              <EmptyState title="Select a signal to inspect its evidence chain." />
            )}
          </section>
        </div>
      </div>
    </div>
  );
}

function toRfc3339(value: string) {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? null : date.toISOString();
}

function IndexMetric({ label, value, suffix = '' }: { label: string; value?: number; suffix?: string }) {
  return (
    <div className="instrument-cell px-4 py-3">
      <div className="app-eyebrow">{label}</div>
      <div className="numeric-value mt-1 text-sm font-semibold text-fg">
        {value === undefined ? '—' : `${value.toLocaleString()}${suffix}`}
      </div>
    </div>
  );
}

function FilterButton({ selected, onClick, children }: { selected: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      aria-pressed={selected}
      onClick={onClick}
      className={cn(
        'focus-ring flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-caption font-medium transition',
        selected
          ? 'border-accent/30 bg-accent/10 text-accent'
          : 'border-control bg-canvas/40 text-muted hover:text-fg-secondary',
      )}
    >
      {children}
    </button>
  );
}

function EmptyState({ title }: { title: string }) {
  return (
    <div className="data-grid grid min-h-48 place-items-center p-8 text-center">
      <div className="chart-ready-card rounded-xl border px-5 py-4 backdrop-blur-xl">
        <Icon name="sparkles" className="mx-auto h-5 w-5 text-accent" />
        <p className="mt-3 text-label font-medium text-muted">{title}</p>
      </div>
    </div>
  );
}
