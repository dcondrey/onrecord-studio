import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import { invokeCommand } from '../../ipc';
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
import Icon from '../ui/icons';

const patternLabels: Record<CorpusPatternKind, string> = {
  EXACT_DUPLICATE: 'Exact copies',
  NEAR_DUPLICATE: 'Near copies',
  SHARED_STATUTE: 'Shared authorities',
  SHARED_ENTITY: 'Shared entities',
  SHARED_TERM: 'Recurring language',
  TEMPORAL_CLUSTER: 'Date clusters',
};

type KindFilter = CorpusPatternKind | 'ALL';

export default function PatternExplorer() {
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const [patterns, setPatterns] = useState<CorpusPattern[]>([]);
  const [status, setStatus] = useState<CorpusIndexStatus | null>(null);
  const [kindFilter, setKindFilter] = useState<KindFilter>('ALL');
  const [minimumEvidenceCount, setMinimumEvidenceCount] = useState(2);
  const [minimumConfidence, setMinimumConfidence] = useState(0);
  const [indexedFrom, setIndexedFrom] = useState('');
  const [indexedTo, setIndexedTo] = useState('');
  const [selectedPatternId, setSelectedPatternId] = useState<string | null>(null);
  const [excerpt, setExcerpt] = useState<{
    evidence: CorpusEvidenceReference;
    chunk: DocumentTextChunk;
  } | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [excerptLoadingKey, setExcerptLoadingKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const refreshGate = useLatestRequestGate();
  const evidenceGate = useLatestRequestGate();

  const refresh = useCallback(async () => {
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
            resultLimit: 250,
          },
        }),
        invokeCommand('get_corpus_index_status', {}),
      ]);
      if (!refreshGate.isCurrent(request)) return;
      setPatterns(nextPatterns);
      setStatus(nextStatus);
      setSelectedPatternId((current) =>
        current && nextPatterns.some((pattern) => pattern.patternId === current)
          ? current
          : nextPatterns[0]?.patternId ?? null,
      );
    } catch (reason) {
      if (!refreshGate.isCurrent(request)) return;
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (refreshGate.isCurrent(request)) setIsLoading(false);
    }
  }, [activePortalKey, indexedFrom, indexedTo, minimumConfidence, minimumEvidenceCount, refreshGate]);

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

  const openEvidence = async (evidence: CorpusEvidenceReference) => {
    // TODO(patterns): Preserve the selected evidence offset in workspace state
    // so reopening the pattern view restores the excerpt.
    const key = `${evidence.portalKey}:${evidence.documentId}`;
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
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      if (evidenceGate.isCurrent(request)) setExcerptLoadingKey(null);
    }
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex min-h-full w-full max-w-[1800px] flex-col gap-4 p-4 lg:p-6">
        <header className="onboarding-panel flex flex-col gap-4 rounded-2xl border p-5 lg:flex-row lg:items-end lg:justify-between">
          <div>
            <div className="app-eyebrow flex items-center gap-2 text-cyan-300/70">
              <Icon name="sparkles" className="h-3.5 w-3.5" /> Corpus intelligence
            </div>
            <h2 className="mt-2 text-[26px] font-semibold tracking-[-0.035em] text-slate-100 sm:text-[30px]">
              Connections backed by source evidence.
            </h2>
            <p className="mt-1 max-w-3xl text-[10px] leading-relaxed text-slate-500">
              Deterministic fingerprints, normalized authorities, recurring language, and dates are
              matched across extracted files. Every signal links back to a document and source offset.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-[9px] text-slate-500">
              Minimum evidence
              <select
                value={minimumEvidenceCount}
                onChange={(event) => setMinimumEvidenceCount(Number(event.target.value))}
                className="bg-transparent font-mono text-slate-200 outline-none"
              >
                {[2, 3, 5, 10].map((value) => <option key={value} value={value}>{value}</option>)}
              </select>
            </label>
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-[9px] text-slate-500">
              Confidence
              <select
                value={minimumConfidence}
                onChange={(event) => setMinimumConfidence(Number(event.target.value))}
                className="bg-transparent font-mono text-slate-200 outline-none"
              >
                <option value={0}>Any</option>
                <option value={7000}>70%+</option>
                <option value={8000}>80%+</option>
                <option value={9000}>90%+</option>
                <option value={9500}>95%+</option>
              </select>
            </label>
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-[9px] text-slate-500">
              Indexed after
              <input type="datetime-local" value={indexedFrom} max={indexedTo || undefined} onChange={(event) => setIndexedFrom(event.target.value)} className="bg-transparent font-mono text-slate-200 outline-none" />
            </label>
            <label className="secondary-action flex items-center gap-2 rounded-xl border px-3 py-2 text-[9px] text-slate-500">
              Indexed before
              <input type="datetime-local" value={indexedTo} min={indexedFrom || undefined} onChange={(event) => setIndexedTo(event.target.value)} className="bg-transparent font-mono text-slate-200 outline-none" />
            </label>
            <button
              type="button"
              onClick={() => void refresh()}
              disabled={isLoading}
              className="primary-action focus-ring flex items-center gap-2 rounded-xl border border-transparent px-3 py-2 text-[9px] font-semibold text-white transition hover:brightness-105 disabled:opacity-50"
            >
              <Icon name="sync" className={cn('h-3.5 w-3.5', isLoading && 'animate-spin')} />
              Refresh index
            </button>
          </div>
        </header>

        <div className="instrument-strip app-panel grid grid-cols-2 overflow-hidden sm:grid-cols-4">
          <IndexMetric label="Documents" value={status?.indexedDocumentCount} />
          <IndexMetric label="Segments" value={status?.indexedSegmentCount} />
          <IndexMetric label="Term postings" value={status?.indexedTermCount} />
          <IndexMetric label="Writer queue" value={status?.availableWriteCapacity} suffix=" free" />
        </div>

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
          <div role="alert" className="rounded-xl border border-rose-400/20 bg-rose-500/[0.07] px-4 py-3 text-[10px] text-rose-200">
            {error}
          </div>
        )}

        <div className="grid min-h-[500px] flex-1 grid-cols-1 gap-4 xl:grid-cols-[minmax(360px,0.8fr)_minmax(0,1.2fr)]">
          <section className="app-panel min-h-0 overflow-hidden">
            <div className="border-b border-slate-800 px-4 py-3 text-[10px] font-semibold text-slate-300">
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
                    setExcerpt(null);
                  }}
                  className={cn(
                    'focus-ring block w-full border-b border-slate-800/70 px-4 py-3 text-left transition hover:bg-slate-900/70',
                    selectedPattern?.patternId === pattern.patternId && 'bg-cyan-400/[0.06] ring-1 ring-inset ring-cyan-400/15',
                  )}
                >
                  <div className="flex items-center justify-between gap-3">
                    <span className="rounded-md border border-slate-700 bg-slate-950 px-1.5 py-0.5 text-[8px] font-semibold uppercase tracking-wider text-slate-400">
                      {patternLabels[pattern.kind]}
                    </span>
                    <span className="font-mono text-[8px] text-cyan-300/70">
                      {(pattern.confidenceBasisPoints / 100).toFixed(0)}% · {pattern.evidence.length} files
                    </span>
                  </div>
                  <h3 className="mt-2 text-[11px] font-semibold leading-snug text-slate-200">{pattern.title}</h3>
                  <p className="mt-1 line-clamp-2 text-[9px] leading-relaxed text-slate-600">{pattern.explanation}</p>
                </button>
              ))}
            </div>
          </section>

          <section className="app-panel min-h-0 overflow-hidden">
            {selectedPattern ? (
              <div className="flex h-full min-h-0 flex-col">
                <div className="border-b border-slate-800 p-4">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <span className="app-eyebrow text-indigo-300/70">{patternLabels[selectedPattern.kind]}</span>
                    <span className="font-mono text-[8px] text-slate-600">{selectedPattern.algorithmVersion}</span>
                  </div>
                  <h2 className="mt-2 text-base font-semibold text-slate-100">{selectedPattern.title}</h2>
                  <p className="mt-1 text-[10px] leading-relaxed text-slate-500">{selectedPattern.explanation}</p>
                </div>
                <div className="min-h-0 flex-1 overflow-y-auto p-4">
                  <div className="mb-2 text-[9px] font-bold uppercase tracking-[0.16em] text-slate-600">Source evidence</div>
                  <div className="space-y-2">
                    {selectedPattern.evidence.map((evidence) => {
                      const key = `${evidence.portalKey}:${evidence.documentId}`;
                      return (
                        <button
                          key={`${key}:${evidence.characterStart ?? 'document'}`}
                          type="button"
                          onClick={() => void openEvidence(evidence)}
                          className="focus-ring w-full rounded-xl border border-slate-800 bg-slate-950/45 p-3 text-left transition hover:border-indigo-400/25 hover:bg-indigo-500/[0.04]"
                        >
                          <div className="flex items-center justify-between gap-3">
                            <span className="font-mono text-[9px] text-slate-300">
                              {evidence.portalKey} / request {evidence.requestId} / document {evidence.documentId}
                            </span>
                            <span className="text-[8px] font-semibold text-indigo-300">
                              {excerptLoadingKey === key ? 'Loading…' : 'Open excerpt'}
                            </span>
                          </div>
                          <p className="mt-1.5 text-[9px] text-slate-500">{evidence.evidenceLabel}</p>
                          <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 font-mono text-[8px] text-slate-700">
                            {evidence.characterStart !== null && <span>chars {evidence.characterStart}–{evidence.characterEnd}</span>}
                            <span title={evidence.contentSha256}>sha256 {evidence.contentSha256.slice(0, 16)}…</span>
                          </div>
                        </button>
                      );
                    })}
                  </div>
                  {excerpt && (
                    <div className="mt-4 rounded-xl border border-cyan-400/20 bg-slate-950/75 p-4">
                      <div className="flex items-start justify-between gap-3">
                        <div>
                          <div className="text-[9px] font-semibold text-cyan-200">Masked source excerpt</div>
                          <div className="mt-0.5 font-mono text-[8px] text-slate-600">
                            {excerpt.chunk.portalKey} / document {excerpt.chunk.documentId} · offset {excerpt.chunk.characterOffset}
                          </div>
                        </div>
                        <button type="button" onClick={() => setExcerpt(null)} aria-label="Close excerpt" className="text-slate-600 hover:text-slate-300">
                          <Icon name="x" className="h-3.5 w-3.5" />
                        </button>
                      </div>
                      <pre className="mt-3 max-h-72 overflow-auto whitespace-pre-wrap break-words font-mono text-[9px] leading-relaxed text-slate-400">
                        {excerpt.chunk.maskedText}
                      </pre>
                      <div className="mt-2 text-[8px] text-slate-700">
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
      <div className="numeric-value mt-1 text-sm font-semibold text-slate-200">
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
        'focus-ring flex items-center gap-2 rounded-lg border px-2.5 py-1.5 text-[9px] font-medium transition',
        selected
          ? 'border-cyan-400/25 bg-cyan-400/[0.08] text-cyan-200'
          : 'border-slate-800 bg-slate-950/35 text-slate-500 hover:text-slate-300',
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
        <Icon name="sparkles" className="mx-auto h-5 w-5 text-cyan-300/70" />
        <p className="mt-3 text-[10px] font-medium text-slate-500">{title}</p>
      </div>
    </div>
  );
}
