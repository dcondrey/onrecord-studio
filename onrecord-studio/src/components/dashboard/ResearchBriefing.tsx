import { useRef, useState } from "react";
import type { ResearchReport } from "../../types";
import { briefingFindings, researchRunIssues } from "../../utils/researchBriefing";
import { INVESTIGATION_TOPICS } from "../../utils/investigations";
import { Citations } from "./ResearchEvidence";
import ResearchAgenda from "./ResearchAgenda";

export default function ResearchBriefing({ reports, states, onOpen, onReview }: {
  reports: ResearchReport[];
  states: Record<string, string>;
  onOpen: (reportId: string, section: "findings" | "requests" | "meetings", itemIndex?: number) => void;
  onReview?: (reportId: string) => void;
}) {
  const [topic, setTopic] = useState("all");
  const [selected, setSelected] = useState("");
  const [expanded, setExpanded] = useState(false);
  const leadRef = useRef<HTMLElement>(null);
  const result = briefingFindings(reports, states, topic);
  const key = (item: typeof result.findings[number]) => `${item.report.id}:${item.index}`;
  const item = result.findings.find((entry) => key(entry) === selected) ?? result.findings[0];
  const others = result.findings.filter((entry) => entry !== item);
  const topics = new Set(reports.map((report) => report.topic));
  const issues = item ? researchRunIssues(item.report) : [];
  const request = item?.requests[0];
  return <div className="space-y-4" aria-label="Investigation briefing">
    <div className="flex flex-wrap items-center justify-between gap-3">
      <h3 className="text-sm font-semibold text-slate-300">Evidence worth following</h3>
      <details className="text-label leading-4 text-slate-400">
        <summary className="cursor-pointer">{topic === "all" ? "Across your investigations" : INVESTIGATION_TOPICS.find((entry) => entry.id === topic)?.title}</summary>
        <label className="mt-2 block">Briefing topic<select aria-label="Briefing topic" className="analysis-input ml-2" value={topic} onChange={(event) => { setTopic(event.target.value); setSelected(""); setExpanded(false); }}><option value="all">Across investigations</option>{INVESTIGATION_TOPICS.filter((entry) => topics.has(entry.id)).map((entry) => <option key={entry.id} value={entry.id}>{entry.title}</option>)}</select></label>
      </details>
    </div>
    {result.omitted > 0 && <p role="alert" className="rounded-lg border border-amber-700 p-3 text-sm text-amber-200">{result.omitted} saved finding{result.omitted === 1 ? " was" : "s were"} withheld from this briefing because supporting or opposing quotations could not be matched to retained evidence. Open the original dossier to inspect the gap.</p>}
    {!item ? <div className="app-panel p-5 text-sm text-slate-300"><h4 className="font-semibold">No active, source-backed finding to feature</h4><p className="mt-2">The saved dossiers may contain unresolved questions, completed work or evidence gaps. Open a dossier below to examine what the current records establish.</p></div> : <article ref={leadRef} className="app-panel overflow-hidden p-5 sm:p-6" aria-label={item.finding.title}>
      <p className="text-label leading-4 text-cyan-300">{INVESTIGATION_TOPICS.find((entry) => entry.id === item.report.topic)?.title ?? item.report.topic} · Saved {new Date(item.report.createdAt).toLocaleDateString()} · Working finding</p>
      <h4 className="mt-3 max-w-5xl text-2xl font-semibold leading-snug tracking-tight text-slate-100">{item.finding.title}</h4>
      <div className="mt-5 grid gap-5 lg:grid-cols-[minmax(0,1.2fr)_minmax(0,1fr)]">
        <div className="min-w-0 space-y-4">
          <div><h5 className="text-sm font-semibold text-slate-100">Why this matters</h5><p className="mt-2 text-sm leading-relaxed text-slate-300">{item.finding.whyItMatters || "The practical consequence has not been established. Test that before escalating this lead."}</p></div>
          <div><h5 className="text-sm font-semibold text-amber-200">What remains unproven</h5><p className="mt-2 text-sm leading-relaxed text-slate-300">{item.finding.alternativeExplanations[0] || "No alternative explanation was recorded. Matching quotations alone does not establish the interpretation."}</p>
            <details className="mt-2 text-label leading-4 text-slate-400"><summary className="cursor-pointer text-cyan-300">Read the interpretation and competing evidence</summary><div className="mt-3 space-y-3"><p className="text-sm leading-relaxed text-slate-300">{item.finding.interpretation}</p>{item.finding.alternativeExplanations.length > 1 && <ul className="list-disc space-y-2 pl-4">{item.finding.alternativeExplanations.slice(1).map((explanation, index) => <li key={index}>{explanation}</li>)}</ul>}{item.counterEvidence.length > 0 && <div><p className="mb-2 font-semibold text-slate-300">Counter-evidence</p><Citations citations={item.counterEvidence} report={item.report} /></div>}</div></details>
          </div>
          <div><h5 className="text-sm font-semibold text-cyan-200">What would change this conclusion</h5><p className="mt-2 text-sm leading-relaxed text-slate-300">{item.finding.nextTest || "Define a test that could disprove this interpretation before advancing it."}</p></div>
        </div>
        <div className="order-first min-w-0 rounded-xl border border-cyan-700/60 bg-cyan-950/15 p-4 lg:order-last">
          <h5 className="text-sm font-semibold text-cyan-200">What to do next</h5>
          {request ? <><p className="mt-2 text-sm font-semibold text-slate-100">{request.request.title}</p><button type="button" className="primary-action focus-ring mt-3 rounded-lg px-4 py-2.5 text-sm font-semibold text-white" onClick={() => onOpen(item.report.id, "requests", request.index)}>Review targeted request</button><p className="mt-3 text-label leading-4 text-slate-400">Suggested custodian: {request.request.custodian}</p><p className="mt-2 text-sm leading-relaxed text-slate-300">{request.request.gap}</p><p className="mt-2 text-label leading-4 text-slate-400">Draft prepared from the cited records. Nothing has been sent.</p></> : <button type="button" className="primary-action focus-ring mt-3 rounded-lg px-4 py-2.5 text-sm font-semibold text-white" onClick={() => onOpen(item.report.id, "findings", item.index)}>Open the next test</button>}

        </div>
      </div>
      <div className="mt-5 border-t border-slate-700 pt-4">
        <h5 className="mb-3 text-sm font-semibold text-slate-100">Evidence in the saved record</h5>
        <div className="max-h-64 overflow-auto pr-1"><Citations citations={item.citations.slice(0, 1)} report={item.report} /></div>
        {item.citations.length > 1 && <details className="mt-3 text-label leading-4"><summary className="cursor-pointer text-cyan-300">Read {item.citations.length - 1} more supporting passages</summary><div className="mt-3 space-y-3"><Citations citations={item.citations.slice(1)} report={item.report} /></div></details>}
        {issues.length > 0 && <details className="mt-4 rounded-lg border border-amber-800/70 p-3 text-label leading-4 text-amber-200" aria-label="Limits of saved investigations"><summary className="cursor-pointer">Research limitation: {issues[0].summary}</summary><ul className="mt-3 list-disc space-y-2 pl-4">{issues.map((issue, index) => <li key={index}>{issue.summary}{issue.details.length > 0 && <details className="mt-1"><summary className="cursor-pointer text-slate-400">Recorded attempts</summary><pre className="mt-2 max-h-44 overflow-auto whitespace-pre-wrap break-words text-slate-400">{issue.details.join("\n\n")}</pre></details>}</li>)}</ul></details>}
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-3"><button type="button" className="analysis-button" onClick={() => onOpen(item.report.id, "findings", item.index)}>Open full investigation</button>{onReview && <button type="button" className="analysis-button" onClick={() => onReview(item.report.id)}>Review evidence for publication</button>}<details className="text-label leading-4 text-slate-400"><summary className="cursor-pointer">Scope and selection</summary><p className="mt-2">Saved quotations are not a live source check or a verified interpretation. This reading order favors a recorded test, related request, counter-evidence and up to two source snapshots, with one finding per topic first.</p><ul className="mt-2 list-disc space-y-1 pl-4">{item.report.synthesis.limitations.map((limitation, index) => <li key={index}>{limitation}</li>)}</ul></details></div>
    </article>}
    {others.length > 0 && <section aria-label="Other discoveries"><h4 className="mb-3 text-sm font-semibold text-slate-200">Also worth investigating</h4><div className="grid gap-3 md:grid-cols-2">{(expanded ? others : others.slice(0, 4)).map((entry) => <button type="button" key={key(entry)} className="app-panel p-4 text-left transition hover:border-cyan-600" onClick={() => { setSelected(key(entry)); leadRef.current?.scrollIntoView?.({ block: "start" }); }}><span className="text-label leading-4 text-cyan-300">{INVESTIGATION_TOPICS.find((value) => value.id === entry.report.topic)?.title ?? entry.report.topic}</span><span className="mt-2 block text-base font-semibold leading-snug text-slate-100">{entry.finding.title}</span><span className="mt-3 block text-label leading-4 text-slate-400">Read evidence and next action →</span></button>)}</div>{others.length > 4 && <button type="button" className="analysis-button mt-3" aria-expanded={expanded} onClick={() => setExpanded((value) => !value)}>{expanded ? "Show fewer discoveries" : `Browse ${others.length - 4} more discoveries`}</button>}</section>}
    <ResearchAgenda reports={reports.filter((report) => topic === "all" || report.topic === topic)} states={states} onOpen={onOpen} meetingsOnly />
  </div>;
}
