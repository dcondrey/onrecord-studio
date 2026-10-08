import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { TopicFindingBasis, TopicFindingMatch, TopicPackView, TopicPacksReport, TopicTextCoverage } from '../../types';
import { buildBriefingMarkdown, TOPIC_BASIS_LABELS, topicBriefingProject } from '../../utils/briefing';
import { ConfidenceMeter, EvidenceTrace } from '../ui/EvidenceBasis';
import VerifiableReportExport from './VerifiableReportExport';
import { downloadText } from './textDownload';
import Button from '../ui/Button';
import { CountBadge } from '../ui/Badge';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import { cn } from '../../utils/cn';
import { Meter } from '../charts/marks/Meter';
import { SourceCaption } from '../charts/frame/SourceCaption';
import ChartFrame from '../charts/frame/ChartFrame';
import { Bar } from '../charts/marks/Bar';
import { MarkGroup, MUTED_LABEL_CLASS, LABEL_CLASS } from '../charts/marks/shared';
import { measured } from '../charts/core/nulls';
import { scaleLinear } from '../charts/core/scale';
import { VIZ } from '../charts/core/claim';
import { LegendSwatch, type SwatchKind } from '../charts/frame/Legend';
import { fitsInside, measureText } from '../charts/core/layout';
import { useRovingFocus } from '../charts/hooks/useRovingFocus';

/** `export_verifiable_report` accepts at most this many selected findings. */
const REPORT_SELECTION_LIMIT = 200;

const COVERAGE_LABELS: Record<TopicTextCoverage, string> = {
  FULL: 'full text read',
  PREFIX: 'opening of the text read',
  FILE_NAME_ONLY: 'no stored text; file name read',
};

/**
 * Records tagged by topic vocabularies, and the findings whose subject is one of those records.
 * A tag is the record's own wording, not a finding.
 */
export default function TopicPacks() {
  const [report, setReport] = useState<TopicPacksReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const gate = useLatestRequestGate();
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const findingsRefreshRequests = useAppStore((state) => state.findingsRefreshRequests);

  useEffect(() => {
    const request = gate.begin();
    setError(null);
    invokeCommand('list_topic_packs', {})
      .then((value) => {
        if (!gate.isCurrent(request)) return;
        setReport(value);
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        setError(formatNativeError(reason));
      });
    return gate.invalidate;
  }, [findingsRefreshRequests, gate]);

  if (error) return <InlineError summary="Topic packs couldn’t be read." detail={error} />;
  if (!report) return <Skeleton variant="lines" lines={3} label="Reading records for topic cues" />;

  const topic = report.topics.find((candidate) => candidate.id === selectedId) ?? report.topics[0];
  const selectedIndex = Math.max(0, report.topics.findIndex((candidate) => candidate.id === topic?.id));
  const onTabKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const last = report.topics.length - 1;
    const next = event.key === 'ArrowRight' ? (selectedIndex === last ? 0 : selectedIndex + 1)
      : event.key === 'ArrowLeft' ? (selectedIndex === 0 ? last : selectedIndex - 1)
      : event.key === 'Home' ? 0
      : event.key === 'End' ? last
      : null;
    if (next === null || last < 0) return;
    event.preventDefault();
    setSelectedId(report.topics[next].id);
    tabRefs.current[next]?.focus();
  };
  return (
    <section aria-label="Topics" className="space-y-4 text-body text-fg-secondary">
      <ReadingDepth report={report} />
      <PackBars report={report} selectedId={topic?.id ?? null} onSelect={setSelectedId} />
      <div role="tablist" aria-label="Topic" onKeyDown={onTabKeyDown} className="flex gap-6 overflow-x-auto border-b border-line">
        {report.topics.map((candidate, index) => {
          const selected = candidate.id === topic?.id;
          return (
            <button
              key={candidate.id}
              ref={(node) => { tabRefs.current[index] = node; }}
              id={`topic-tab-${candidate.id}`}
              type="button"
              role="tab"
              aria-selected={selected}
              aria-controls="topic-panel"
              tabIndex={selected ? 0 : -1}
              onClick={() => setSelectedId(candidate.id)}
              className={cn(
                'focus-ring -mb-px inline-flex items-center gap-1.5 whitespace-nowrap rounded-t-sm border-b-2 px-0.5 pb-2.5 text-control transition-colors duration-fast',
                selected ? 'border-accent text-fg' : candidate.findingTotal === 0 ? 'border-transparent text-muted hover:text-fg-secondary' : 'border-transparent text-fg-secondary hover:text-fg',
              )}
            >
              {candidate.title}
              <CountBadge value={candidate.findingTotal} aria-label={`${candidate.findingTotal} finding${candidate.findingTotal === 1 ? '' : 's'}`} />
            </button>
          );
        })}
      </div>
      {topic && <TopicDetail topic={topic} report={report} />}
    </section>
  );
}

