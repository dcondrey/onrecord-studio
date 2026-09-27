import type {
  ResearchCitation,
  ResearchEvidence,
  ResearchReport,
} from "../types";

export const RESEARCH_REVIEW_NOTICE =
  "Research interpretations require human review. Exact quotation checks do not establish that an interpretation is correct. Follow-through states record workflow progress, not verification of a claim or proof that a request was submitted.";

const plain = (value: string) =>
  value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/([\\`*_{}[\]()#+.!|~-])/g, "\\$1");
const inline = (value: string) => plain(value.replace(/\s+/g, " ").trim());
const code = (value: string, language = "text") => {
  const longest = Math.max(0, ...Array.from(value.matchAll(/`+/g), (m) => m[0].length));
  const fence = "`".repeat(Math.max(3, longest + 1));
  return `${fence}${language}\n${value}\n${fence}`;
};
const link = (value: string) => {
  try {
    const url = new URL(value);
    return /^https?:$/.test(url.protocol)
      ? `<${url.href.replace(/</g, "%3C").replace(/>/g, "%3E")}>`
      : "No public HTTP source link recorded";
  } catch {
    return "No public HTTP source link recorded";
  }
};
export function researchActionLabel(value?: string): string {
  return (
    {
      OPEN: "To do",
      IN_PROGRESS: "In progress",
      DONE: "Completed",
      DISMISSED: "Dismissed",
    }[value ?? "OPEN"] ?? "Unrecognized state"
  );
}

/** Only the selected case's action states belong in a portable case packet. */
export function researchCaseSnapshot(
  report: ResearchReport,
  states: Record<string, string>,
  exportedAt: string,
) {
  const actionStates: Record<string, string> = {};
  for (const [kind, items] of [
    ["finding", report.synthesis.findings],
    ["request", report.synthesis.requests],
    ["meeting", report.synthesis.meetings],
  ] as const) {
    items.forEach((_, index) => {
      const key = `${report.id}:${kind}:${index}`;
      actionStates[key] = states[key] ?? "OPEN";
    });
  }
  return {
    schemaVersion: 1,
    exportedAt,
    reviewNotice: RESEARCH_REVIEW_NOTICE,
    report,
    actionStates,
  };
}

/** Requests are connected through shared cited sources, not an inferred proof relationship. */
export function requestFindingLinks(report: ResearchReport, evidenceIds: string[]) {
  return report.synthesis.findings.flatMap((finding, index) =>
    [...finding.citations, ...finding.counterEvidence].some((citation) =>
      evidenceIds.includes(citation.evidenceId),
    )
      ? [{ index, title: finding.title }]
      : [],
  );
}

function citedQueryTable(citation: ResearchCitation, evidence?: ResearchEvidence) {
  if (!evidence?.sql) return null;
  try {
    const row: unknown = JSON.parse(citation.quote);
    const result: { columns: unknown } = JSON.parse(evidence.text);
    if (
      !Array.isArray(row) ||
      !Array.isArray(result.columns) ||
      !result.columns.every((column) => typeof column === "string") ||
      row.length !== result.columns.length ||
      !row.length
    )
      return null;
    return [
      `| ${result.columns.map((column) => inline(column)).join(" | ")} |`,
      `| ${row.map(() => "---").join(" | ")} |`,
      `| ${row
        .map((cell) =>
          cell === null
            ? "Not established"
            : inline(typeof cell === "object" ? JSON.stringify(cell) : String(cell)),
        )
        .join(" | ")} |`,
      "A missing value is not zero. Exact cited row:",
      code(citation.quote, "json"),
    ].join("\n\n").replace(/ \|\n\n\|/g, " |\n|");
  } catch {
    return null;
  }
}

