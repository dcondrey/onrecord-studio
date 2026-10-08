import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { ResearchReport } from '../../types';
import { INVESTIGATION_TOPICS } from '../../utils/investigations';
import { cn } from '../../utils/cn';
import CaseworkPanel from './CaseworkPanel';
import FinancialInvestigationsPanel from './FinancialInvestigationsPanel';
import OrganisationFilingsPanel from './OrganisationFilingsPanel';
import MonitoringPanel from './MonitoringPanel';
import EvidenceReviewPanel from './EvidenceReviewPanel';
import InvestigationEvaluationPanel from './InvestigationEvaluationPanel';
import PackageVerifier from './PackageVerifier';
import { Field, Select } from '../ui/Field';
import InlineError from '../ui/InlineError';
import Icon from '../ui/icons';

const tabs = ['Cases and follow-through', 'Spending and outcomes', 'Coverage and changes', 'Evidence review', 'Organization filings'] as const;
type Tab = typeof tabs[number];

const COVERAGE_TOPICS = ['homelessness', 'police', 'abatement', 'bathrooms', 'waste'] as const;
const TOPIC_LABEL_OVERRIDES: Record<string, string> = { bathrooms: 'Public restrooms', waste: 'Waste collection' };
const topicLabel = (id: string) => TOPIC_LABEL_OVERRIDES[id] ?? INVESTIGATION_TOPICS.find((topic) => topic.id === id)?.title ?? id;

export default function ProfessionalWorkbench({ initialReportId = '', initialTab = tabs[0] }: { initialReportId?: string; initialTab?: Tab } = {}) {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [organisationQuery, setOrganisationQuery] = useState('');
  const [reports, setReports] = useState<ResearchReport[]>([]);
  const [reportId, setReportId] = useState(initialReportId);
  const [caseContext, setCaseContext] = useState<{ id: string|null; topic: string; reportId?: string }>({ id: null, topic: 'homelessness' });
  const [error, setError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const [assessmentOpened, setAssessmentOpened] = useState(false);
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);
  const gate = useLatestRequestGate();
  useEffect(() => {
    if (!isTauri()) return;
    const request = gate.begin();
    void invokeCommand('get_research_status', {}).then(status => {
      if (!gate.isCurrent(request)) return;
      if (!Array.isArray(status.reports)) throw new Error("The native research catalog returned an invalid response.");
      setReports(status.reports); setError('');
      setReportId(current => current || status.reports[0]?.id || '');
    }).catch(reason => { if (gate.isCurrent(request)) setError(formatNativeError(reason)); });
    return gate.invalidate;
  }, [gate, refresh]);

  const select = (value: Tab) => {
    setTab(value);
    // Opening the review re-reads the catalog, so a dossier saved elsewhere is listed.
    if (value === 'Evidence review') setRefresh(n => n + 1);
  };

  const onTabKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const index = tabs.indexOf(tab);
    const last = tabs.length - 1;
    const next = event.key === 'ArrowRight' ? (index === last ? 0 : index + 1)
      : event.key === 'ArrowLeft' ? (index === 0 ? last : index - 1)
      : event.key === 'Home' ? 0
      : event.key === 'End' ? last
      : null;
    if (next === null) return;
    event.preventDefault();
    select(tabs[next]);
    tabRefs.current[next]?.focus();
  };

  return <section className="app-panel" aria-label="Investigation workbench">
    <div role="tablist" aria-label="Investigation tools" onKeyDown={onTabKeyDown} className="flex gap-6 overflow-x-auto border-b border-line px-5 pt-3">
      {tabs.map((value, index) => (
        <button
          type="button"
          key={value}
          ref={(node) => { tabRefs.current[index] = node; }}
          id={`wb-tab-${index}`}
          role="tab"
          aria-selected={value === tab}
          aria-controls="wb-panel"
          tabIndex={value === tab ? 0 : -1}
          onClick={() => select(value)}
          className={cn(
            'focus-ring -mb-px whitespace-nowrap rounded-t-sm border-b-2 px-0.5 pb-2.5 pt-1 text-control transition-colors duration-fast',
            value === tab ? 'border-accent text-fg' : 'border-transparent text-muted hover:text-fg-secondary',
          )}
        >
          {value}
        </button>
      ))}
    </div>
    <div role="tabpanel" id="wb-panel" aria-labelledby={`wb-tab-${tabs.indexOf(tab)}`} tabIndex={0} className="p-5 outline-none">
      {tab === tabs[0] && <CaseworkPanel onSelectCase={(id, topic, selectedReportId) => { setCaseContext(current => current.id === id && current.topic === topic && current.reportId === selectedReportId ? current : { id, topic, reportId: selectedReportId }); if (selectedReportId) setReportId(selectedReportId); }} />}
      {tab === tabs[1] && <FinancialInvestigationsPanel onCheckOrganisation={(vendor) => { setOrganisationQuery(vendor); setTab('Organization filings'); }} />}
      {tab === tabs[2] && <>
        <div className="mb-5 flex flex-wrap items-end gap-x-4 gap-y-2">
          <Field label="Topic" className="w-60">
            <Select value={caseContext.topic} onChange={e => setCaseContext({ id: null, topic: e.target.value })}>
              {COVERAGE_TOPICS.map(topic => <option key={topic} value={topic}>{topicLabel(topic)}</option>)}
            </Select>
          </Field>
          <p className="pb-1.5 text-caption text-muted">{caseContext.id ? 'Coverage linked to the case selected in Cases and follow-through.' : 'Topic-wide coverage. Select a case to link new expectations to it.'}</p>
        </div>
        <MonitoringPanel caseId={caseContext.id} topic={caseContext.topic} />
      </>}
      {tab === tabs[3] && <div className="space-y-5">
        {error && <InlineError summary="Saved investigations couldn’t be listed." detail={error} onRetry={() => setRefresh(n => n + 1)} />}
        <Field label="Investigation" className="max-w-xl">
          <Select value={reportId} onChange={e => setReportId(e.target.value)}>
            <option value="">Select a saved investigation</option>
            {reportId && !reports.some(report => report.id === reportId) && <option value={reportId}>Original dossier of selected case</option>}
            {reports.map(report => <option key={report.id} value={report.id}>{report.synthesis.title}</option>)}
          </Select>
        </Field>
        <PackageVerifier />
        {reportId ? <EvidenceReviewPanel reportId={reportId} /> : <p className="text-body text-muted">Evidence review needs a saved investigation. Research creates one from collected public records.</p>}
        <details className="group border-t border-line pt-4" onToggle={(event) => { if (event.currentTarget.open) setAssessmentOpened(true); }}>
          <summary className="focus-ring flex cursor-pointer list-none items-center gap-2 rounded text-body font-medium text-fg [&::-webkit-details-marker]:hidden">
            <Icon name="chevron" size="xs" className="text-muted transition-transform duration-base group-open:rotate-90" />
            Assess usefulness
          </summary>
          {assessmentOpened && <div className="mt-4"><InvestigationEvaluationPanel /></div>}
        </details>
      </div>}
      {tab === tabs[4] && <OrganisationFilingsPanel key={organisationQuery} initialQuery={organisationQuery} />}
    </div>
  </section>;
}
