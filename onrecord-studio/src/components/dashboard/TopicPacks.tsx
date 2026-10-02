import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { TopicFindingMatch, TopicPackView, TopicPacksReport, TopicTextCoverage } from '../../types';
import { buildBriefingMarkdown, TOPIC_BASIS_LABELS, topicBriefingProject } from '../../utils/briefing';
import { ConfidenceMeter, EvidenceTrace } from '../ui/EvidenceBasis';
import VerifiableReportExport from './VerifiableReportExport';

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

  if (error) return <div role="alert" className="text-label text-rose-300">Topics unavailable: {error}</div>;
  if (!report) return <div role="status" className="text-label text-slate-400">Reading records for topic cues…</div>;

  const topic = report.topics.find((candidate) => candidate.id === selectedId) ?? report.topics[0];
  return (
    <section aria-label="Topics" className="space-y-3 text-label text-slate-300">
      <p className="text-slate-400">
        Scanned {report.documentsScanned.toLocaleString()} documents{report.documentLimitReached ? ' (scan limit reached)' : ''} and {report.requestsScanned.toLocaleString()} requests{report.requestLimitReached ? ' (scan limit reached)' : ''}.
        {report.documentsPrefixOnly > 0 && ` ${report.documentsPrefixOnly.toLocaleString()} documents were read to their first ${report.textPrefixCharacters.toLocaleString()} characters only.`}
        {report.documentsFileNameOnly > 0 && ` ${report.documentsFileNameOnly.toLocaleString()} have no stored text and were matched on file name only.`}
        {' '}An untagged record may still concern a topic in words these lists do not hold.
      </p>
      <div role="tablist" aria-label="Topic" className="flex flex-wrap gap-1.5">
        {report.topics.map((candidate) => (
          <button
            key={candidate.id}
            type="button"
            role="tab"
            aria-selected={candidate.id === topic?.id}
            onClick={() => setSelectedId(candidate.id)}
            className="focus-ring rounded-full border border-slate-700 px-2.5 py-1 font-semibold aria-selected:border-cyan-400/60 aria-selected:text-cyan-200"
          >
            {candidate.title} · {candidate.findingTotal}
          </button>
        ))}
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
  const openRequest = (portalKey: string, requestId: number) => {
    setActivePortal(portalKey);
    setSearchFilters('', null, null, null);
    setSelectedRequest(requestId, portalKey);
    setWorkspaceView('evidence');
  };
  const exportMarkdown = () => {
    const markdown = buildBriefingMarkdown(topicBriefingProject(topic));
    const url = URL.createObjectURL(new Blob([markdown], { type: 'text/markdown;charset=utf-8' }));
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `onrecord-topic-${topic.id}.md`;
    anchor.click();
    URL.revokeObjectURL(url);
  };
  const reportIds = topic.findings.slice(0, REPORT_SELECTION_LIMIT).map((match) => match.finding.id);

  return (
    <div role="tabpanel" aria-label={topic.title} className="space-y-3">
      <p className="text-slate-400">Cues: {topic.cues.join(', ')}</p>

      <div>
        <h4 className="font-semibold text-slate-200">Findings about tagged records ({topic.findingTotal})</h4>
        {topic.findings.length === 0 ? (
          <p className="mt-1 text-slate-400">
            No active finding has a tagged document or request as its subject, among {report.documentsScanned.toLocaleString()} documents and {report.requestsScanned.toLocaleString()} requests scanned.
          </p>
        ) : (
          <>
            {topic.findingTotal > topic.findings.length && <p className="mt-1 text-slate-400">Showing the {topic.findings.length} highest-ranked of {topic.findingTotal}.</p>}
            <ul className="mt-1 space-y-2" aria-label={`${topic.title} findings`}>
              {topic.findings.map((match) => <TopicFindingRow key={match.finding.id} match={match} />)}
            </ul>
            <div className="mt-2 flex flex-wrap items-start gap-2">
              <button type="button" onClick={exportMarkdown} className="focus-ring rounded border border-slate-600 px-3 py-1.5 font-semibold text-slate-100">
                Export topic briefing (Markdown)
              </button>
            </div>
            {isTauri() && (
              <div className="mt-2">
                {topic.findings.length > REPORT_SELECTION_LIMIT && <p className="mb-1 text-slate-400">The verifiable report holds the {REPORT_SELECTION_LIMIT} highest-ranked findings of this topic; more exist.</p>}
                <VerifiableReportExport findingIds={reportIds} />
              </div>
            )}
          </>
        )}
      </div>

      <div>
        <h4 className="font-semibold text-slate-200">Tagged documents ({topic.documentTotal})</h4>
        {topic.documentTotal > topic.documents.length && <p className="text-slate-400">Showing {topic.documents.length} of {topic.documentTotal}.</p>}
        <ul className="mt-1 space-y-1">
          {topic.documents.map((document) => (
            <li key={`${document.portalKey}-${document.documentId}`} className="flex flex-wrap items-center justify-between gap-2 rounded border border-slate-800 px-2 py-1">
              <span className="min-w-0">
                <span className="block truncate text-slate-200">{document.fileName ?? `Document ${document.documentId}`}</span>
                <span className="block text-slate-400">{document.portalKey} / request {document.requestId} / document {document.documentId} · {COVERAGE_LABELS[document.textCoverage]} · matched: {document.matchedCues.join(', ')}</span>
              </span>
              <button type="button" onClick={() => openRequest(document.portalKey, document.requestId)} className="focus-ring rounded border border-slate-700 px-2 py-0.5 text-cyan-200">Open request</button>
            </li>
          ))}
        </ul>
      </div>

      <div>
        <h4 className="font-semibold text-slate-200">Tagged requests ({topic.requestTotal})</h4>
        {topic.requestTotal > topic.requests.length && <p className="text-slate-400">Showing {topic.requests.length} of {topic.requestTotal}.</p>}
        <ul className="mt-1 space-y-1">
          {topic.requests.map((request) => (
            <li key={`${request.portalKey}-${request.requestId}`} className="flex flex-wrap items-center justify-between gap-2 rounded border border-slate-800 px-2 py-1">
              <span className="min-w-0">
                <span className="block truncate text-slate-200">{request.title ?? `Request ${request.requestId}`}</span>
                <span className="block text-slate-400">{request.portalKey} / request {request.requestId} · matched: {request.matchedCues.join(', ')}</span>
              </span>
              <button type="button" onClick={() => openRequest(request.portalKey, request.requestId)} className="focus-ring rounded border border-slate-700 px-2 py-0.5 text-cyan-200">Open request</button>
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
    <li className="rounded-lg border border-slate-800 p-2" data-basis={basis}>
      <p className="font-semibold text-slate-100">{finding.headline}</p>
      <div className="mt-1 flex flex-wrap items-center gap-2">
        <span className="rounded-full border border-slate-600 px-1.5 py-0.5 text-caption font-semibold">{TOPIC_BASIS_LABELS[basis]}</span>
        {finding.confidence.kind === 'MEASURED' && <ConfidenceMeter confidence={finding.confidence} />}
      </div>
      <div className="mt-1">
        <EvidenceTrace label={finding.headline} explanation={finding.explanation} citations={finding.citations} />
      </div>
    </li>
  );
}
