import type { ResearchCitation, ResearchFinding, ResearchMeeting, ResearchReport } from "../types";

const normalized = (value: string) => value.replace(/\s+/g, " ").trim();
const genericTerms = new Set("the and for from with that this these those their into after before through whether would could should about which also they have does will were been than then any all are was not but records record provide request retrieve obtain compare including includes include existing related specific reported retrieved report shows show".split(" "));
const terms = (value: string) => new Set((value.toLowerCase().match(/[a-z][a-z-]{2,}/g) ?? []).filter((word) => !genericTerms.has(word)));

/** Shared documents alone are too broad: match the actual next test to the request's scope. */
function requestRelevance(finding: ResearchFinding, request: ResearchReport["synthesis"]["requests"][number]): number {
  const primary = terms(`${finding.title} ${finding.nextTest}`);
  const subject = new Set([...primary, ...terms(finding.interpretation)]);
  const title = terms(request.title);
  const scope = terms(`${request.gap} ${request.resolves}`);
  const draft = terms(request.draft);
  const matched = [...subject].filter((term) => title.has(term) || scope.has(term));
  if (matched.length < 2) return 0;
  return [...subject].reduce((score, term) => score + (primary.has(term) ? 2 : 1) * (Number(title.has(term)) * 4 + Number(scope.has(term)) * 2 + Number(draft.has(term))), 0);
}
export function retainedCitation(report: ResearchReport, citation: ResearchCitation): boolean {
  const evidence = report.evidence.find((source) => source.id === citation.evidenceId);
  if (!evidence || !citation.quote.trim()) return false;
  if (evidence.sql) {
    try {
      const quoted: unknown = JSON.parse(citation.quote);
      const result = JSON.parse(evidence.text);
      if (Array.isArray(quoted) && Array.isArray(result.rows)) {
        return result.rows.some((row: unknown) => JSON.stringify(row) === JSON.stringify(quoted));
      }
    } catch { /* Other retained citations are passages, not full query rows. */ }
  }
  return normalized(evidence.text).includes(normalized(citation.quote));
}

/** A date is a documented agenda reference, not a confirmed event or participation deadline. */
export function citedMeetingDate(report: ResearchReport, meeting: ResearchMeeting): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(meeting.date)) return false;
  const item = normalized(meeting.agendaItem).toLowerCase();
  if (!item || !meeting.citations.every((citation) => retainedCitation(report, citation)) || !meeting.citations.some((citation) => normalized(citation.quote).toLowerCase().includes(item))) return false;
  const date = new Date(`${meeting.date}T12:00:00Z`);
  if (!Number.isFinite(date.valueOf()) || date.toISOString().slice(0, 10) !== meeting.date) return false;
  const [year, month, day] = meeting.date.split("-").map(Number);
  const full = date.toLocaleString("en-US", { month: "long", timeZone: "UTC" });
  const short = date.toLocaleString("en-US", { month: "short", timeZone: "UTC" });
  const forms = [meeting.date, `${full} ${day}, ${year}`, `${full} ${day} ${year}`, `${short} ${day}, ${year}`, `${short} ${day} ${year}`, `${month}/${day}/${year}`, `${String(month).padStart(2, "0")}/${String(day).padStart(2, "0")}/${year}`];
  return meeting.citations.some((citation) => retainedCitation(report, citation) && forms.some((form) => normalized(citation.quote).includes(form)));
}

export interface BriefingFinding {
  report: ResearchReport;
  finding: ResearchFinding;
  index: number;
  citations: ResearchCitation[];
  counterEvidence: ResearchCitation[];
  requests: { index: number; request: ResearchReport["synthesis"]["requests"][number] }[];
  reasons: string[];
  sourceCount: number;
  priority: number;
}

