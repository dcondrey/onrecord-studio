import type { ResearchReport } from "../types";
import { citedMeetingDate } from "./researchBriefing";

export interface ResearchAction {
  key: string;
  reportId: string;
  topic: string;
  section: "requests" | "meetings";
  title: string;
  purpose: string;
  date: string | null;
  state: string;
  supportingFindings: number;
  itemIndex: number;
}

/** Keep active older work visible while taking new suggestions from the latest case per topic. */
export function researchAgenda(
  reports: ResearchReport[],
  states: Record<string, string>,
  today: string,
): ResearchAction[] {
  const latest = new Set<string>();
  const seen = new Set<string>();
  const actions: ResearchAction[] = [];
  for (const report of [...reports].sort((a, b) =>
    b.createdAt.localeCompare(a.createdAt),
  )) {
    const newest = !latest.has(report.topic);
    latest.add(report.topic);
    for (const [section, items] of [
      ["requests", report.synthesis.requests],
      ["meetings", report.synthesis.meetings],
    ] as const) {
      items.forEach((item, index) => {
        const key = `${report.id}:${section === "requests" ? "request" : "meeting"}:${index}`;
        const state = states[key] ?? "OPEN";
        if (
          state === "DONE" ||
          state === "DISMISSED" ||
          (!newest && state !== "IN_PROGRESS")
        )
          return;
        const meeting = "agendaItem" in item;
        if (meeting && !citedMeetingDate(report, item)) return;
        const identity = meeting
          ? `${item.body}|${item.date}|${item.agendaItem}`
          : `${item.custodian}|${item.draft}`;
        if (seen.has(identity)) return;
        seen.add(identity);
        const evidence = meeting
          ? item.citations.map((c) => c.evidenceId)
          : item.evidenceIds;
        actions.push({
          key,
          reportId: report.id,
          itemIndex: index,
          topic: report.topic,
          section,
          state,
          title: meeting ? item.agendaItem : item.title,
          purpose: meeting ? item.action : item.resolves,
          date: meeting ? item.date : null,
          supportingFindings: report.synthesis.findings.filter((f) =>
            f.citations.some((c) => evidence.includes(c.evidenceId)),
          ).length,
        });
      });
    }
  }
  const rank = (item: ResearchAction) =>
    item.date && item.date >= today
      ? 0
      : item.state === "IN_PROGRESS"
        ? 1
        : item.section === "requests"
          ? 2
          : 3;
  return actions.sort(
    (a, b) =>
      rank(a) - rank(b) ||
      (rank(a) === 0
        ? (a.date ?? "").localeCompare(b.date ?? "")
        : b.supportingFindings - a.supportingFindings),
  );
}
