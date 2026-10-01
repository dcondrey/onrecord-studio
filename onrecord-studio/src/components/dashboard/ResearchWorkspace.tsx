import { useEffect, useRef, useState } from "react";
import ResearchBriefing from "./ResearchBriefing";
import { Citations, EvidenceLinks } from "./ResearchEvidence";
import ResearchExport from "./ResearchExport";
import { requestFindingLinks } from "../../utils/researchExport";
import { isTauri } from "@tauri-apps/api/core";
import { invokeCommand } from "../../ipc";
import type {
  ResearchFinding,
  ResearchPolicy,
  ResearchReport,
  ResearchStatus,
} from "../../types";
import { documentBriefingEvidence, localDateInput } from "../../utils/briefing";
import { INVESTIGATION_TOPICS } from "../../utils/investigations";
import { useAppStore } from "../../store/useAppStore";
import { useLatestRequestGate } from "../../hooks/useLatestRequestGate";
import { minimumResearchCalls, researchBudgetMessage } from "../../utils/researchReadiness";

export default function ResearchWorkspace({ onReview }: { onReview?: (reportId: string) => void } = {}) {
  const [status, setStatus] = useState<ResearchStatus | null>(null);
  const [error, setError] = useState("");
  const [topic, setTopic] = useState("homelessness");
  const [selectedReport, setSelectedReport] = useState<ResearchReport | null>(null);
  const [selectedSection, setSelectedSection] = useState<"findings" | "requests" | "meetings">("findings");
  const [selectedItemIndex, setSelectedItemIndex] = useState<number | undefined>();
  const [pending, setPending] = useState(false);
  const controlsRef = useRef<HTMLDetailsElement>(null);
  const settingsRef = useRef<HTMLDetailsElement>(null);
  const gate = useLatestRequestGate();
  const workspaceRef = useRef<HTMLElement>(null);
  const refresh = async () => {
    const token = gate.begin();
    try {
      const next = await invokeCommand("get_research_status", {});
      if (gate.isCurrent(token)) { setStatus(next); setError(""); }
    } catch (reason) {
      if (gate.isCurrent(token)) setError(String(reason));
    }
  };
  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await refresh();
      if (active) timer = setTimeout(() => void poll(), 8000);
    };
    void poll();
    return () => { active = false; clearTimeout(timer); gate.invalidate(); };
    // The gate is stable; refresh is intentionally owned by this polling lifecycle.
  }, [gate]);
  // The catalog can drop removed/private reports as well as age out old entries. Never keep
  // publishing cached evidence or substitute a different dossier after that happens.
  const report = status?.reports.find((item) => item.id === selectedReport?.id);
  const open = (id: string, section: "findings" | "requests" | "meetings", itemIndex?: number) => {
    const saved = status?.reports.find((item) => item.id === id);
    if (saved) { setSelectedReport(saved); setSelectedSection(section); setSelectedItemIndex(itemIndex); workspaceRef.current?.scrollIntoView?.({ block: "start" }); }
  };
  const run = async () => {
    setPending(true); setError("");
    try { await invokeCommand("run_research_now", { topic }); await refresh(); }
    catch (reason) { setError(String(reason)); }
    finally { setPending(false); }
  };
  const missingCredential = status && status.policy.provider !== "ollama" && !status.credentialAvailable;
  const budgetMessage = status && researchBudgetMessage(status.policy, status.callsToday);
  const latestSavedAt = status?.reports.reduce<string | null>((latest, item) => Number.isFinite(Date.parse(item.createdAt)) && (!latest || Date.parse(item.createdAt) > Date.parse(latest)) ? item.createdAt : latest, null);
  const openSettings = () => {
    if (controlsRef.current) controlsRef.current.open = true;
    if (settingsRef.current) {
      settingsRef.current.open = true;
      settingsRef.current.scrollIntoView?.({ block: "nearest" });
    }
  };
  const sourceIssues = status?.sources.filter((source) => /ERROR|FAILED|BLOCKED|UNAVAILABLE|RETRY/i.test(source.state)) ?? [];
  const stage = !isTauri() ? "Local investigations are available in the desktop app."
    : !status ? error ? "Saved investigations unavailable" : "Loading saved investigations…"
    : !status.policy.enabled ? "Automatic research is paused. Saved investigations remain available."
    : missingCredential ? "Model access is unavailable. Configure a credential to run another investigation."
    : status.phase;
  return <section ref={workspaceRef} aria-label="Autonomous research" className="space-y-4">
    <div className="flex flex-wrap items-center justify-between gap-3 text-xs text-slate-400"><p role="status">{stage}</p>{status && <p className={status.policy.provider === "ollama" ? "text-emerald-300" : "text-amber-200"}>{status.policy.provider === "ollama" ? "Model: local Ollama. Nothing is sent off this computer." : `Model: ${status.policy.provider === "openrouter" ? "OpenRouter" : "OpenAI"} (external). Investigations${status.policy.enabled ? "" : " you run manually"} send selected public-record excerpts to it.`}</p>}{latestSavedAt && <p>Latest saved dossier: {new Date(latestSavedAt).toLocaleString()}</p>}</div>
    {status && (missingCredential || budgetMessage) && <div className="rounded-lg border border-amber-700 p-3 text-sm text-amber-200">
      {missingCredential && <p>Add <code>{status.policy.provider === "openrouter" ? "OPENROUTER_API_KEY" : "OPENAI_API_KEY"}</code> to <code>~/.env</code> for the selected {status.policy.provider === "openrouter" ? "OpenRouter" : "OpenAI API"} provider. The app checks this file automatically. If your key is for another provider, select that provider in Research settings.</p>}
      {budgetMessage && <p className={missingCredential ? "mt-2" : undefined}>{budgetMessage}</p>}
      <button type="button" className="analysis-button mt-2" onClick={openSettings}>Open Research settings</button>
    </div>}
    {error && <div role="alert" className="rounded-lg border border-rose-700 p-3 text-sm text-rose-300"><p>Research status could not be refreshed{status ? "; showing the last loaded saved records" : ""}.</p><p className="mt-1">{error}</p><button type="button" className="analysis-button mt-2" onClick={() => void refresh()}>Retry research status</button></div>}
    {status?.lastError && <div role="alert" className="rounded-lg border border-amber-700 p-3 text-sm text-amber-200"><p>Latest research attempt needs attention</p><p className="mt-1">{status.lastError}</p></div>}
    {sourceIssues.length > 0 && <details className="rounded-lg border border-amber-800 p-3 text-xs text-amber-200"><summary className="cursor-pointer">Collection gaps affect the available evidence: {sourceIssues.map((source) => source.label).slice(0, 3).join("; ")}{sourceIssues.length > 3 ? "; additional sources" : ""}</summary><ul className="mt-2 space-y-2">{sourceIssues.map((source) => <li key={source.url}>{/^https?:\/\//.test(source.url) ? <a className="text-cyan-300" href={source.url} target="_blank" rel="noreferrer">{source.label}</a> : source.label}: {source.detail}</li>)}</ul></details>}
    {selectedReport && status && !report && <p role="alert" className="rounded-lg border border-amber-700 p-3 text-sm text-amber-200">The selected dossier is no longer in the current catalog. It may have aged out of the recent list or become unavailable. Its cached evidence is no longer displayed; use the original dossier in a saved case to check availability.</p>}
    {report && status ? <><button type="button" className="analysis-button" onClick={() => setSelectedReport(null)}>Back to investigation briefing</button><Dossier key={`${report.id}:${selectedSection}:${selectedItemIndex}`} initialSection={selectedSection} initialItemIndex={selectedItemIndex} report={report} actionStates={status.actionStates} onChanged={() => void refresh()} /></>
      : status && status.reports.length > 0 ? <ResearchBriefing reports={status.reports} states={status.actionStates} onOpen={open} onReview={onReview} />
      : <div className="app-panel p-5 text-sm leading-relaxed text-slate-400"><h3 className="font-semibold text-slate-200">{!isTauri() ? "Open the desktop workspace to read saved investigations" : !status ? error ? "The saved investigation catalog could not be read" : "Checking for saved investigations" : !status.policy.enabled ? "No saved dossier yet; research is paused" : missingCredential ? "No saved dossier yet; model access needs setup" : "No completed investigation is available yet"}</h3><p className="mt-2">{!status ? "Findings will appear after the local research catalog is available. No research progress is assumed while it is unavailable." : status.policy.enabled && !missingCredential ? `Current recorded stage: ${status.phase}. A dossier appears only after the research run saves its evidence and conclusions.` : "Open Research settings below to configure the next run. Existing public records can still be reviewed from the evidence workspace."}</p></div>}
    <details ref={controlsRef} className="app-panel p-4" open={!status || status.reports.length === 0}><summary className="cursor-pointer text-sm font-semibold text-slate-300">Research controls and saved dossiers</summary><div className="space-y-4 pt-4">
      {status && status.reports.length > 0 && <label className="flex flex-wrap items-center gap-3 text-xs text-slate-300">Open a saved dossier<select aria-label="Research dossier" className="analysis-input min-w-0 flex-1" value={report?.id ?? ""} onChange={(event) => open(event.target.value, "findings")}><option value="">Choose an investigation</option>{report && !status.reports.some((item) => item.id === report.id) && <option value={report.id}>{report.synthesis.title} · currently open</option>}{status.reports.map((item) => <option key={item.id} value={item.id}>{item.synthesis.title} · {new Date(item.createdAt).toLocaleDateString()}</option>)}</select></label>}
      <div className="flex flex-wrap items-center gap-3"><label htmlFor="research-topic" className="text-xs text-slate-300">Investigate next</label><select id="research-topic" aria-label="Research topic" className="analysis-input" value={topic} onChange={(event) => setTopic(event.target.value)}>{INVESTIGATION_TOPICS.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}</select><button type="button" disabled={pending || !status?.policy.enabled || !!missingCredential || !!budgetMessage || !isTauri()} onClick={() => void run()} className="primary-action focus-ring rounded-lg px-4 py-2 text-sm font-semibold text-white disabled:opacity-50">{pending ? "Starting…" : "Investigate now"}</button></div>
      {status && <ResearchSettings settingsRef={settingsRef} policy={status.policy} callsToday={status.callsToday} available={status.credentialAvailable} onSaved={(value) => { gate.invalidate(); setStatus(value); }} />}
    </div></details>
    {status && <details className="app-panel p-4 text-xs text-slate-400"><summary className="cursor-pointer font-semibold text-slate-200">Collection coverage and source issues</summary><div className="mt-3 grid gap-3 md:grid-cols-2">{status.sources.map((source) => <div key={source.url}><a href={source.url} target="_blank" rel="noreferrer" className="text-cyan-300">{source.label}</a><p className="mt-1">{source.state}: {source.detail}</p></div>)}</div><p className="mt-3">{status.sources.length === 0 ? "No collection results were returned in this status response. " : ""}Collections are bounded. Paused and removed sources remain under your control. Review Activity for extraction failures and the actual documents acquired.</p></details>}
  </section>;
}

function ResearchSettings({
  settingsRef,
  policy,
  callsToday,
  available,
  onSaved,
}: {
  settingsRef: React.RefObject<HTMLDetailsElement>;
  policy: ResearchPolicy;
  callsToday: number;
  available: boolean;
  onSaved: (status: ResearchStatus) => void;
}) {
  const [draft, setDraft] = useState(policy);
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const gate = useLatestRequestGate();
  useEffect(() => {
    if (!dirty) setDraft(policy);
  }, [dirty, policy]);
  const update = (patch: Partial<ResearchPolicy>) => {
    setDirty(true);
    setDraft((prior) => ({ ...prior, ...patch }));
  };
  const save = async () => {
    const token = gate.begin();
    setSaving(true);
    setError("");
    try {
      const result = await invokeCommand("set_research_policy", {
        policy: draft,
      });
      if (gate.isCurrent(token)) {
        onSaved(result);
        setDirty(false);
      }
    } catch (reason) {
      if (gate.isCurrent(token)) setError(String(reason));
    } finally {
      if (gate.isCurrent(token)) setSaving(false);
    }
  };
  return (
    <details ref={settingsRef} className="mt-4 text-xs text-slate-400">
      <summary className="cursor-pointer">Research settings</summary>
      <fieldset disabled={saving} className="mt-3 space-y-3">
        <div className="flex flex-wrap gap-4">
          <label>
            <input
              type="checkbox"
              checked={draft.enabled}
              onChange={(event) => update({ enabled: event.target.checked })}
            />{" "}
            Autonomous research enabled
          </label>
          <label>
            <input
              type="checkbox"
              checked={draft.autoCollect}
              onChange={(event) =>
                update({ autoCollect: event.target.checked })
              }
            />{" "}
            Collect primary sources automatically
          </label>
        </div>
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <label>
            Provider
            <select
              className="analysis-input mt-1 w-full"
              value={draft.provider}
              onChange={(event) =>
                update({
                  provider: event.target.value as ResearchPolicy["provider"],
                  model:
                    event.target.value === "openai"
                      ? "gpt-5.4"
                      : event.target.value === "openrouter"
                        ? "openai/gpt-5.4"
                        : "",
                })
              }
            >
              <option value="openai">OpenAI API</option>
              <option value="openrouter">OpenRouter</option>
              <option value="ollama">Local Ollama</option>
            </select>
          </label>
          <label>
            Research model
            <input
              className="analysis-input mt-1 w-full"
              value={draft.model}
              onChange={(event) => update({ model: event.target.value })}
              placeholder="Installed model name"
            />
          </label>
          <label>
            Fallback hours per case
            <input
              type="number"
              min={1}
              max={168}
              className="analysis-input mt-1 w-full"
              value={draft.intervalHours}
              onChange={(event) =>
                update({ intervalHours: Number(event.target.value) })
              }
            />
          </label>
          <label>
            Daily model-call limit
            <input
              type="number"
              min={minimumResearchCalls(draft)}
              max={100}
              className="analysis-input mt-1 w-full"
              value={draft.dailyCallLimit}
              onChange={(event) =>
                update({ dailyCallLimit: Number(event.target.value) })
              }
            />
          </label>
        </div>
        <label className="block">
          Research depth{" "}
          <select
            className="analysis-input ml-2"
            value={draft.researchPasses}
            onChange={(event) =>
              update({ researchPasses: Number(event.target.value) })
            }
          >
            <option value={2}>2 passes · focused</option>
            <option value={4}>4 passes · deep</option>
            <option value={6}>6 passes · extended</option>
          </select>
        </label>
        <label className="block">
          <input
            type="checkbox"
            checked={draft.discoverSources}
            onChange={(event) =>
              update({ discoverSources: event.target.checked })
            }
          />{" "}
          Find additional primary sources for unresolved questions (OpenAI or
          OpenRouter web search)
        </label>
        <p>
          {callsToday} model calls used today (UTC). These settings require a budget of {minimumResearchCalls(draft)} calls per complete investigation, including source discovery when enabled.{" "}
          {draft.provider !== "ollama"
            ? `${draft.provider === "openrouter" ? "OpenRouter and its model provider receive" : "OpenAI receives"} selected public-record excerpts, query results, unresolved case questions, request gaps, decision tracking, and editorial challenges; API usage is billed to your key. ${draft.provider !== policy.provider ? "Save this provider to check its credential." : available ? "Credential detected." : "Credential not detected."}`
            : "Ollama runs at 127.0.0.1:11434; enter an installed model that supports structured JSON."}
        </p>
        <p>
          Each case is revisited when new searchable records arrive, after a
          short settling period and a 10-minute cooldown. The interval above is
          the fallback for cases without changed evidence. Failed attempts wait
          an hour before retrying. The daily call limit applies across cases.
        </p>
        <p>
          Collection, analysis, and storage upkeep run while the desktop app is
          open, including when another view is selected. Disabling research
          stops new model calls after the current call finishes. Storage upkeep
          has separate controls below.
        </p>
        <button
          type="button"
          className="analysis-button"
          onClick={() => void save()}
        >
          {saving ? "Saving…" : "Save research settings"}
        </button>
        {error && (
          <p role="alert" className="text-rose-300">
            {error}
          </p>
        )}
      </fieldset>
    </details>
  );
}

function Dossier({
  initialSection,
  initialItemIndex,
  report,
  actionStates,
  onChanged,
}: {
  initialSection: "findings" | "requests" | "meetings";
  initialItemIndex?: number;
  report: ResearchReport;
  actionStates: Record<string, string>;
  onChanged: () => void;
}) {
  const [tab, setTab] = useState<"findings" | "requests" | "meetings">(
    initialSection,
  );
  const [showAllRequests, setShowAllRequests] = useState(false);
  const ordered = <T,>(items: T[], section: typeof tab) => items.map((item, index) => ({ item, index })).sort((a, b) => section === initialSection && initialItemIndex !== undefined ? Number(b.index === initialItemIndex) - Number(a.index === initialItemIndex) : 0);
  return (
    <div className="app-panel space-y-5 p-5 sm:p-6">
      <div>
        <div className="app-eyebrow text-cyan-300">
          Evidence-grounded analysis · Researcher review needed
        </div>
        <h3 className="mt-2 text-xl font-semibold text-slate-100">
          {tab === "findings" ? report.synthesis.title : tab === "requests" ? "Request the records that can move this case forward" : "Follow the decision and establish its outcome"}
        </h3>
        {tab === "findings" ? <p className="mt-2 text-sm leading-relaxed text-slate-300">{report.synthesis.summary}</p> : <details className="mt-2 text-xs text-slate-400"><summary className="cursor-pointer">Investigation context</summary><p className="mt-2 font-semibold text-slate-300">{report.synthesis.title}</p><p className="mt-2 leading-relaxed">{report.synthesis.summary}</p></details>}
        <p className="mt-2 text-xs text-slate-400">
          {new Date(report.createdAt).toLocaleString()} · {report.model} · Saved evidence is reproduced below; source versions and interpretations need review.
        </p>
      </div>
      <div
        className="flex flex-wrap gap-2"
        role="group"
        aria-label="Dossier sections"
      >
        {(["findings", "requests", "meetings"] as const).map((value) => (
          <button
            key={value}
            type="button"
            aria-pressed={tab === value}
            className="analysis-button"
            onClick={() => setTab(value)}
          >
            {value === "findings"
              ? "Findings and connections"
              : value === "requests"
                ? "Recommended records requests"
                : "Meetings and follow-through"}
          </button>
        ))}
      </div>
      {tab === "findings" && (
        <>
          {report.synthesis.findings.length === 0 && (
            <p className="text-sm text-slate-400">
              This investigation did not establish a useful finding yet. The
              questions and recommended requests identify what evidence is
              needed.
            </p>
          )}
          {ordered(report.synthesis.findings, "findings").map(({ item: finding, index }) => (
            <article
              key={index}
              className="space-y-3 rounded-xl border border-slate-700 p-4"
            >
              <h4 className="text-base font-semibold text-slate-100">
                {finding.title}
              </h4>
              <p className="text-sm leading-relaxed text-slate-300">
                {finding.interpretation}
              </p>
              <p className="text-sm text-cyan-200">
                Why it matters: {finding.whyItMatters}
              </p>
              <Citations citations={finding.citations} report={report} />
              {finding.counterEvidence.length > 0 && (
                <div>
                  <h5 className="text-xs font-semibold text-slate-300">
                    Counter-evidence
                  </h5>
                  <Citations
                    citations={finding.counterEvidence}
                    report={report}
                  />
                </div>
              )}
              <div className="text-xs leading-relaxed text-slate-400">
                <span className="font-semibold text-slate-300">
                  Other explanations to test
                </span>
                <ul className="mt-1 list-disc space-y-1 pl-4">
                  {finding.alternativeExplanations.map((item, i) => (
                    <li key={i}>{item}</li>
                  ))}
                </ul>
              </div>
              <p className="text-sm text-slate-200">
                Next test: {finding.nextTest}
              </p>
              {finding.citations.some((c) =>
                report.evidence.some(
                  (e) =>
                    e.id === c.evidenceId &&
                    e.portalKey &&
                    e.documentId !== null &&
                    e.requestId !== null,
                ),
              ) && (
                <button
                  type="button"
                  className="analysis-button"
                  onClick={() => addFindingToBriefing(report, finding)}
                >
                  Add cited passages to briefing
                </button>
              )}
              <ActionState
                actionKey={`${report.id}:finding:${index}`}
                value={actionStates[`${report.id}:finding:${index}`]}
                onChanged={onChanged}
              />
            </article>
          ))}
          <div>
            <h4 className="text-sm font-semibold text-slate-200">
              Questions developed from the evidence
            </h4>
            <div className="mt-3 space-y-3">
              {report.synthesis.questions.map((question, index) => (
                <div key={index}>
                  <p className="text-sm text-slate-200">{question.question}</p>
                  <p className="mt-1 text-xs text-slate-400">{question.why}</p>
                  <EvidenceLinks ids={question.evidenceIds} report={report} />
                </div>
              ))}
            </div>
          </div>
        </>
      )}
      {tab === "requests" && (
        <div className="space-y-4">
          {report.synthesis.requests.length === 0 && (
            <p className="text-sm text-slate-400">
              No evidence-grounded records requests were identified in this
              pass.
            </p>
          )}
          {ordered(report.synthesis.requests, "requests").filter(({ index }) => initialSection !== "requests" || initialItemIndex === undefined || showAllRequests || index === initialItemIndex).map(({ item: request, index }) => (
            <article
              key={index}
              className="space-y-3 rounded-xl border border-slate-700 p-4"
            >
              <h4 className="text-base font-semibold text-slate-100">
                {request.title}
              </h4>
              <p className="text-xs text-cyan-200">
                Suggested custodian: {request.custodian}
              </p>
              <p className="text-sm text-slate-300">
                Missing evidence: {request.gap}
              </p>
              <p className="text-sm text-slate-300">
                What it would resolve: {request.resolves}
              </p>
              <details open={initialSection === "requests" && initialItemIndex === index}>
                <summary className="cursor-pointer text-sm text-cyan-300">
                  Review request draft
                </summary>
                <pre className="mt-3 whitespace-pre-wrap text-sm leading-relaxed text-slate-300">
                  {request.draft}
                </pre>
                <button
                  type="button"
                  className="analysis-button mt-3"
                  onClick={() =>
                    download(
                      `${report.topic}-request-${index + 1}.txt`,
                      request.draft,
                    )
                  }
                >
                  Download draft
                </button>
                <p className="mt-2 text-xs text-slate-400">
                  Review scope and recipient before submitting. No request has
                  been sent.
                </p>
              </details>
              <details className="text-xs text-slate-400"><summary className="cursor-pointer text-cyan-300">Evidence behind this request</summary><div className="mt-3 space-y-3">
              {requestFindingLinks(report, request.evidenceIds).length > 0 && (
                <div className="text-xs text-slate-400">
                  <p className="font-semibold text-slate-300">Related findings sharing cited sources</p>
                  <ul className="mt-1 list-disc space-y-1 pl-4">
                    {requestFindingLinks(report, request.evidenceIds).map((finding) => (
                      <li key={finding.index}>{finding.title}</li>
                    ))}
                  </ul>
                </div>
              )}
              <EvidenceLinks ids={request.evidenceIds} report={report} />
              </div></details>
              <ActionState
                actionKey={`${report.id}:request:${index}`}
                value={actionStates[`${report.id}:request:${index}`]}
                onChanged={onChanged}
              />
            </article>
          ))}
        </div>
      )}
      {tab === "requests" && initialSection === "requests" && initialItemIndex !== undefined && !showAllRequests && report.synthesis.requests.length > 1 && <button type="button" className="analysis-button" onClick={() => setShowAllRequests(true)}>Show the other {report.synthesis.requests.length - 1} requests from this dossier</button>}
      {tab === "meetings" && (
        <div className="space-y-4">
          {report.synthesis.meetings.length === 0 && (
            <p className="text-sm text-slate-400">
              No relevant meeting with a verified date was found in the
              retrieved agendas. The engine will recheck as new records arrive.{" "}
              <a
                href="https://www.sandiego.gov/city-clerk/officialdocs/legisdocs/brown-act"
                target="_blank"
                rel="noreferrer"
                className="text-cyan-300"
              >
                Check current Council agendas
              </a>
              .
            </p>
          )}
          {ordered(report.synthesis.meetings, "meetings").map(({ item: meeting, index }) => (
            <article
              key={index}
              className="space-y-3 rounded-xl border border-slate-700 p-4"
            >
              <div className="text-xs text-cyan-300">
                {meeting.date < localDateInput(new Date())
                  ? "Historical meeting · follow-through"
                  : "Upcoming or current meeting · confirm agenda"}
              </div>
              <h4 className="text-base font-semibold text-slate-100">
                {meeting.body} · {meeting.date}
              </h4>
              <p className="text-sm text-slate-200">
                Agenda item: {meeting.agendaItem}
              </p>
              <p className="text-sm text-slate-300">{meeting.relevance}</p>
              <p className="text-sm text-cyan-200">
                Recommended action: {meeting.action}
              </p>
              <Citations citations={meeting.citations} report={report} />
              <p className="text-xs text-slate-400">
                Confirm the latest agenda, participation instructions, and any
                deadline at the official source.
              </p>
              <ActionState
                actionKey={`${report.id}:meeting:${index}`}
                value={actionStates[`${report.id}:meeting:${index}`]}
                onChanged={onChanged}
              />
            </article>
          ))}
        </div>
      )}
      <ResearchExport report={report} actionStates={actionStates} />
      <details className="text-xs text-slate-400">
        <summary className="cursor-pointer">
          Coverage, limitations, and investigation trail
        </summary>
        <ul className="mt-3 list-disc space-y-2 pl-4">
          {report.synthesis.limitations.map((item, i) => (
            <li key={i}>{item}</li>
          ))}
        </ul>
        <ol className="mt-4 list-decimal space-y-1 pl-4">
          {report.investigationLog.map((item, i) => (
            <li key={i}>{item}</li>
          ))}
        </ol>
        <p className="mt-3">
          Input tokens: {report.inputTokens.toLocaleString()} · Output tokens,
          including reasoning: {report.outputTokens.toLocaleString()}
        </p>
      </details>
    </div>
  );
}

function addFindingToBriefing(
  report: ResearchReport,
  finding: ResearchFinding,
) {
  const state = useAppStore.getState();
  for (const citation of [...finding.citations, ...finding.counterEvidence]) {
    const evidence = report.evidence.find((e) => e.id === citation.evidenceId);
    if (
      !evidence?.portalKey ||
      evidence.requestId === null ||
      evidence.documentId === null
    )
      continue;
    const item = documentBriefingEvidence({
      portalKey: evidence.portalKey,
      requestId: evidence.requestId,
      documentId: evidence.documentId,
      fileName: evidence.label,
      characterOffset: evidence.characterOffset,
      evidenceId: evidence.id,
      sourceUrl: evidence.sourceUrl,
      agencyName: "Check agency in original record",
      requestTitle: finding.title,
      excerpt: citation.quote,
      rawFileSha256: evidence.sourceSha256 || null,
    });
    state.addBriefingEvidence({
      ...item,
      claim: `Research interpretation requiring review: ${finding.interpretation}`,
      note: `Dossier: ${report.synthesis.title}. Next test: ${finding.nextTest} Alternatives: ${finding.alternativeExplanations.join("; ")}`,
      readiness: "LEAD",
    });
  }
  state.setWorkspaceView("briefing");
}

function ActionState({
  actionKey,
  value = "OPEN",
  onChanged,
}: {
  actionKey: string;
  value?: string;
  onChanged: () => void;
}) {
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const change = async (state: string) => {
    setSaving(true);
    setError("");
    try {
      await invokeCommand("set_research_action_state", { actionKey, state });
      onChanged();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setSaving(false);
    }
  };
  return (
    <div>
      <label className="text-xs text-slate-400">
        Follow-through{" "}
        <select
          aria-label={`Follow-through ${actionKey}`}
          disabled={saving}
          className="analysis-input ml-2"
          value={value}
          onChange={(event) => void change(event.target.value)}
        >
          <option value="OPEN">To do</option>
          <option value="IN_PROGRESS">In progress</option>
          <option value="DONE">Completed</option>
          <option value="DISMISSED">Dismissed</option>
        </select>
      </label>
      {error && (
        <p role="alert" className="mt-2 text-xs text-rose-300">
          {error}
        </p>
      )}
    </div>
  );
}
function download(name: string, content: string) {
  const url = URL.createObjectURL(
    new Blob([content], { type: "text/plain;charset=utf-8" }),
  );
  const anchor = document.createElement("a");
  anchor.href = url;
  anchor.download = name;
  anchor.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
