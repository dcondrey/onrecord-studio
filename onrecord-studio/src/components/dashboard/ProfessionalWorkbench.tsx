import { useEffect, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { ResearchReport } from '../../types';
import CaseworkPanel from './CaseworkPanel';
import FinancialInvestigationsPanel from './FinancialInvestigationsPanel';
import MonitoringPanel from './MonitoringPanel';
import EvidenceReviewPanel from './EvidenceReviewPanel';
import InvestigationEvaluationPanel from './InvestigationEvaluationPanel';
import PackageVerifier from './PackageVerifier';

const tabs = ['Cases and follow-through', 'Spending and outcomes', 'Coverage and changes', 'Evidence review', 'Usefulness'] as const;
type Tab = typeof tabs[number];
export default function ProfessionalWorkbench({ initialReportId = '', initialTab = tabs[0] }: { initialReportId?: string; initialTab?: Tab } = {}) {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [reports, setReports] = useState<ResearchReport[]>([]);
  const [reportId, setReportId] = useState(initialReportId);
  const [caseContext, setCaseContext] = useState<{ id: string|null; topic: string; reportId?: string }>({ id: null, topic: 'homelessness' });
  const [error, setError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const gate = useLatestRequestGate();
  useEffect(() => {
    if (!isTauri()) return;
    const request = gate.begin();
    void invokeCommand('get_research_status', {}).then(status => {
      if (!gate.isCurrent(request)) return;
      if (!Array.isArray(status.reports)) throw new Error("The native research catalog returned an invalid response.");
      setReports(status.reports); setError('');
      setReportId(current => current || status.reports[0]?.id || '');
    }).catch(reason => { if (gate.isCurrent(request)) setError(String(reason)); });
    return gate.invalidate;
  }, [gate, refresh]);
  return <section className="app-panel p-4 sm:p-5" aria-label="Investigation workbench">
    <div className="mb-4"><div className="app-eyebrow">Investigation workbench</div><h2 className="mt-1 text-xl font-semibold text-slate-100">Turn discoveries into a documented case</h2></div>
    <div role="tablist" aria-label="Investigation tools" className="mb-5 flex flex-wrap gap-2">{tabs.map(value => <button type="button" key={value} role="tab" aria-selected={value === tab} className={`rounded-lg border px-3 py-2 text-label leading-4 ${value === tab ? 'border-cyan-400/40 bg-cyan-400/10 text-cyan-100' : 'border-slate-700 text-slate-400'}`} onClick={() => { setTab(value); if (value === 'Evidence review') setRefresh(n => n + 1); }}>{value}</button>)}</div>
    <div className="mb-5"><PackageVerifier /></div>
    <div role="tabpanel" aria-label={tab}>
      {tab === tabs[0] && <CaseworkPanel onSelectCase={(id, topic, selectedReportId) => { setCaseContext(current => current.id === id && current.topic === topic && current.reportId === selectedReportId ? current : { id, topic, reportId: selectedReportId }); if (selectedReportId) setReportId(selectedReportId); }} />}
      {tab === tabs[1] && <FinancialInvestigationsPanel />}
      {tab === tabs[2] && <><div className="mb-4 flex flex-wrap items-center gap-3"><label className="text-label leading-4 text-slate-300">Topic<select className="analysis-input ml-2" value={caseContext.topic} onChange={e => setCaseContext({ id: null, topic: e.target.value })}>{['homelessness','police','abatement','bathrooms','waste'].map(topic => <option key={topic}>{topic}</option>)}</select></label><p className="text-label leading-4 text-slate-400">{caseContext.id ? 'Coverage linked to the case selected in Cases and follow-through.' : 'Topic-wide coverage. Select a case to link new expectations to it.'}</p></div><MonitoringPanel caseId={caseContext.id} topic={caseContext.topic} /></>}
      {tab === tabs[3] && <>{error && <p role="alert" className="text-sm text-rose-300">{error}</p>}<div className="mb-4 flex flex-wrap gap-3"><label className="min-w-0 flex-1 text-label leading-4 text-slate-300">Investigation<select className="analysis-input mt-1 w-full" value={reportId} onChange={e => setReportId(e.target.value)}><option value="">Select a saved investigation</option>{reportId && !reports.some(report => report.id === reportId) && <option value={reportId}>Original dossier of selected case</option>}{reports.map(report => <option key={report.id} value={report.id}>{report.synthesis.title}</option>)}</select></label><button className="analysis-button" onClick={() => setRefresh(n => n + 1)}>Refresh investigations</button></div>{reportId ? <EvidenceReviewPanel reportId={reportId} /> : <p className="text-sm text-slate-400">A saved investigation is needed for evidence review. Autonomous research creates dossiers from collected public records.</p>}</>}
      {tab === tabs[4] && <InvestigationEvaluationPanel />}
    </div>
  </section>;
}
