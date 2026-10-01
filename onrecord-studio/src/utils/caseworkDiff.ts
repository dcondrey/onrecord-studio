import type { CaseworkCase } from "../types/casework";
export interface CaseworkDifference { section: string; label: string; local: string; incoming: string; kind: "changed" | "added" | "removed" }
const display = (value: unknown): string => {
  if (value === null || value === undefined || value === "") return "Not recorded";
  if (Array.isArray(value)) return value.length ? value.map(display).join("\n") : "None recorded";
  return String(value);
};
const fields = (value: object, labels: Record<string, string>) => Object.entries(labels).map(([key, label]) => `${label}: ${display((value as Record<string, unknown>)[key])}`).join("\n");
export function caseworkDifferences(local: CaseworkCase | undefined, incoming: CaseworkCase): CaseworkDifference[] {
  const changes: CaseworkDifference[] = [];
  const compare = (section: string, label: string, before: unknown, after: unknown) => {
    if (JSON.stringify(before) === JSON.stringify(after)) return;
    changes.push({ section, label, local: display(before), incoming: display(after), kind: "changed" });
  };
  const collection = <T extends { id: string }>(section: string, before: T[], after: T[], labels: Record<string, string>, name: (item: T) => string, extra?: (left: T | undefined, right: T | undefined) => void) => {
    const ids = new Set([...before.map((item) => item.id), ...after.map((item) => item.id)]);
    for (const id of ids) {
      const left = before.find((item) => item.id === id); const right = after.find((item) => item.id === id);
      const title = name((right ?? left)!);
      if (!left || !right) changes.push({ section, label: title, local: left ? fields(left, labels) : "Not present locally", incoming: right ? fields(right, labels) : "Removed by incoming version", kind: left ? "removed" : "added" });
      else for (const [key, label] of Object.entries(labels)) compare(section, `${title} · ${label}`, (left as Record<string, unknown>)[key], (right as Record<string, unknown>)[key]);
      extra?.(left, right);
    }
  };
  for (const [key, label] of Object.entries({ title: "Case title", owner: "Case owner", disposition: "Case disposition", notes: "Case notes", topic: "Research topic", reportId: "Founding dossier" })) compare("Case", label, local?.[key as keyof CaseworkCase], incoming[key as keyof CaseworkCase]);
  collection("Research questions", local?.questions ?? [], incoming.questions, { question: "Question", hypothesis: "Hypothesis", nextTest: "Next test", owner: "Owner", disposition: "Disposition" }, (q) => q.question || "Untitled question");
  collection("Records requests", local?.requests ?? [], incoming.requests, { title: "Title", custodian: "Custodian", state: "State", gap: "Missing evidence", resolves: "What it resolves", draft: "Request draft", expectedRecordTypes: "Expected records", expectedFields: "Expected fields", expectedPeriod: "Expected period", submittedAt: "Submission date", responseDue: "Recorded response due", externalReference: "Agency reference", unresolvedOmissions: "Unresolved omissions", compiledDraft: "Compiled CPRA request" }, (r) => r.title, (left, right) => {
    collection(`Productions · ${(right ?? left)!.title}`, left?.productions ?? [], right?.productions ?? [], { receivedAt: "Received date", description: "Description", recordTypes: "Produced records", fields: "Produced fields", period: "Produced period", notes: "Notes" }, (p) => p.description || `Production received ${p.receivedAt || "date not recorded"}`);
  });
  collection("Decisions", local?.decisions ?? [], incoming.decisions, { body: "Decision body", date: "Agenda date", agendaItem: "Agenda item", proposedDecision: "Proposed decision", action: "Next action", state: "State", outcome: "Recorded outcome", vote: "Recorded vote" }, (d) => `${d.body} · ${d.agendaItem}`);
  collection("Commitments", local?.commitments ?? [], incoming.commitments, { promise: "Promised action", responsibleParty: "Responsible party", dueDate: "Due date", completionTest: "Completion test", state: "Assessment state", assessment: "Assessment explanation" }, (c) => c.promise || "Untitled commitment");
  collection("Evidence register", local?.evidence ?? [], incoming.evidence, { label: "Source", reportId: "Saved dossier", quote: "Cited passage", sourceUrl: "Source URL", sourceSha256: "Source SHA-256", verification: "Source verification" }, (c) => c.label);
  const citationLabels = (ids: string[], record: CaseworkCase | undefined) => ids.map((id) => { const cite = record?.evidence.find((e) => e.id === id); return cite ? `${cite.label}: ${cite.quote.slice(0, 200)}${cite.quote.length > 200 ? "…" : ""}` : `Missing citation: ${id}`; });
  for (const [section, before, after, citationFields] of [
    ["Question evidence", local?.questions ?? [], incoming.questions, ["citationIds", "counterCitationIds"]],
    ["Request evidence", local?.requests ?? [], incoming.requests, ["citationIds"]],
    ["Decision evidence", local?.decisions ?? [], incoming.decisions, ["citationIds", "outcomeCitationIds"]],
    ["Commitment evidence", local?.commitments ?? [], incoming.commitments, ["citationIds", "completionCitationIds"]],
  ] as const) {
    const ids = new Set([...before.map((item) => item.id), ...after.map((item) => item.id)]);
    for (const id of ids) for (const field of citationFields) {
      const left = before.find((item) => item.id === id) as unknown as Record<string, unknown> | undefined;
      const right = after.find((item) => item.id === id) as unknown as Record<string, unknown> | undefined;
      const name = String(right?.question ?? right?.title ?? right?.promise ?? right?.agendaItem ?? left?.question ?? left?.title ?? left?.promise ?? left?.agendaItem ?? "Tracked item");
      compare(section, `${name} · ${field === "counterCitationIds" ? "Counter-evidence" : field === "completionCitationIds" ? "Completion evidence" : field === "outcomeCitationIds" ? "Outcome evidence" : "Supporting evidence"}`, citationLabels((left?.[field] as string[] | undefined) ?? [], local), citationLabels((right?.[field] as string[] | undefined) ?? [], incoming));
    }
  }
  for (const requestId of new Set([...(local?.requests ?? []).map((r) => r.id), ...incoming.requests.map((r) => r.id)])) {
    const before = local?.requests.find((r) => r.id === requestId); const after = incoming.requests.find((r) => r.id === requestId);
    for (const productionId of new Set([...(before?.productions ?? []).map((p) => p.id), ...(after?.productions ?? []).map((p) => p.id)])) {
      const left = before?.productions.find((p) => p.id === productionId); const right = after?.productions.find((p) => p.id === productionId);
      compare("Production evidence", `${(after ?? before)!.title} · ${(right ?? left)!.description || "Recorded production"}`, citationLabels(left?.citationIds ?? [], local), citationLabels(right?.citationIds ?? [], incoming));
    }
  }
  for (const evidenceId of new Set([...(local?.evidence ?? []).map((e) => e.id), ...incoming.evidence.map((e) => e.id)])) {
    const left = local?.evidence.find((e) => e.id === evidenceId); const right = incoming.evidence.find((e) => e.id === evidenceId);
    const snapshots = (record: typeof left) => record?.sourceReferences.map((source) => `${source.snapshot}: ${source.url} (SHA-256 ${source.sha256 || "not recorded"})`) ?? [];
    compare("Evidence provenance", `${(right ?? left)!.label} · Original source snapshots`, snapshots(left), snapshots(right));
    compare("Evidence provenance", `${(right ?? left)!.label} · Extracted character locator`, left?.characterOffset, right?.characterOffset);
  }
  return changes;
}