function TopicDetail({ topic, report }: { topic: TopicPackView; report: TopicPacksReport }) {
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setSelectedRequest = useAppStore((state) => state.setSelectedRequest);
  const portals = useAppStore((state) => state.portals);
  const portalName = (portalKey: string) => portals.find(({ registration }) => registration.portalKey === portalKey)?.registration.displayName ?? portalKey;
  const openRequest = (portalKey: string, requestId: number) => {
    setActivePortal(portalKey);
    setSearchFilters('', null, null, null);
    setSelectedRequest(requestId, portalKey);
    setWorkspaceView('evidence');
  };
  const exportMarkdown = () =>
    downloadText(`onrecord-topic-${topic.id}.md`, buildBriefingMarkdown(topicBriefingProject(topic)), 'text/markdown;charset=utf-8');
  const reportIds = topic.findings.slice(0, REPORT_SELECTION_LIMIT).map((match) => match.finding.id);

  return (
    <div role="tabpanel" id="topic-panel" aria-labelledby={`topic-tab-${topic.id}`} className="space-y-5">
      <p className="text-caption text-muted">Cues: {topic.cues.join(', ')}</p>

      <div>
        <h4 className="font-medium text-fg">Findings about tagged records <span className="tabular-nums text-muted">{topic.findingTotal}</span></h4>
        {topic.findings.length === 0 ? (
          <p className="mt-1 text-muted">
            No active finding has a tagged document or request as its subject, among {report.documentsScanned.toLocaleString()} documents and {report.requestsScanned.toLocaleString()} requests scanned.
          </p>
        ) : (
          <>
            {topic.findingTotal > topic.findings.length && <p className="mt-1 text-caption tabular-nums text-muted">Top {topic.findings.length} of {topic.findingTotal} findings</p>}
            <ul className="mt-1 space-y-2" aria-label={`${topic.title} findings`}>
              {topic.findings.map((match) => <TopicFindingRow key={match.finding.id} match={match} />)}
            </ul>
            <div className="mt-2 flex flex-wrap items-start gap-2">
              <Button size="sm" onClick={exportMarkdown}>Export topic briefing (Markdown)</Button>
            </div>
            {isTauri() && (
              <div className="mt-2">
                {topic.findingTotal > reportIds.length && <p className="mb-1 text-muted">The verifiable report holds the {reportIds.length} highest-ranked of this topic&apos;s {topic.findingTotal} findings.</p>}
                <VerifiableReportExport findingIds={reportIds} />
              </div>
            )}
          </>
        )}
      </div>

      <div>
        <h4 className="font-medium text-fg">Tagged documents <span className="tabular-nums text-muted">{topic.documentTotal}</span></h4>
        {topic.documentTotal > topic.documents.length && <p className="text-caption tabular-nums text-muted">Showing {topic.documents.length} of {topic.documentTotal}</p>}
        <ul className="mt-1 space-y-1">
          {topic.documents.map((document) => (
            <li key={`${document.portalKey}-${document.documentId}`} className="flex items-center justify-between gap-3 rounded-md px-2 py-1.5 hover:bg-raised/60">
              <span className="min-w-0">
                <span className="block truncate text-fg">{document.fileName ?? `Document ${document.documentId}`}</span>
                <span className="block truncate text-caption text-muted" title={`Document ${document.documentId}`}>{portalName(document.portalKey)} · Request #{document.requestId} · {COVERAGE_LABELS[document.textCoverage]} · matched: {document.matchedCues.join(', ')}</span>
              </span>
              <Button size="sm" variant="ghost" onClick={() => openRequest(document.portalKey, document.requestId)}>Open request</Button>
            </li>
          ))}
        </ul>
      </div>

      <div>
        <h4 className="font-medium text-fg">Tagged requests <span className="tabular-nums text-muted">{topic.requestTotal}</span></h4>
        {topic.requestTotal > topic.requests.length && <p className="text-caption tabular-nums text-muted">Showing {topic.requests.length} of {topic.requestTotal}</p>}
        <ul className="mt-1 space-y-1">
          {topic.requests.map((request) => (
            <li key={`${request.portalKey}-${request.requestId}`} className="flex items-center justify-between gap-3 rounded-md px-2 py-1.5 hover:bg-raised/60">
              <span className="min-w-0">
                <span className="block truncate text-fg">{request.title ?? `Request ${request.requestId}`}</span>
                <span className="block truncate text-caption text-muted">{portalName(request.portalKey)} · Request #{request.requestId} · matched: {request.matchedCues.join(', ')}</span>
              </span>
              <Button size="sm" variant="ghost" onClick={() => openRequest(request.portalKey, request.requestId)}>Open request</Button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

function TopicFindingRow({ match }: { match: TopicFindingMatch }) {
  const { finding, basis } = match;
  return (
    <li className="rounded-lg border border-line p-3" data-basis={basis}>
      <p className="font-medium text-fg">{finding.headline}</p>
      <div className="mt-1 flex flex-wrap items-center gap-2">
        <span className="inline-flex h-5 items-center rounded-full border border-line bg-raised px-2 text-caption font-medium">{TOPIC_BASIS_LABELS[basis]}</span>
        {finding.confidence.kind === 'MEASURED' && <ConfidenceMeter confidence={finding.confidence} />}
      </div>
      <div className="mt-1">
        <EvidenceTrace label={finding.headline} explanation={finding.explanation} citations={finding.citations} />
      </div>
    </li>
  );
}

const count = (n: number) => n.toLocaleString('en-US');

/**
 * F16, how deeply the scanned documents were read: a tag can only come from the text that was read,
 * so a shallow read bounds what an absent tag means.
 */
function ReadingDepth({ report }: { report: TopicPacksReport }) {
  const full = Math.max(0, report.documentsScanned - report.documentsPrefixOnly - report.documentsFileNameOnly);
  const prefix = `the first ${count(report.textPrefixCharacters)} characters`;
  const limits = [report.documentLimitReached && 'the document scan limit was reached', report.requestLimitReached && 'the request scan limit was reached'].filter(Boolean).join('; ');
  const headline = `${count(full)} of ${count(report.documentsScanned)} scanned documents were read in full; ${count(report.documentsPrefixOnly)} to ${prefix} only; ${count(report.documentsFileNameOnly)} by file name only.`;
  return (
    <figure className="m-0 grid gap-2" aria-label="Reading depth">
      <figcaption className="text-body text-fg">{headline}</figcaption>
      <Meter
        label="Reading depth of scanned documents"
        max={Math.max(1, report.documentsScanned)}
        valueText={headline}
        segments={[
          { key: 'full', value: full, step: 4, text: `${count(full)} read in full` },
          { key: 'prefix', value: report.documentsPrefixOnly, step: 2, text: `${count(report.documentsPrefixOnly)} read to ${prefix}` },
          { key: 'name', value: report.documentsFileNameOnly, step: 1, text: `${count(report.documentsFileNameOnly)} by file name only (no stored text)` },
        ]}
      />
      <p className="text-caption text-muted">
        Also scanned: {count(report.requestsScanned)} requests.{limits && ` Note: ${limits}.`} An untagged record may still concern a topic in words these lists do not hold.
      </p>
      <SourceCaption sources={{ tables: ['documents', 'requests'] }} />
    </figure>
  );
}

const BASIS_ORDER: TopicFindingBasis[] = ['COUNTED', 'STATED', 'PATTERN'];
const BASIS_SHORT: Record<TopicFindingBasis, string> = { COUNTED: 'counted', STATED: 'stated', PATTERN: 'pattern' };

/** Findings per basis, only when every finding is loaded: a truncated page is not a total. */
function basisCounts(topic: TopicPackView): Record<TopicFindingBasis, number> | null {
  if (topic.findings.length !== topic.findingTotal) return null;
  const counts: Record<TopicFindingBasis, number> = { COUNTED: 0, STATED: 0, PATTERN: 0 };
  for (const match of topic.findings) counts[match.basis] += 1;
  return counts;
}

/**
 * Each basis's glyph, shared by the plot and the legend. Only a counted finding above zero is
 * crimson; a zero draws the neutral flag outline. Pattern's dashed line is the comparison dash,
 * distinct from both absence-slot dashes.
 */
const BASIS_GLYPH: Record<TopicFindingBasis, SwatchKind> = { COUNTED: 'flag', STATED: 'hollow', PATTERN: 'dashed-line' };

function BasisGlyph({ basis, n }: { basis: TopicFindingBasis; n: number }) {
  if (basis === 'COUNTED' && n === 0) {
    return <path d="M4 11V1.5h8l-2 2.5 2 2.5H5.5" style={{ fill: 'none', stroke: VIZ.mark, strokeWidth: 1, strokeLinejoin: 'round' }} />;
  }
  return <LegendSwatch kind={BASIS_GLYPH[basis]} tone={basis === 'COUNTED' ? 'counted' : 'mark'} />;
}

/**
 * F16, packs ranked by tagged documents, with each pack's findings split by basis. What the bars
 * plot is a descriptive count of tagged records, so the frame carries no claim tag; the basis split
 * is shown only where every finding is loaded, because a page is not a total.
 * Activating a row selects that pack.
 */
function PackBars({ report, selectedId, onSelect }: { report: TopicPacksReport; selectedId: string | null; onSelect(id: string): void }) {
  const packs = [...report.topics].sort((a, b) => b.documentTotal - a.documentTotal || a.title.localeCompare(b.title));
  const top = packs[0];
  const withFindings = packs.filter((pack) => pack.findingTotal > 0).length;
  const headline = top ? `${top.title} tags the most documents (${count(top.documentTotal)}); ${withFindings} of ${packs.length} packs ${withFindings === 1 ? 'has' : 'have'} a finding about a tagged record.` : null;
  return (
    <ChartFrame
      title="Topic packs by tagged documents"
      storageKey="topic-pack-bars"
      headline={headline}
      sources={{ tables: ['documents', 'requests', 'findings'] }}
      summary={`${packs.length} packs. Findings are split by basis only where every finding is loaded.`}
      status={packs.length === 0 ? 'empty' : 'ready'}
      emptyText="No topic pack is installed."
      table={{
        caption: 'Topic packs',
        columns: [
          { key: 'pack', label: 'Pack' },
          { key: 'documents', label: 'Tagged documents', kind: 'count' },
          { key: 'requests', label: 'Tagged requests', kind: 'count' },
          { key: 'findings', label: 'Findings by basis' },
        ],
        rows: packs.map((pack) => {
          const counts = basisCounts(pack);
          return {
            pack: pack.title,
            documents: pack.documentTotal,
            requests: pack.requestTotal,
            findings: counts ? BASIS_ORDER.map((basis) => `${counts[basis]} ${TOPIC_BASIS_LABELS[basis].toLowerCase()}`).join('; ') : `first ${pack.findings.length} of ${pack.findingTotal} loaded; not totalled by basis`,
          };
        }),
      }}
      legend={[
        { key: 'documents', label: 'Tagged documents', swatch: 'rect', tone: 'mark' },
        { key: 'counted', label: 'Counted finding', swatch: BASIS_GLYPH.COUNTED, tone: 'counted' },
        { key: 'stated', label: 'Stated finding', swatch: BASIS_GLYPH.STATED, tone: 'mark' },
        { key: 'pattern', label: 'Pattern finding', swatch: BASIS_GLYPH.PATTERN, tone: 'mark' },
      ]}
      minHeight={packs.length * 30 + 8}
      interactive={{ roledescription: 'ranked bars' }}
    >
      {({ width }) => <PackPlot width={width} packs={packs} selectedId={selectedId} onSelect={onSelect} />}
    </ChartFrame>
  );
}

function PackPlot({ width, packs, selectedId, onSelect }: { width: number; packs: TopicPackView[]; selectedId: string | null; onSelect(id: string): void }) {
  const roving = useRovingFocus({ shape: { rows: packs.length }, onActivate: (i) => onSelect(packs[i].id) });
  const nameWidth = width < 480 ? Math.round(width * 0.4) : Math.min(240, Math.round(width * 0.3));
  const findingsWidth = Math.min(260, Math.round(width * 0.36));
  const x = scaleLinear([0, Math.max(1, ...packs.map((pack) => pack.documentTotal))], [nameWidth, width - findingsWidth - 48], { nice: false });
  return (
    <g>
      {packs.map((pack, index) => {
        const y = index * 30 + 4;
        const counts = basisCounts(pack);
        const name = `${pack.title}: ${count(pack.documentTotal)} tagged documents; ${counts ? BASIS_ORDER.map((basis) => `${counts[basis]} ${BASIS_SHORT[basis]}`).join(', ') : `first ${pack.findings.length} of ${pack.findingTotal} findings loaded`}.${pack.id === selectedId ? ' Selected.' : ''}`;
        let fx = width - findingsWidth;
        return (
          <MarkGroup key={pack.id} box={{ x: 0, y, width, height: 22 }} interaction={{ name, item: roving.getItemProps(index), onClick: () => onSelect(pack.id) }}>
            <text x={0} y={y + 11} dy="0.32em" className={pack.id === selectedId ? 'fill-fg text-[12px] font-semibold' : 'fill-fg-secondary text-[12px]'}>
              {fitsInside(pack.title, nameWidth) ? pack.title : `${pack.title.slice(0, Math.max(6, Math.floor(nameWidth / 7) - 1))}…`}
            </text>
            <Bar band={{ pos: y + 4, thickness: 14 }} value={measured(pack.documentTotal)} scale={x} label={`${count(pack.documentTotal)} tagged`} />
            {counts ? BASIS_ORDER.map((basis) => {
              const at = fx;
              const label = `${counts[basis]} ${BASIS_SHORT[basis]}`;
              fx += 16 + measureText(label) + 12;
              return (
                <g key={basis} data-basis={basis}>
                  <g transform={`translate(${at - 2} ${y + 6})`}>
                    <BasisGlyph basis={basis} n={counts[basis]} />
                  </g>
                  <text x={at + 16} y={y + 12} dy="0.32em" className={LABEL_CLASS}>{label}</text>
                </g>
              );
            }) : (
              <text x={fx} y={y + 12} dy="0.32em" className={MUTED_LABEL_CLASS}>{`first ${pack.findings.length} of ${pack.findingTotal} findings`}</text>
            )}
          </MarkGroup>
        );
      })}
      <line x1={x(0) + 0.5} x2={x(0) + 0.5} y1={0} y2={packs.length * 30} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
    </g>
  );
}