export function buildResearchMarkdown(
  report: ResearchReport,
  states: Record<string, string>,
  exportedAt = new Date().toISOString(),
): string {
  const synthesis = report.synthesis;
  const source = (id: string) => {
    const index = report.evidence.findIndex((evidence) => evidence.id === id);
    return index < 0
      ? `Source unavailable: ${inline(id)}`
      : `[E${index + 1} · ${inline(report.evidence[index].label)}](#e${index + 1})`;
  };
  const sources = (ids: string[]) =>
    [...new Set(ids)].map(source).join("; ") || "No source cited";
  const citations = (items: ResearchCitation[]) =>
    items.length
      ? items
          .map((citation) => {
            const evidence = report.evidence.find((item) => item.id === citation.evidenceId);
            return `${source(citation.evidenceId)}\n\n${citedQueryTable(citation, evidence) ?? code(citation.quote)}`;
          })
          .join("\n\n")
      : "None retrieved. This does not establish that none exists.";
  const state = (kind: string, index: number) =>
    researchActionLabel(states[`${report.id}:${kind}:${index}`]);
  const lines = [
    `# ${inline(synthesis.title)}`,
    plain(synthesis.summary),
    `Case: ${inline(report.id)} · Topic: ${inline(report.topic)}`,
    `Investigation created: ${inline(report.createdAt)}\n\nPacket exported: ${inline(exportedAt)}\n\nModel: ${inline(report.model)}`,
    RESEARCH_REVIEW_NOTICE,
    "## Casework index",
    "[Findings](#findings) · [Research questions](#research-questions) · [Records requests](#records-requests) · [Meetings and follow-through](#meetings-and-follow-through) · [Evidence register](#evidence-register) · [Limits and investigation trail](#limits-and-investigation-trail)",
    ...synthesis.findings.map((finding, index) =>
      `- [F${index + 1} · ${inline(finding.title)}](#f${index + 1}) — ${state("finding", index)}`,
    ),
    ...synthesis.requests.map((request, index) =>
      `- [R${index + 1} · ${inline(request.title)}](#r${index + 1}) — ${state("request", index)}`,
    ),
    ...synthesis.meetings.map((meeting, index) =>
      `- [M${index + 1} · ${inline(meeting.date)} · ${inline(meeting.agendaItem)}](#m${index + 1}) — ${state("meeting", index)}`,
    ),
    "## Findings",
    ...(synthesis.findings.length ? synthesis.findings.flatMap((finding, index) => [
      `### F${index + 1}`,
      `**${inline(finding.title)}**`,
      `Follow-through: ${state("finding", index)}`,
      plain(finding.interpretation),
      `**Why it matters:** ${plain(finding.whyItMatters)}`,
      "**Supporting citations**",
      citations(finding.citations),
      "**Counter-evidence**",
      citations(finding.counterEvidence),
      "**Alternative explanations to test**",
      finding.alternativeExplanations.length
        ? finding.alternativeExplanations.map((value) => `- ${plain(value)}`).join("\n")
        : "None recorded; alternatives have not been ruled out.",
      `**Next test:** ${plain(finding.nextTest)}`,
    ]) : ["No finding established in this investigation."]),
    "## Research questions",
    ...(synthesis.questions.length ? synthesis.questions.flatMap((question, index) => [
      `### Q${index + 1}`,
      plain(question.question),
      `**Why pursue it:** ${plain(question.why)}`,
      `Sources: ${sources(question.evidenceIds)}`,
    ]) : ["No additional questions recorded."]),
    "## Records requests",
    "Drafts require scope and recipient review. Exporting this packet does not submit a request.",
    ...(synthesis.requests.length ? synthesis.requests.flatMap((request, index) => {
      const findings = requestFindingLinks(report, request.evidenceIds);
      return [
        `### R${index + 1}`,
        `**${inline(request.title)}**`,
        `Follow-through: ${state("request", index)}`,
        `**Suggested custodian:** ${plain(request.custodian)}`,
        `**Missing evidence:** ${plain(request.gap)}`,
        `**What this could resolve:** ${plain(request.resolves)}`,
        ...(findings.length ? [`Related findings sharing cited sources: ${findings.map((finding) => `[F${finding.index + 1}](#f${finding.index + 1})`).join(", ")}`] : []),
        `Sources: ${sources(request.evidenceIds)}`,
        "**Request draft**",
        code(request.draft),
      ];
    }) : ["No records requests identified in this investigation."]),
    "## Meetings and follow-through",
    "Dates and agenda items are recorded as of the investigation. Verify the current agenda, participation instructions, and outcome at the original source.",
    ...(synthesis.meetings.length ? synthesis.meetings.flatMap((meeting, index) => [
      `### M${index + 1}`,
      `**${inline(meeting.body)} · ${inline(meeting.date)}**`,
      `Agenda item: ${plain(meeting.agendaItem)}`,
      `Follow-through: ${state("meeting", index)}`,
      plain(meeting.relevance),
      `**Recommended action:** ${plain(meeting.action)}`,
      citations(meeting.citations),
    ]) : ["No relevant meeting with a verified date was retrieved."]),
    "## Evidence register",
    "The retained evidence below is the material available to this investigation, not a claim that all relevant records were retrieved. Original-file hashes and snapshot identities support rechecking the source.",
    ...report.evidence.flatMap((evidence, index) => [
      `### E${index + 1}`,
      `**${inline(evidence.label)}**`,
      `Evidence ID: ${inline(evidence.id)}`,
      `Original source: ${evidence.sourceUrl ? link(evidence.sourceUrl) : "No direct document URL recorded; see query source snapshots below."}`,
      `Recorded evidence SHA-256: ${inline(evidence.sourceSha256 || "Not recorded")}`,
      ...(evidence.portalKey !== null ? [`Local locator: portal ${inline(evidence.portalKey)}; request ${evidence.requestId ?? "not recorded"}; document ${evidence.documentId ?? "not recorded"}; extracted character offset ${evidence.characterOffset ?? "not recorded"}. Character offsets are not page numbers.`] : []),
      `**Evidence limitations:** ${plain(evidence.limitations || "Not recorded")}`,
      ...evidence.sourceReferences.map((reference) =>
        `Source snapshot ${inline(reference.snapshot)}: ${link(reference.url)}\n\nOriginal-file SHA-256: ${inline(reference.sha256 || "Not recorded")}`,
      ),
      ...(evidence.snapshotProvenance.length ? ["**Snapshot identities**", code(evidence.snapshotProvenance.join("\n"))] : []),
      ...(evidence.sql ? ["**Reproducible query**", code(evidence.sql, "sql")] : []),
      evidence.sql ? "**Exact retained query result**" : "**Retained source passage**",
      code(evidence.text, evidence.sql ? "json" : "text"),
    ]),
    "## Limits and investigation trail",
    ...(synthesis.limitations.length
      ? synthesis.limitations.map((value) => `- ${plain(value)}`)
      : ["No limitations recorded; coverage has not been established as complete."]),
    "### Investigation steps",
    ...report.investigationLog.map((value, index) => `${index + 1}. ${plain(value)}`),
    `Model usage: ${report.inputTokens} input tokens; ${report.outputTokens} output tokens, including reasoning.`,
  ];
  return `${lines.join("\n\n")}\n`;
}
