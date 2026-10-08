import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { invokeCommand } from "../../ipc";
import { useLatestRequestGate } from "../../hooks/useLatestRequestGate";
import CaseMerge from "./CaseMerge";
import CaseSharing, { SignerTrustNotice } from "./CaseSharing";
import OverdueFollowUp from "./OverdueFollowUp";
import { useContributors } from "./useContributors";
import type { CaseExportLedgerEntry, CaseworkCase, CaseworkCitation, CaseworkCommitment, CaseworkImportPreview, CaseworkQuestion, CaseworkRequest, CaseworkState, CompromisedImportWarning, FollowUpTrigger, RequestFilingRecordResult } from "../../types/casework";
import type { CaseEvidencePackageResult, RevokedSigningKey, SigningKeyRevocations, SigningKeyRotationResult } from "../../types/investigation";
import { localDateInput } from "../../utils/briefing";
import { caseworkDifferences } from "../../utils/caseworkDiff";
import { downloadFile } from "./downloadFile";
import { formatShortDate } from "../objects/labels";
import InlineError from "../ui/InlineError";

const list = (text: string) => text.split("\n").map((s) => s.trim()).filter(Boolean);
const id = () => crypto.randomUUID();
// A key can carry more than one entry (retired for hygiene, then later found compromised); this
// mirrors integrity::check_revocation's worst-severity rule so the list never renders a key that
// was escalated to COMPROMISED as if it were merely RETIRED, and never double-counts or double-keys
// it as two separate rows.
function worstRevocationPerKey(entries: RevokedSigningKey[]): RevokedSigningKey[] {
  const byKey = new Map<string, RevokedSigningKey>();
  for (const entry of entries) {
    const existing = byKey.get(entry.keyId);
    if (!existing || (existing.status === "RETIRED" && entry.status !== "RETIRED")) byKey.set(entry.keyId, entry);
  }
  return [...byKey.values()];
}
/** "Custodian 1" or "Produced fields 1.2": the index token that tells repeated controls apart. */
function isIndexToken(word: string) {
  return word.length > 0 && [...word].every((char) => "0123456789.".includes(char));
}
/** Each repeated control sits in its own titled card, so the visible label drops the index; the
 * accessible name keeps it, because a screen reader lists the controls without their cards. */
function visibleLabel(label: string) {
  return label.split(" ").filter((word) => !isIndexToken(word)).join(" ");
}
/** "NEEDS_REVIEW" → "Needs review". */
function sentenceLabel(value: string) {
  const words = value.split("_").join(" ").toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}
