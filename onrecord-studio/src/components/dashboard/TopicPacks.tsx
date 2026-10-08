import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { TopicFindingMatch, TopicPackView, TopicPacksReport, TopicTextCoverage } from '../../types';
import { buildBriefingMarkdown, TOPIC_BASIS_LABELS, topicBriefingProject } from '../../utils/briefing';
import { ConfidenceMeter, EvidenceTrace } from '../ui/EvidenceBasis';
import VerifiableReportExport from './VerifiableReportExport';
import { downloadText } from './textDownload';
import Button from '../ui/Button';
import { CountBadge } from '../ui/Badge';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import { cn } from '../../utils/cn';

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
      <p className="text-caption text-muted">
        Scanned {report.documentsScanned.toLocaleString()} documents{report.documentLimitReached ? ' (scan limit reached)' : ''} and {report.requestsScanned.toLocaleString()} requests{report.requestLimitReached ? ' (scan limit reached)' : ''}.
        {report.documentsPrefixOnly > 0 && ` ${report.documentsPrefixOnly.toLocaleString()} documents were read to their first ${report.textPrefixCharacters.toLocaleString()} characters only.`}
        {report.documentsFileNameOnly > 0 && ` ${report.documentsFileNameOnly.toLocaleString()} have no stored text and were matched on file name only.`}
        {' '}An untagged record may still concern a topic in words these lists do not hold.
      </p>
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