export function researchRunIssues(report: ResearchReport): { summary: string; details: string[] }[] {
  const issues: { summary: string; details: string[] }[] = [];
  const reads = report.investigationLog.filter((entry) => /^(Passage read failed:|Deeper read (unavailable|rejected):)/.test(entry));
  if (reads.length) issues.push({ summary: `${reads.length} deeper document read${reads.length === 1 ? " did" : "s did"} not complete. Surrounding context may be missing.`, details: reads });
  const empty = report.investigationLog.filter((entry) => /^Follow-up search .*: 0 (excerpts|ranked passages)\./.test(entry));
  if (empty.length) issues.push({ summary: `${empty.length} follow-up search${empty.length === 1 ? " returned" : "es returned"} no passages. This does not establish that the requested records do not exist.`, details: empty });
  const queries = report.investigationLog.filter((entry) => /^Comparison .* could not run:/.test(entry));
  if (queries.length) issues.push({ summary: `${queries.length} proposed comparison${queries.length === 1 ? " could" : "s could"} not be executed.`, details: queries });
  const challenge = report.investigationLog.find((entry) => entry.startsWith("Independent evidence challenge:"));
  if (!challenge) issues.push({ summary: "No separate evidence-challenge step is recorded in this saved run. Editorial review alone does not establish corroboration.", details: [] });
  else if (/^Independent evidence challenge: 0 new/.test(challenge)) issues.push({ summary: "The evidence challenge found no additional source evidence. Interpretations remain provisional.", details: [challenge] });
  return issues;
}

/** Readiness for a useful next step, never a probability, truth score or allegation ranking. */
export function briefingFindings(reports: ResearchReport[], states: Record<string, string>, topic = "all"): { findings: BriefingFinding[]; omitted: number } {
  const newest = new Set<string>();
  const seen = new Set<string>();
  const findings: BriefingFinding[] = [];
  let omitted = 0;
  for (const report of [...reports].sort((a, b) => b.createdAt.localeCompare(a.createdAt))) {
    const latest = !newest.has(report.topic);
    newest.add(report.topic);
    if (topic !== "all" && report.topic !== topic) continue;
    report.synthesis.findings.forEach((finding, index) => {
      const state = states[`${report.id}:finding:${index}`] ?? "OPEN";
      if (["DONE", "DISMISSED"].includes(state) || (!latest && state !== "IN_PROGRESS")) return;
      const identity = `${report.topic}:${normalized(finding.title)}:${normalized(finding.interpretation)}`;
      if (seen.has(identity)) return;
      seen.add(identity);
      const citations = finding.citations.filter((citation) => retainedCitation(report, citation));
      if (!citations.length || citations.length !== finding.citations.length) { omitted += 1; return; }
      const counterEvidence = finding.counterEvidence.filter((citation) => retainedCitation(report, citation));
      if (counterEvidence.length !== finding.counterEvidence.length) { omitted += 1; return; }
      const ids = new Set(citations.map((citation) => citation.evidenceId));
      const sources = new Set(citations.flatMap((citation) => {
        const evidence = report.evidence.find((source) => source.id === citation.evidenceId)!;
        return evidence.sourceReferences.length
          ? evidence.sourceReferences.map((source) => `${source.url}|${source.sha256}`)
          : [`${evidence.sourceUrl ?? evidence.id}|${evidence.sourceSha256}`];
      }));
      const requests = report.synthesis.requests.map((request, requestIndex) => ({ request, index: requestIndex, relevance: requestRelevance(finding, request) })).filter(({ request, index: requestIndex, relevance }) => relevance > 0 && request.evidenceIds.some((id) => ids.has(id)) && !["DONE", "DISMISSED"].includes(states[`${report.id}:request:${requestIndex}`] ?? "OPEN")).sort((a, b) => b.relevance - a.relevance || a.index - b.index);
      const reasons = ["Supporting quotations match retained evidence"];
      if (finding.nextTest.trim()) reasons.push("Specific next test recorded");
      if (requests.length) reasons.push("Related records request available");
      if (counterEvidence.length) reasons.push("Counter-evidence included");
      findings.push({ report, finding, index, citations, counterEvidence, requests, reasons, sourceCount: sources.size, priority: Number(Boolean(finding.nextTest.trim())) * 3 + Number(requests.length > 0) * 2 + Number(counterEvidence.length > 0) + Math.min(sources.size, 2) });
    });
  }
  findings.sort((a, b) => b.priority - a.priority || b.report.createdAt.localeCompare(a.report.createdAt) || a.index - b.index);
  // The first reading pass covers distinct investigations; one prolific dossier cannot hide the rest.
  const first: BriefingFinding[] = [];
  const remaining: BriefingFinding[] = [];
  const topics = new Set<string>();
  for (const finding of findings) {
    if (topics.has(finding.report.topic)) remaining.push(finding);
    else { first.push(finding); topics.add(finding.report.topic); }
  }
  return { findings: [...first, ...remaining], omitted };
}