function Field({ label, value, onChange, multiline = false, type = "text" }: { label: string; value: string; onChange: (value: string) => void; multiline?: boolean; type?: string }) {
  return <label className="block text-control text-fg-secondary">{visibleLabel(label)}{multiline ? <textarea aria-label={label} className="analysis-input mt-1.5 h-auto min-h-20 w-full py-2" rows={3} value={value} onChange={(e) => onChange(e.target.value)} /> : <input aria-label={label} className="analysis-input mt-1.5 w-full" type={type} value={value} onChange={(e) => onChange(e.target.value)} />}</label>;
}
function State({ label, value, options, onChange }: { label: string; value: string; options: string[]; onChange: (value: string) => void }) {
  return <label className="block text-control text-fg-secondary">{visibleLabel(label)}<select aria-label={label} className="analysis-input mt-1.5 w-full" value={value} onChange={(e) => onChange(e.target.value)}>{options.map((option) => <option key={option} value={option}>{sentenceLabel(option)}</option>)}</select></label>;
}
function CitationPicker({ label, values, evidence, onChange }: { label: string; values: string[]; evidence: CaseworkCitation[]; onChange: (value: string[]) => void }) {
  return <details className="text-label leading-4"><summary className="text-accent">{label} ({values.length})</summary><fieldset className="mt-2 max-h-64 space-y-2 overflow-auto"><legend className="sr-only">{label}</legend>{evidence.map((cite) => <label key={cite.id} className="block rounded border border-line-strong p-2 text-fg-secondary"><input type="checkbox" checked={values.includes(cite.id)} onChange={(e) => onChange(e.target.checked ? [...values, cite.id] : values.filter((value) => value !== cite.id))} /> {cite.label}<span className="ml-2 text-warning">{cite.verification === "SAVED_REPORT" ? "Saved report source · current source matches" : cite.verification === "HISTORICAL_REPORT" ? "Historical report · source has a newer version" : "Unverified portable source"}</span><span className="mt-1 block whitespace-pre-wrap">{cite.quote.slice(0, 900)}{cite.quote.length > 900 ? "…" : ""}</span></label>)}</fieldset></details>;
}
function requestOmissions(request: CaseworkRequest) {
  const present = (value: string, actual: string[]) => actual.some((a) => a.trim().toLowerCase() === value.trim().toLowerCase());
  return [...request.unresolvedOmissions,
    ...request.expectedRecordTypes.filter((r) => !present(r, request.productions.flatMap((p) => p.recordTypes))).map((r) => `Expected record not yet recorded as produced: ${r}`),
    ...request.expectedFields.filter((f) => !present(f, request.productions.flatMap((p) => p.fields))).map((f) => `Expected field not yet recorded as produced: ${f}`),
    ...(request.expectedPeriod && !present(request.expectedPeriod, request.productions.map((p) => p.period)) ? [`Requested period coverage remains unconfirmed: ${request.expectedPeriod}`] : []),
  ];
}
const FOLLOW_UP_LABELS: Record<FollowUpTrigger, string> = {
  DETERMINATION_DUE: "Determination due",
  DETERMINATION_OVERDUE: "Determination overdue",
  PRODUCTION_INCOMPLETE: "Production incomplete",
};
// The compiled request, its modeled clock and its scheduled follow-ups. An unfiled request has no
// `filingPlan` at all and is shown as having no clock, never as a clock reading zero days.
function RequestFiling({ caseId, request, busy, onFile, record }: { caseId: string; request: CaseworkRequest; busy: boolean; onFile: (requestId: string) => void; record?: RequestFilingRecordResult }) {
  const plan = request.filingPlan;
  return <details><summary className="text-label leading-4 text-accent">Compiled CPRA request and statutory clock</summary>
    <pre className="mt-2 whitespace-pre-wrap text-label leading-4 text-fg-secondary">{request.compiledDraft || "Save the case to compile this request from its recorded scope and citations."}</pre>
    {request.compiledDraft && <button type="button" className="analysis-button mt-2" onClick={() => downloadFile(`${request.id}-cpra-request.txt`, request.compiledDraft)}>Download compiled request</button>}
    {plan === null
      ? <p className="mt-2 text-label leading-4 text-warning">No determination clock: no submission date is recorded for this request. The app sends nothing; record the date you filed it.</p>
      : <div className="mt-2 space-y-1 text-label leading-4 text-fg-secondary">
          <p>Filed {plan.filedOn} · determination due {plan.determinationDueOn} under {plan.statuteCitation} · {plan.calendarDaysElapsed} of {plan.allowedCalendarDays} allowed calendar days elapsed as of {plan.evaluatedAt}, the last save.</p>
          <p className="text-muted">{plan.measuredTo === null ? "Measured to today: nothing is recorded at or after the filing date." : `Measured to the earliest recorded production, ${plan.measuredTo}.`}</p>
          {plan.deadlinePassedWithoutRecordedResponse && <p className="text-warning">The deadline passed with no recorded response. That is what this states; it is not a finding that the agency failed to determine.</p>}
          <ul className="list-disc space-y-1 pl-4">{plan.followUps.map((followUp, i) => <li key={i}>{FOLLOW_UP_LABELS[followUp.trigger]} on {followUp.scheduledFor}: {followUp.rationale}</li>)}</ul>
          {plan.measuredTo === null && <OverdueFollowUp caseId={caseId} requestId={request.id} dueOn={plan.determinationDueOn} />}
          <button type="button" className="analysis-button" disabled={busy} onClick={() => onFile(request.id)}>Sign filing record for this request</button>
          {record && <p className="text-highlight">Signed filing record at {record.outputPath} (SHA-256 {record.packageSha256.slice(0, 12)}, key {record.signerKeyId.slice(0, 12)}). The signature proves these bytes are unchanged since this installation produced them, not who filed or that a send occurred. Verify it with {record.verifierScriptPath}, or open {record.verifierPagePath} in a browser — that page re-checks the signature and hash locally and states that it proves internal consistency only, never who signed.</p>}
        </div>}
  </details>;
}
export default function CaseworkPanel({ onSelectCase }: { onSelectCase?: (caseId: string | null, topic: string, reportId: string) => void } = {}) {
  const [state, setState] = useState<CaseworkState | null>(null);
  const [draft, setDraft] = useState<CaseworkCase | null>(null);
  const [author, setAuthor] = useState("");
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [payload, setPayload] = useState("");
  const [preview, setPreview] = useState<CaseworkImportPreview | null>(null);
  const [exported, setExported] = useState("");
  const [custodyExport, setCustodyExport] = useState<CaseEvidencePackageResult | null>(null);
  const [withheld, setWithheld] = useState<Record<string, string>>({});
  const [chain, setChain] = useState<CaseExportLedgerEntry[] | null>(null);
  const [filingRecords, setFilingRecords] = useState<Record<string, RequestFilingRecordResult>>({});
  const [rotationReason, setRotationReason] = useState("");
  const [rotationCompromised, setRotationCompromised] = useState(false);
  const [rotationResult, setRotationResult] = useState<SigningKeyRotationResult | null>(null);
  const [confirmingCompromise, setConfirmingCompromise] = useState(false);
  const [compromisedImports, setCompromisedImports] = useState<{ status: "loading" | "failed" | "ready"; entries: CompromisedImportWarning[] }>({ status: "ready", entries: [] });
  const [markingCompromisedKeyId, setMarkingCompromisedKeyId] = useState<string | null>(null);
  const [markCompromisedReason, setMarkCompromisedReason] = useState("");
  // The CLAUDE.md activeDocumentRef pattern: activeCaseIdRef records which case the viewer is
  // actually looking at right now, set only where that changes (open()). loadCompromisedImports
  // discards a result whose case the viewer has since left, using this ref, not the shared `gate`
  // (every save/export/chain-view button uses that) — sharing it would let an unrelated click
  // silently discard a real compromise warning, since `gate.begin()` invalidates every other
  // in-flight token, not just ones for the same case.
  //
  // Revocations and compromised-import status are installation-level facts, not case-scoped, so
  // their refresh after mark-compromised or a compromising rotation (`refreshingRevocations`) must
  // run unconditionally on that command's own promise, never inside `perform`'s gated `apply`: `open()` also resets
  // `busy`, so switching cases while that write is still in flight re-enables the very buttons
  // that started it, and `gate.begin()` for the next click invalidates the pending token — if the
  // refresh lived in `apply`, it would be silently skipped and the view would never learn the key
  // was actually marked compromised. `loadCompromisedImports` re-reads `activeCaseIdRef.current`
  // (not a case id captured at click time) so the refresh still lands on whatever case is open
  // when the write actually finishes, not the one that was open when the button was clicked.
  const activeCaseIdRef = useRef<string | null>(null);
  const loadCompromisedImports = (caseId: string) => {
    if (activeCaseIdRef.current !== caseId) return;
    setCompromisedImports({ status: "loading", entries: [] });
    invokeCommand("compromised_case_imports", { caseId })
      .then((entries) => { if (activeCaseIdRef.current === caseId) setCompromisedImports({ status: "ready", entries }); })
      .catch(() => { if (activeCaseIdRef.current === caseId) setCompromisedImports({ status: "failed", entries: [] }); });
  };
  const [revocations, setRevocations] = useState<SigningKeyRevocations | null>(null);
  const refreshingRevocations = <T,>(write: Promise<T>) =>
    write
      .then((value) => {
        void invokeCommand("get_signing_key_revocations", {}).then(setRevocations).catch(() => setRevocations(null));
        if (activeCaseIdRef.current) loadCompromisedImports(activeCaseIdRef.current);
        return value;
      })
      .catch((error) => { setError(String(error)); throw error; });
  const gate = useLatestRequestGate();
  const contributors = useContributors();
  async function perform<T>(operation: () => Promise<T>, apply: (value: T) => void) {
    const token = gate.begin(); setBusy(true); setError(""); setMessage("");
    try { const value = await operation(); if (gate.isCurrent(token)) apply(value); }
    catch (reason) { if (gate.isCurrent(token)) setError(String(reason)); }
    finally { if (gate.isCurrent(token)) setBusy(false); }
  }
  const reloadState = () => { if (isTauri()) void perform(() => invokeCommand("get_casework_state", {}), setState); };
  const draftIsDirty = () => {
    if (!draft) return false;
    const saved = state?.cases.find((c) => c.id === draft.id);
    return !saved || JSON.stringify(saved) !== JSON.stringify(draft);
  };
  useEffect(() => {
    if (isTauri()) void perform(() => invokeCommand("get_casework_state", {}), setState);
    return () => gate.invalidate();
    // Loading belongs to this lifecycle; edits are never replaced by a background poll.
  }, [gate]);
  const open = (value: CaseworkCase) => { gate.invalidate(); setBusy(false); setDraft(structuredClone(value)); setExported(""); setCustodyExport(null); setWithheld({}); setChain(null); setPreview(null); setError(""); setMessage(""); setMarkingCompromisedKeyId(null); setMarkCompromisedReason(""); activeCaseIdRef.current = value.id; loadCompromisedImports(value.id); onSelectCase?.(value.id, value.topic, value.reportId); };
  // `source: "import"` because only an accepted import can add a new signer to this case — a
  // plain save doesn't change who signed which revision, so it doesn't need a compromised-imports
  // refresh. Always reload when the case identity itself changed, since the banner is scoped to
  // whichever case is on screen.
  const accept = (value: CaseworkCase, source: "save" | "import" = "save") => { const switched = activeCaseIdRef.current !== value.id; if (switched) { setChain(null); setCustodyExport(null); setWithheld({}); setExported(""); setMarkingCompromisedKeyId(null); setMarkCompromisedReason(""); } activeCaseIdRef.current = value.id; if (switched || source === "import") loadCompromisedImports(value.id); onSelectCase?.(value.id, value.topic, value.reportId); setDraft(value); setState((prior) => prior ? { ...prior, cases: [value, ...prior.cases.filter((c) => c.id !== value.id)] } : { cases: [value], events: [], leads: [], availableEvidence: [] }); setMessage(`Saved revision ${value.revision}.`); };
  const update = (patch: Partial<CaseworkCase>) => setDraft((prior) => prior ? { ...prior, ...patch } : null);
  const patchQuestion = (index: number, patch: Partial<CaseworkQuestion>) => draft && update({ questions: draft.questions.map((q, i) => i === index ? { ...q, ...patch } : q) });
  const patchRequest = (index: number, patch: Partial<CaseworkRequest>) => draft && update({ requests: draft.requests.map((r, i) => i === index ? { ...r, ...patch } : r) });
  const patchCommitment = (index: number, patch: Partial<CaseworkCommitment>) => draft && update({ commitments: draft.commitments.map((c, i) => i === index ? { ...c, ...patch } : c) });
  const today = localDateInput(new Date());
  const differences = preview ? caseworkDifferences(state?.cases.find((c) => c.id === preview.case.id), preview.case) : [];
  return <section aria-label="Persistent investigation casework" className="app-panel space-y-5 p-5 sm:p-6">
    <div className="flex flex-wrap items-start justify-between gap-3"><div><h3 className="text-heading text-fg">Cases</h3><p className="mt-1 text-caption text-muted">Track requests, productions and dated decisions for a case.</p></div>{state !== null && <button type="button" className="analysis-button" disabled={busy} onClick={reloadState}>Refresh</button>}</div>
    {!isTauri() && <p className="text-sm text-muted">Persistent casework runs in the desktop app.</p>}
    {state === null && error ? <InlineError summary="Cases could not be loaded." detail={error} onRetry={reloadState} /> : <>
    {error && <p role="alert" className="text-body text-danger">{error}</p>}{message && <p role="status" className="text-body text-fg-secondary">{message}</p>}
    <div className="grid gap-3 sm:grid-cols-2"><Field label="Contributor name or team label" value={author} onChange={setAuthor} /><label className="text-label leading-4 text-fg-secondary">Open saved case<select aria-label="Open saved case" className="analysis-input mt-1 w-full" value={draft?.id ?? ""} onChange={(e) => { const selected = state?.cases.find((c) => c.id === e.target.value); if (!selected) return; if (draftIsDirty() && !window.confirm(`Discard unsaved changes to ${draft?.title || "this case"}?`)) return; open(selected); }}><option value="">Choose a case</option>{state?.cases.map((c) => <option key={c.id} value={c.id}>{c.title} · revision {c.revision}</option>)}</select></label></div>
    <p className="text-caption text-muted">Contributor labels record responsibility; they are not authenticated identities.</p>
    <details open={!draft}><summary className="font-semibold text-fg">Ranked investigative leads</summary><p className="mt-2 text-label leading-4 text-muted">Ranking explains public consequence, available records, changed findings and dated decision opportunities. Scores prioritize review; they do not establish wrongdoing or confidence.</p><div className="mt-3 grid gap-3 lg:grid-cols-2">{state?.leads.slice(0, 8).map((lead) => <article key={`${lead.reportId}:${lead.findingIndex}`} className="rounded border border-line-strong p-3"><h4 className="text-sm font-semibold text-fg">{lead.title}</h4><p className="mt-2 text-label leading-4 text-fg-secondary">Next test: {lead.nextAction}</p><details className="mt-2 text-label leading-4 text-muted"><summary className="">Priority {lead.score}/11 · why this lead ranks here</summary><ul className="mt-2 space-y-2">{lead.factors.map((factor) => <li key={factor.label}><strong>{factor.label}: {factor.score}/{factor.maximum}.</strong> {factor.reason}</li>)}</ul></details><details className="mt-2 text-label leading-4 text-muted"><summary className="text-accent">Cited evidence ({lead.citations.length})</summary>{lead.citations.map((cite) => <blockquote key={cite.id} className="mt-2 border-l border-accent pl-2">{cite.quote}<p>{cite.sourceUrl && /^https?:\/\//.test(cite.sourceUrl) && <a className="text-accent" href={cite.sourceUrl} target="_blank" rel="noreferrer">{cite.label}</a>}</p></blockquote>)}</details><button type="button" disabled={busy} className="analysis-button mt-3" onClick={() => { const existing = state?.cases.find((c) => c.reportId === lead.reportId); if (existing) open(existing); else void perform(() => invokeCommand("create_casework_case", { reportId: lead.reportId }), accept); }}>{lead.caseId ? "Open investigation" : "Build case from this dossier"}</button></article>)}</div>{state && state.leads.length === 0 && <p className="mt-3 text-sm text-muted">Saved research findings will become cited leads here.</p>}</details>
    {draft && <div className="space-y-5" aria-label={`Case ${draft.title}`}>
      {draft.evidence.some((cite) => cite.verification === "HISTORICAL_REPORT") && <p role="status" className="rounded border border-warning p-3 text-sm text-warning">Historical evidence: a public source has a newer version. Retained quotations and source hashes remain available, and this case can still be edited. Assessments relying only on older evidence need review before they can be marked supported, decided or complete.</p>}
      {compromisedImports.status === "failed" && <InlineError summary="Could not check whether this case's imports came from a since-compromised signing key. Treat that as unknown, not as clear, until this check succeeds." onRetry={() => loadCompromisedImports(draft.id)} />}
      {compromisedImports.status === "ready" && compromisedImports.entries.length > 0 && <div role="alert" className="rounded border border-danger p-3 text-sm text-danger"><p className="font-semibold">{compromisedImports.entries.length} previously imported revision{compromisedImports.entries.length === 1 ? "" : "s"} of this case came from a signing key since marked compromised:</p><ul className="mt-2 list-disc pl-4 text-label leading-4">{compromisedImports.entries.map((warning) => <li key={warning.revision}>revision {warning.revision} · signer <code>{warning.signerKeyId.slice(0, 12)}…</code> · compromised <time dateTime={warning.revokedAt}>{formatShortDate(warning.revokedAt)}</time> ({warning.reason})</li>)}</ul><p className="mt-2 text-label leading-4">This does not mean the imported content is false — it means the channel it arrived through is no longer trusted. Review what those revisions changed before relying on them.</p></div>}
      <div className="grid gap-3 sm:grid-cols-3"><Field label="Case title" value={draft.title} onChange={(title) => update({ title })} /><Field label="Case owner" value={draft.owner} onChange={(owner) => update({ owner })} /><State label="Case disposition" value={draft.disposition} options={["OPEN", "ACTIVE", "ON_HOLD", "RESOLVED", "DISMISSED"]} onChange={(disposition) => update({ disposition })} /></div>
      <p className="text-label leading-4 text-muted">Revision {draft.revision} · Updated {new Date(draft.updatedAt).toLocaleString()}. Concurrent edits are rejected instead of overwriting another contributor.</p>
      <details><summary className="text-sm text-accent">Attach later evidence to this case</summary><p className="mt-2 text-label leading-4 text-muted">Select a source from a later saved dossier. It becomes available to cite in productions, outcomes and commitment assessments after saving.</p><div className="mt-2 max-h-60 space-y-2 overflow-auto">{state?.availableEvidence.filter((cite) => !draft.evidence.some((e) => e.id === cite.id)).map((cite) => <label key={cite.id} className="block text-label leading-4 text-fg-secondary"><input type="checkbox" checked={false} onChange={() => update({ evidence: [...draft.evidence, cite] })} /> {cite.label} · {cite.reportId}<span className="ml-4 block text-muted">{cite.quote.slice(0, 240)}</span></label>)}</div></details>
      <Field label="Case notes" value={draft.notes} onChange={(notes) => update({ notes })} multiline />
      <details open><summary className="font-semibold text-fg">Questions, hypotheses and falsification tests</summary><div className="mt-3 space-y-3">{draft.questions.map((q, index) => <article key={q.id} className="space-y-3 rounded border border-line-strong p-3"><Field label={`Question ${index + 1}`} value={q.question} onChange={(question) => patchQuestion(index, { question })} /><Field label={`Hypothesis ${index + 1}`} value={q.hypothesis} onChange={(hypothesis) => patchQuestion(index, { hypothesis })} multiline /><Field label={`Next test ${index + 1}`} value={q.nextTest} onChange={(nextTest) => patchQuestion(index, { nextTest })} multiline /><div className="grid gap-3 sm:grid-cols-2"><Field label={`Question owner ${index + 1}`} value={q.owner} onChange={(owner) => patchQuestion(index, { owner })} /><State label={`Question disposition ${index + 1}`} value={q.disposition} options={["OPEN", "TESTING", "SUPPORTED", "REFUTED", "DISMISSED", "NEEDS_REVIEW"]} onChange={(disposition) => patchQuestion(index, { disposition })} /></div><CitationPicker label={`Supporting evidence for question ${index + 1}`} values={q.citationIds} evidence={draft.evidence} onChange={(citationIds) => patchQuestion(index, { citationIds })} /><CitationPicker label={`Counter-evidence for question ${index + 1}`} values={q.counterCitationIds} evidence={draft.evidence} onChange={(counterCitationIds) => patchQuestion(index, { counterCitationIds })} /></article>)}</div><button type="button" className="analysis-button mt-3" onClick={() => update({ questions: [...draft.questions, { id: id(), question: "", hypothesis: "", nextTest: "", owner: draft.owner, disposition: "OPEN", citationIds: [], counterCitationIds: [] }] })}>Add research question</button></details>
      <details><summary className="font-semibold text-fg">Records requests, productions and omissions</summary><p className="mt-2 text-label leading-4 text-muted">Dates and submissions are entered from your records. The app does not send requests. Save to prepare a follow-up tied to the remaining evidence gaps.</p><div className="mt-3 space-y-4">{draft.requests.map((r, index) => <article key={r.id} className="space-y-3 rounded border border-line-strong p-4"><h4 className="font-semibold text-fg">{r.title}</h4><p className="text-label leading-4 text-accent">What this resolves: {r.resolves}</p><Field label={`Custodian ${index + 1}`} value={r.custodian} onChange={(custodian) => patchRequest(index, { custodian })} /><div className="grid gap-3 sm:grid-cols-2"><Field label={`Expected record types ${index + 1} (one per line)`} value={r.expectedRecordTypes.join("\n")} onChange={(value) => patchRequest(index, { expectedRecordTypes: list(value) })} multiline /><Field label={`Expected fields ${index + 1} (one per line)`} value={r.expectedFields.join("\n")} onChange={(value) => patchRequest(index, { expectedFields: list(value) })} multiline /></div><Field label={`Expected period ${index + 1}`} value={r.expectedPeriod} onChange={(expectedPeriod) => patchRequest(index, { expectedPeriod })} /><Field label={`Request draft ${index + 1}`} value={r.draft} onChange={(draft) => patchRequest(index, { draft })} multiline /><div className="grid gap-3 sm:grid-cols-2"><State label={`Request state ${index + 1}`} value={r.state} options={["DRAFT", "SUBMITTED", "ACKNOWLEDGED", "PARTIAL", "PRODUCED", "CLOSED"]} onChange={(state) => patchRequest(index, { state })} /><Field label={`Agency request reference ${index + 1}`} value={r.externalReference} onChange={(externalReference) => patchRequest(index, { externalReference })} /><Field type="date" label={`Actual submission date ${index + 1}`} value={r.submittedAt ?? ""} onChange={(value) => patchRequest(index, { submittedAt: value || null })} /><Field type="date" label={`Recorded response due date ${index + 1}`} value={r.responseDue ?? ""} onChange={(value) => patchRequest(index, { responseDue: value || null })} /></div>
        {r.productions.map((production, pi) => { const change = (patch: Partial<typeof production>) => patchRequest(index, { productions: r.productions.map((p, i) => i === pi ? { ...p, ...patch } : p) }); return <div key={production.id} className="space-y-2 rounded bg-surface/70 p-3"><h5 className="text-sm text-fg">Production {pi + 1}</h5><Field type="date" label={`Received date ${index + 1}.${pi + 1}`} value={production.receivedAt} onChange={(receivedAt) => change({ receivedAt })} /><Field label={`Production description ${index + 1}.${pi + 1}`} value={production.description} onChange={(description) => change({ description })} /><Field label={`Produced record types ${index + 1}.${pi + 1}`} value={production.recordTypes.join("\n")} onChange={(value) => change({ recordTypes: list(value) })} multiline /><Field label={`Produced fields ${index + 1}.${pi + 1}`} value={production.fields.join("\n")} onChange={(value) => change({ fields: list(value) })} multiline /><Field label={`Produced period ${index + 1}.${pi + 1}`} value={production.period} onChange={(period) => change({ period })} /><Field label={`Production notes ${index + 1}.${pi + 1}`} value={production.notes} onChange={(notes) => change({ notes })} multiline /><CitationPicker label={`Production evidence ${index + 1}.${pi + 1}`} values={production.citationIds} evidence={draft.evidence} onChange={(citationIds) => change({ citationIds })} /></div>; })}
        <button type="button" className="analysis-button" onClick={() => patchRequest(index, { productions: [...r.productions, { id: id(), receivedAt: "", description: "", recordTypes: [], fields: [], period: "", citationIds: [], notes: "" }] })}>Record production for request {index + 1}</button><Field label={`Unresolved omissions ${index + 1} (one per line)`} value={r.unresolvedOmissions.join("\n")} onChange={(value) => patchRequest(index, { unresolvedOmissions: list(value) })} multiline /><ul className="list-disc space-y-1 pl-4 text-label leading-4 text-warning">{requestOmissions(r).map((omission, i) => <li key={i}>{omission}</li>)}</ul>{r.followUpDraft && <details><summary className="text-label leading-4 text-accent">Follow-up from last saved revision</summary><pre className="mt-2 whitespace-pre-wrap text-label leading-4 text-fg-secondary">{r.followUpDraft}</pre><button type="button" className="analysis-button mt-2" onClick={() => downloadFile(`${r.id}-follow-up.txt`, r.followUpDraft)}>Download follow-up draft</button></details>}<RequestFiling caseId={draft.id} request={r} busy={busy} onFile={(requestId) => void perform(() => invokeCommand("export_request_filing_record", { caseId: draft.id, requestId }), (value) => { setFilingRecords((prior) => ({ ...prior, [requestId]: value })); setMessage(`Signed filing record written to ${value.outputPath}.`); })} record={filingRecords[r.id]} /></article>)}</div></details>
      <details><summary className="font-semibold text-fg">Dated decisions and follow-through</summary><div className="mt-3 space-y-3">{draft.decisions.map((decision, index) => { const change = (patch: Partial<typeof decision>) => update({ decisions: draft.decisions.map((d, i) => i === index ? { ...d, ...patch } : d) }); return <article key={decision.id} className="space-y-3 rounded border border-line-strong p-3"><h4 className="text-sm font-semibold text-fg">{decision.body} · {decision.date ? formatShortDate(decision.date) : "Date not recorded"} · {decision.agendaItem}</h4><p className="text-label leading-4 text-warning">{decision.date < today ? "Past agenda: establish the outcome from minutes or a signed action." : "Decision window: verify the current agenda and participation instructions."}</p><p className="text-sm text-fg-secondary">{decision.proposedDecision}</p><Field label={`Decision action ${index + 1}`} value={decision.action} onChange={(action) => change({ action })} multiline /><State label={`Decision state ${index + 1}`} value={decision.state} options={["PROPOSED", "MONITORING", "DECIDED", "UNKNOWN", "NEEDS_REVIEW"]} onChange={(state) => change({ state })} /><Field label={`Recorded decision outcome ${index + 1}`} value={decision.outcome} onChange={(outcome) => change({ outcome })} multiline /><Field label={`Recorded vote ${index + 1}`} value={decision.vote} onChange={(vote) => change({ vote })} /><CitationPicker label={`Agenda citations ${index + 1}`} values={decision.citationIds} evidence={draft.evidence} onChange={(citationIds) => change({ citationIds })} /><CitationPicker label={`Outcome citations ${index + 1}`} values={decision.outcomeCitationIds} evidence={draft.evidence} onChange={(outcomeCitationIds) => change({ outcomeCitationIds })} /></article>; })}</div>{draft.decisions.length === 0 && <p className="mt-3 text-sm text-muted">No dated agenda opportunity was established in the source dossier.</p>}</details>
      <details><summary className="font-semibold text-fg">Commitments and audit recommendations</summary><div className="mt-3 space-y-3">{draft.commitments.map((commitment, index) => <article key={commitment.id} className="space-y-3 rounded border border-line-strong p-3"><Field label={`Promised action ${index + 1}`} value={commitment.promise} onChange={(promise) => patchCommitment(index, { promise })} multiline /><Field label={`Responsible party ${index + 1}`} value={commitment.responsibleParty} onChange={(responsibleParty) => patchCommitment(index, { responsibleParty })} /><Field type="date" label={`Commitment due date ${index + 1}`} value={commitment.dueDate ?? ""} onChange={(value) => patchCommitment(index, { dueDate: value || null })} /><Field label={`Evidence needed to establish completion ${index + 1}`} value={commitment.completionTest} onChange={(completionTest) => patchCommitment(index, { completionTest })} multiline /><State label={`Commitment state ${index + 1}`} value={commitment.state} options={["UNKNOWN", "IN_PROGRESS", "COMPLETE", "NOT_MET", "NEEDS_REVIEW"]} onChange={(state) => patchCommitment(index, { state })} />{commitment.dueDate && commitment.dueDate < today && commitment.state === "UNKNOWN" && <p className="text-label leading-4 text-warning">The stated due date has passed; completion remains unknown.</p>}<Field label={`Completion assessment ${index + 1}`} value={commitment.assessment} onChange={(assessment) => patchCommitment(index, { assessment })} multiline /><CitationPicker label={`Original promise citations ${index + 1}`} values={commitment.citationIds} evidence={draft.evidence} onChange={(citationIds) => patchCommitment(index, { citationIds })} /><CitationPicker label={`Completion or noncompletion evidence ${index + 1}`} values={commitment.completionCitationIds} evidence={draft.evidence} onChange={(completionCitationIds) => patchCommitment(index, { completionCitationIds })} /></article>)}</div><button type="button" className="analysis-button mt-3" onClick={() => update({ commitments: [...draft.commitments, { id: id(), promise: "", responsibleParty: "", dueDate: null, completionTest: "", state: "UNKNOWN", assessment: "", citationIds: [], completionCitationIds: [] }] })}>Track a cited commitment</button><p className="mt-2 text-label leading-4 text-muted">Completion or noncompletion requires evidence and a written assessment. A missed date does not by itself establish a broken promise.</p></details>
      <details><summary className="text-sm text-accent">Withhold citations from the next chain-of-custody export ({Object.keys(withheld).length})</summary><p className="mt-2 text-label leading-4 text-muted">A withheld citation is removed from the signed package's evidence, but is still named by label and reason inside it — never silently dropped. Every checked citation needs a reason before export.</p><div className="mt-2 max-h-64 space-y-2 overflow-auto">{draft.evidence.map((cite) => <div key={cite.id} className="rounded border border-line-strong p-2 text-label leading-4 text-fg-secondary"><label className="block"><input type="checkbox" aria-label={`Withhold ${cite.label}`} checked={cite.id in withheld} onChange={(e) => setWithheld((prior) => { const next = { ...prior }; if (e.target.checked) next[cite.id] = ""; else delete next[cite.id]; return next; })} /> {cite.label}</label>{cite.id in withheld && <input aria-label={`Reason for withholding ${cite.label}`} placeholder="Reason (required)" className="analysis-input mt-1 w-full" value={withheld[cite.id]} onChange={(e) => setWithheld((prior) => ({ ...prior, [cite.id]: e.target.value }))} />}</div>)}</div></details>
      <div className="flex flex-wrap gap-3"><button type="button" className="primary-action rounded px-4 py-2 text-sm" disabled={busy || !author.trim()} onClick={() => void perform(() => invokeCommand("save_casework_case", { case: draft, expectedRevision: draft.revision, author }), accept)}>Save case revision</button><button type="button" className="analysis-button" disabled={busy || !author.trim()} onClick={() => void perform(() => invokeCommand("export_casework_packet", { caseId: draft.id, contributor: author }), setExported)}>Prepare portable case packet</button><button type="button" className="analysis-button" disabled={busy || Object.values(withheld).some((reason) => !reason.trim())} onClick={() => void perform(() => invokeCommand("export_case_evidence_package", { caseId: draft.id, targetPath: null, withheldCitations: Object.entries(withheld).map(([citationId, reason]) => ({ citationId, reason })) }), setCustodyExport)}>Export signed chain-of-custody package</button><button type="button" className="analysis-button" disabled={busy} onClick={() => void perform(() => invokeCommand("case_export_chain", { caseId: draft.id }), setChain)}>View export chain</button></div>
      {exported && <div><label className="block text-label leading-4 text-fg-secondary">Portable case JSON<textarea readOnly rows={8} value={exported} className="analysis-input mt-1 w-full font-mono" /></label><button type="button" className="analysis-button mt-2" onClick={() => downloadFile(`${draft.id}-case.json`, exported)}>Download saved case JSON</button><p className="mt-1 text-label leading-4 text-muted">Contains the saved revision, source references, contributor history and merge base. Save current edits before preparing a new packet.</p></div>}
      {custodyExport && <div className="rounded border border-highlight/30 bg-canvas/70 p-3 text-label leading-4"><p className="font-semibold text-highlight">Signed chain-of-custody package written to {custodyExport.outputPath}</p><p className="mt-1 text-muted">Signed revision {custodyExport.caseRevision} — the last <strong>saved</strong> revision. Unsaved edits in this form are never included; save the case first if they should be.</p><p className="mt-1 text-muted">SHA-256 {custodyExport.packageSha256} · signed with {custodyExport.signatureAlgorithm} key {custodyExport.signerKeyId.slice(0, 12)}….</p><p className="mt-1 text-muted">Export ledger position {custodyExport.ledgerSequence}{custodyExport.previousExportSha256 ? <> · chained to prior export {custodyExport.previousExportSha256.slice(0, 12)}…</> : <> · first export in this case's local chain</>}.</p><p className="mt-1 text-muted">Verify independently with <code>node {custodyExport.verifierScriptPath} {custodyExport.outputPath}</code> — this does not require the application, and the script was written next to your reports for whoever receives this package. For a recipient without Node, <code>{custodyExport.verifierPagePath}</code> performs the same checks in a browser with no network access; both prove internal consistency only, not who signed.</p><p className="mt-1 text-muted">{custodyExport.verifiedCitationCount} citation{custodyExport.verifiedCitationCount === 1 ? "" : "s"} confirmed against current sources at export time.</p>{custodyExport.unverifiedCitationLabels.length > 0 && <div className="mt-2 rounded border border-warning/30 bg-warning/10 p-2 text-warning"><p className="font-semibold">{custodyExport.unverifiedCitationLabels.length} citation{custodyExport.unverifiedCitationLabels.length === 1 ? "" : "s"} could not be confirmed and remain marked unverified inside the signed package:</p><ul className="mt-1 list-disc pl-4">{custodyExport.unverifiedCitationLabels.map((label) => <li key={label}>{label}</li>)}</ul></div>}{custodyExport.withheldCitationLabels.length > 0 && <div className="mt-2 rounded border border-danger/30 bg-danger/10 p-2 text-danger"><p className="font-semibold">{custodyExport.withheldCitationLabels.length} citation{custodyExport.withheldCitationLabels.length === 1 ? "" : "s"} withheld by label and reason, not included in the evidence array:</p><ul className="mt-1 list-disc pl-4">{custodyExport.withheldCitationLabels.map((label) => <li key={label}>{label}</li>)}</ul></div>}<p className="mt-2 text-muted">The signature proves these exact bytes have not changed since this installation produced them. It does not attest to the truth of any unverified citation above, and it is not a legal determination of admissibility.</p></div>}
      {chain && <div className="rounded border border-line-strong p-3 text-label leading-4 text-fg-secondary"><p className="font-semibold text-fg">This case's local export ledger ({chain.length} export{chain.length === 1 ? "" : "s"})</p><p className="mt-1 text-muted">{chain.length === 0 ? "No exports to verify yet." : chain.every((entry) => entry.previousLinkValid) ? "Chain intact. Each recorded export names the prior recorded export's hash. This reads this installation's export records, not the exported files; check the files with the bundled verifier script." : <span className="text-danger">Chain broken. A link below does not match. Treat the sequence as tampered or reordered.</span>}</p><ol className="mt-2 space-y-1">{chain.map((entry) => <li key={entry.sequence} className={entry.previousLinkValid ? undefined : "text-danger"}>#{entry.sequence} · {entry.previousLinkValid ? "Linked" : "Link broken"} · {entry.outcome.toLowerCase()} · <time dateTime={entry.reservedAt}>{formatShortDate(entry.reservedAt)}</time> · {entry.previousManifestSha256 ? `chained to ${entry.previousManifestSha256.slice(0, 12)}…` : "first in chain"}</li>)}</ol>{chain.length === 0 && <p className="mt-1 text-muted">No exports recorded for this case yet.</p>}</div>}
      <details><summary className="text-label leading-4 text-muted">Contribution trail</summary><ol className="mt-2 space-y-2 text-label leading-4 text-muted">{state?.events.filter((event) => event.caseId === draft.id).map((event) => <li key={event.revision}>Revision {event.revision} · <time dateTime={event.at}>{formatShortDate(event.at)}</time> · {event.author}: {event.summary}</li>)}</ol></details>
    </div>}
    </>}
    <details>
      <summary className="font-semibold text-fg">Review a shared case packet</summary>
      <p className="mt-2 text-label leading-4 text-muted">
        Paste the exported JSON. Preview checks source references and the revision base. Conflicting
        local work is preserved.
      </p>
      <label className="mt-3 block text-label leading-4 text-fg-secondary">
        Open a shared case file
        <input
          type="file"
          accept=".json,application/json"
          className="mt-2 block w-full text-label leading-4"
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (!file) return;
            gate.invalidate();
            setPreview(null);
            setBusy(false);
            if (file.size > 8 * 1024 * 1024) {
              setError(
                "Case files must be at most 8 MB (signed packets carry the case data twice: once as verifiable bytes, once readable).",
              );
              return;
            }
            void perform(
              () => file.text(),
              (value) => {
                setPayload(value);
                setMessage("Case file loaded. Preview the changes before applying it.");
              },
            );
          }}
        />
      </label>
      <label className="mt-3 block text-label leading-4 text-fg-secondary">
        Incoming case JSON
        <textarea
          aria-label="Incoming case JSON"
          rows={6}
          maxLength={8 * 1024 * 1024}
          value={payload}
          className="analysis-input mt-1 w-full font-mono"
          onChange={(event) => {
            gate.invalidate();
            setBusy(false);
            setPayload(event.target.value);
            setPreview(null);
            setError("");
          }}
        />
      </label>
      <button
        type="button"
        className="analysis-button mt-2"
        disabled={busy || !payload.trim()}
        onClick={() =>
          void perform(() => invokeCommand("preview_casework_import", { payload }), setPreview)
        }
      >
        Preview shared case
      </button>
      {preview && (
        <div className="mt-3 space-y-3">
          <p role="status" className="font-semibold text-accent">
            Import review: {preview.mode.replace(/_/g, " ").toLowerCase()}
          </p>
          <p className="text-sm text-fg-secondary">
            {preview.case.title} · claimed contributor: {preview.contributor}
            {preview.signerKeyId && (
              <>
                {" "}
                · signed by local key <code>{preview.signerKeyId.slice(0, 12)}</code>
              </>
            )}
          </p>
          {preview.signerKeyId && (
            <SignerTrustNotice signerKeyId={preview.signerKeyId} contributors={contributors} />
          )}
          <ul className="list-disc pl-4 text-label leading-4 text-warning">
            {preview.warnings.map((warning, index) => (
              <li key={index}>{warning}</li>
            ))}
          </ul>
          <section aria-label="Changes in shared case" className="space-y-3">
            <h4 className="text-sm font-semibold text-fg">Changes this packet would make</h4>
            {differences.length === 0 ? (
              <p className="text-label leading-4 text-muted">No changes to tracked case fields.</p>
            ) : (
              differences.map((change, index) => (
                <article key={index} className="rounded border border-line-strong p-3">
                  <p className="text-label leading-4 text-accent">
                    {change.section} · {change.kind}
                  </p>
                  <h5 className="mt-1 text-sm font-semibold text-fg">{change.label}</h5>
                  <div className="mt-2 grid gap-3 sm:grid-cols-2">
                    <div>
                      <p className="text-label leading-4 font-semibold text-muted">Local</p>
                      <pre className="mt-1 max-h-60 overflow-auto whitespace-pre-wrap font-sans text-label leading-4 text-fg-secondary">
                        {change.local}
                      </pre>
                    </div>
                    <div>
                      <p className="text-label leading-4 font-semibold text-muted">Incoming</p>
                      <pre className="mt-1 max-h-60 overflow-auto whitespace-pre-wrap font-sans text-label leading-4 text-fg-secondary">
                        {change.incoming}
                      </pre>
                    </div>
                  </div>
                </article>
              ))
            )}
          </section>
          {preview.mode === "CONFLICT" && preview.commuting.length > 0 && (
            <section
              aria-label="Operations that commute with local work"
              className="rounded border border-success/30 p-3"
            >
              <h5 className="text-sm font-semibold text-fg">
                {preview.commuting.length} incoming change
                {preview.commuting.length === 1 ? "" : "s"} touch nothing you changed since the
                shared base
              </h5>
              <p className="mt-1 text-label leading-4 text-muted">
                Each adds or updates an item no local revision has touched since revision{" "}
                {preview.localRevision ?? 0} diverged, as recorded in both operation logs. Applying
                them saves one new revision; the {preview.contested.length} contested change
                {preview.contested.length === 1 ? "" : "s"} below stay for item-by-item merge.
                Authors are labels the other desk supplied, not verified identities.
              </p>
              <ul className="mt-2 list-disc pl-4 text-label leading-4 text-fg-secondary">
                {preview.commuting.map((operation) => (
                  <li key={operation.hash}>
                    {operation.kind.toLowerCase()} {operation.section}: {operation.summary}{" "}
                    <span className="text-muted">
                      · r{operation.revision} · claimed author {operation.author}
                    </span>
                  </li>
                ))}
              </ul>
              <button
                type="button"
                className="analysis-button mt-2"
                disabled={busy}
                onClick={() =>
                  void perform(
                    () =>
                      invokeCommand("apply_commuting_case_operations", {
                        payload,
                        expectedRevision: preview.localRevision,
                        packetSha256: preview.packetSha256,
                        author,
                      }),
                    (value) => {
                      accept(value, "import");
                      setPreview(null);
                      setMessage(
                        `${preview.commuting.length} commuting change${preview.commuting.length === 1 ? "" : "s"} applied as revision ${value.revision}. Preview the packet again to merge what remains.`,
                      );
                    },
                  )
                }
              >
                Apply {preview.commuting.length} commuting change
                {preview.commuting.length === 1 ? "" : "s"}
              </button>
            </section>
          )}
          {draft && draft.id === preview.case.id && (
            <CaseMerge
              local={draft}
              incoming={preview.case}
              onApply={(merged, count) => {
                setDraft(merged);
                setMessage(
                  `${count} item${count === 1 ? "" : "s"} applied to your open draft. Review them, then save a new revision.`,
                );
              }}
            />
          )}
          <details>
            <summary className="text-label leading-4 text-muted">
              Advanced: inspect raw case data
            </summary>
            <div className="mt-2 grid gap-3 sm:grid-cols-2">
              <label className="text-label leading-4 text-fg-secondary">
                Incoming case
                <textarea
                  readOnly
                  className="analysis-input mt-1 w-full font-mono"
                  rows={12}
                  value={JSON.stringify(preview.case, null, 2)}
                />
              </label>
              <label className="text-label leading-4 text-fg-secondary">
                Local case
                <textarea
                  readOnly
                  className="analysis-input mt-1 w-full font-mono"
                  rows={12}
                  value={JSON.stringify(
                    state?.cases.find((c) => c.id === preview.case.id) ?? null,
                    null,
                    2,
                  )}
                />
              </label>
            </div>
          </details>
          {preview.mode === "CONFLICT" ? (
            <p className="text-sm text-warning">
              This packet cannot overwrite local work. Open the local case, compare the versions and
              merge selected changes into a new local revision.
            </p>
          ) : (
            <button
              type="button"
              className="analysis-button"
              disabled={busy || !author.trim()}
              onClick={() =>
                void perform(
                  () =>
                    invokeCommand("import_casework_packet", {
                      payload,
                      expectedRevision: preview.localRevision,
                      packetSha256: preview.packetSha256,
                      author,
                    }),
                  (value) => {
                    accept(value, "import");
                    setPreview(null);
                    setPayload("");
                  },
                )
              }
            >
              Apply reviewed case packet
            </button>
          )}
        </div>
      )}
    </details>
    <CaseSharing activeCase={draft ? { id: draft.id, title: draft.title, revision: draft.revision } : null} author={author} contributors={contributors} onLoadPacket={(value) => { gate.invalidate(); setBusy(false); setPayload(value); setPreview(null); setError(""); }} />
    <details><summary className="font-semibold text-fg">Export signing key</summary><p className="mt-2 text-label leading-4 text-muted">Every signed export and casework packet from this installation uses one local key. Rotating it starts signing with a new key and records the old one's id in a local, signed revocation list. A routine rotation leaves everything the old key already signed trusted; marking it compromised does not.</p>
      <div className="mt-3 grid gap-3 sm:grid-cols-2"><Field label="Reason for rotation" value={rotationReason} onChange={setRotationReason} /><label className="flex items-end gap-2 text-label leading-4 text-fg-secondary pb-1"><input type="checkbox" checked={rotationCompromised} onChange={(e) => setRotationCompromised(e.target.checked)} /> This key is compromised, not just being retired</label></div>
      {rotationCompromised && !confirmingCompromise ? <button type="button" className="analysis-button mt-2" disabled={busy || !rotationReason.trim()} onClick={() => setConfirmingCompromise(true)}>Mark old key compromised…</button> : rotationCompromised ? <div className="mt-2 rounded border border-danger/30 bg-danger/10 p-2 text-label leading-4 text-danger"><p className="font-semibold">This cannot be undone. Every package this installation has ever signed with the current key will read as failing verification everywhere this revocation list is checked, including exports already delivered to someone else.</p><div className="mt-2 flex gap-2"><button type="button" className="analysis-button" disabled={busy} onClick={() => void perform(() => refreshingRevocations(invokeCommand("rotate_export_signing_key", { reason: rotationReason, compromised: true })), (value) => { setRotationResult(value); setRotationReason(""); setRotationCompromised(false); setConfirmingCompromise(false); })}>Confirm: rotate and mark compromised</button><button type="button" className="analysis-button" onClick={() => setConfirmingCompromise(false)}>Cancel</button></div></div> : <button type="button" className="analysis-button mt-2" disabled={busy || !rotationReason.trim()} onClick={() => void perform(() => invokeCommand("rotate_export_signing_key", { reason: rotationReason, compromised: false }), (value) => { setRotationResult(value); setRotationReason(""); setRevocations(null); })}>Rotate key (routine)</button>}
      {rotationResult && <div className="mt-3 rounded border border-highlight/30 bg-canvas/70 p-3 text-label leading-4 text-fg-secondary"><p className="font-semibold text-highlight">Signing key rotated.</p><p className="mt-1">New key: <code>{rotationResult.newKeyId.slice(0, 16)}…</code> · retired key: <code>{rotationResult.retiredKeyId.slice(0, 16)}…</code></p><p className="mt-1">The retired key file was archived, never deleted, at <code>{rotationResult.archivedKeyPath}</code>.</p><p className="mt-1">The signed revocation list is written next to your reports as <code>.onrecord-export-signing-key-revocations-v1.json</code>. Distribute a copy of it to anyone who needs to verify your exports independently: <code>node verify-evidence-package.mjs --revocations &lt;that file&gt; &lt;package&gt;</code>.</p></div>}
      <button type="button" className="analysis-button mt-3" disabled={busy} onClick={() => void perform(() => invokeCommand("get_signing_key_revocations", {}), setRevocations)}>View retired/compromised keys</button>
      {revocations && (() => { const collapsed = worstRevocationPerKey(revocations.entries); return <div className="mt-2 rounded border border-line-strong p-3 text-label leading-4 text-fg-secondary"><p className="font-semibold text-fg">{collapsed.length} recorded key{collapsed.length === 1 ? "" : "s"}</p><ol className="mt-2 space-y-2">{collapsed.map((entry) => <li key={entry.keyId} className={entry.status === "RETIRED" ? undefined : "text-danger"}><div>{entry.status.toLowerCase()} · <code>{entry.keyId.slice(0, 16)}…</code> · <time dateTime={entry.revokedAt}>{formatShortDate(entry.revokedAt)}</time> · {entry.reason}</div>{entry.status === "RETIRED" && (markingCompromisedKeyId === entry.keyId ? <div className="mt-1 flex flex-wrap items-center gap-2"><input aria-label={`Reason this key is compromised`} placeholder="Reason (required)" className="analysis-input" value={markCompromisedReason} onChange={(e) => setMarkCompromisedReason(e.target.value)} /><button type="button" className="analysis-button" disabled={busy || !markCompromisedReason.trim()} onClick={() => void perform(() => refreshingRevocations(invokeCommand("mark_signing_key_compromised", { keyId: entry.keyId, reason: markCompromisedReason })), () => { setMarkingCompromisedKeyId(null); setMarkCompromisedReason(""); })}>Confirm: this key is compromised</button><button type="button" className="analysis-button" onClick={() => { setMarkingCompromisedKeyId(null); setMarkCompromisedReason(""); }}>Cancel</button></div> : <button type="button" className="analysis-button mt-1" onClick={() => setMarkingCompromisedKeyId(entry.keyId)}>Mark compromised…</button>)}</li>)}</ol>{collapsed.length === 0 && <p className="mt-1 text-muted">No keys have been rotated yet.</p>}</div>; })()}
    </details>
  </section>;
}
