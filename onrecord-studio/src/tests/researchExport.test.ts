import { describe, expect, it } from "vitest";
import type { ResearchReport } from "../types";
import { buildResearchMarkdown, researchCaseSnapshot } from "../utils/researchExport";

const report: ResearchReport = {
  id: "case-export",
  topic: "police-budget",
  createdAt: "2026-09-26T12:00:00Z",
  model: "fixture",
  inputTokens: 12,
  outputTokens: 7,
  evidence: [
    {
      id: "sql-42",
      label: "Overtime reconciliation",
      sourceUrl: null,
      portalKey: null,
      requestId: null,
      documentId: null,
      characterOffset: null,
      sourceSha256: "result-digest",
      text: '{"columns":["fund_center","actual","revised_authority"],"rows":[["0012","9007199254740993.01",null]]}',
      sql: "SELECT fund_center, actual, revised_authority FROM analysis_actuals",
      snapshotProvenance: ["actuals:2023:immutable-snapshot"],
      sourceReferences: [{
        url: "https://seshat.datasd.org/actuals.csv",
        snapshot: "actuals-2023",
        sha256: "original-file-digest",
      }],
      limitations: "Revised authority was not supplied.",
    },
    {
      id: "doc-7",
      label: "Council agenda",
      sourceUrl: "https://www.sandiego.gov/agenda.pdf",
      portalKey: "city",
      requestId: 12,
      documentId: 27,
      characterOffset: 0,
      sourceSha256: "agenda-digest",
      text: "September 28, 2026. Item 605: authorize the amendment.",
      sql: null,
      snapshotProvenance: [],
      sourceReferences: [],
      limitations: "An agenda proposal does not establish approval.",
    },
  ],
  synthesis: {
    title: "Test the spending authority",
    summary: "The joined records leave a missing counterpart.",
    findings: [{
      title: "Authority remains unresolved",
      interpretation: "Spending alone does not establish an unauthorized payment.",
      whyItMatters: "Obtain authority before alleging an overrun.",
      citations: [{ evidenceId: "sql-42", quote: '["0012","9007199254740993.01",null]' }],
      counterEvidence: [{ evidenceId: "doc-7", quote: "Item 605: authorize the amendment." }],
      alternativeExplanations: ["An amendment may have changed authority."],
      nextTest: "Reconcile approved amendments with the same fiscal period.",
    }],
    questions: [{ question: "Was the amendment approved?", why: "The agenda is a proposal.", evidenceIds: ["doc-7"] }],
    requests: [{
      title: "Approved amendments",
      custodian: "City Clerk",
      gap: "Final authority",
      resolves: "Whether authority covered the payment",
      draft: "Please provide the approved amendment and attachment.\nKeep the native spreadsheet.",
      evidenceIds: ["doc-7"],
    }],
    meetings: [{
      body: "City Council",
      date: "2026-09-28",
      agendaItem: "605",
      relevance: "The proposal concerns the same authority.",
      action: "Obtain the signed resolution and minutes after the meeting.",
      citations: [{ evidenceId: "doc-7", quote: "September 28, 2026. Item 605: authorize the amendment." }],
    }],
    limitations: ["No approved amendment retrieved."],
  },
  investigationLog: ["Compared the retained table with the agenda."],
};
const states = {
  "case-export:finding:0": "IN_PROGRESS",
  "case-export:request:0": "DONE",
  "unrelated-case:meeting:0": "DISMISSED",
};
const exportedAt = "2026-09-26T15:00:00Z";

describe("portable research case packets", () => {
  it("connects claims, counter-evidence, requests, and meetings to an independently checkable source register", () => {
    const markdown = buildResearchMarkdown(report, states, exportedAt);
    expect(markdown).toContain("[E1 · Overtime reconciliation](#e1)");
    expect(markdown).toContain("[E2 · Council agenda](#e2)");
    expect(markdown).toContain("### E1");
    expect(markdown).toContain("### E2");
    expect(markdown).toContain("Related findings sharing cited sources: [F1](#f1)");
    expect(markdown).toContain("https://seshat.datasd.org/actuals.csv");
    expect(markdown).toContain("original\\-file\\-digest");
    expect(markdown).toContain("actuals:2023:immutable-snapshot");
    expect(markdown).toContain(report.evidence[0].sql);
    expect(markdown).toContain(report.evidence[0].text);
    expect(markdown).toContain(report.synthesis.requests[0].draft);
    expect(markdown).toContain("extracted character offset 0");
    expect(markdown).toContain("Follow-through: In progress");
    expect(markdown).toContain("Follow-through: Completed");
    expect(markdown).toContain("Follow-through: To do");
    expect(markdown).toContain("not verification of a claim");
    expect(markdown).toContain("An amendment may have changed authority");
    expect(markdown).toContain("No approved amendment retrieved");
  });

  it("keeps exact decimals and identifiers while distinguishing missing query values from zero", () => {
    const markdown = buildResearchMarkdown(report, states, exportedAt);
    expect(markdown).toContain("| fund\\_center | actual | revised\\_authority |\n| --- | --- | --- |\n| 0012 | 9007199254740993\\.01 | Not established |");
    expect(markdown).toContain('["0012","9007199254740993.01",null]');
    expect(markdown).not.toContain("9007199254740992");
  });

  it("keeps quotations literal and names a missing source without fabricating a link", () => {
    const modified = structuredClone(report);
    modified.evidence[1].sourceUrl = "javascript:alert('bad')";
    modified.synthesis.findings[0].citations.push({
      evidenceId: "missing",
      quote: "</textarea>\n```\n# This is a literal source quote",
    });
    const markdown = buildResearchMarkdown(modified, states, exportedAt);
    expect(markdown).toContain("Source unavailable: missing");
    expect(markdown).not.toContain("javascript:");
    expect(markdown).toContain("````text\n</textarea>\n```\n# This is a literal source quote\n````");
    expect(markdown).toContain("No public HTTP source link recorded");
  });

  it("preserves the original saved data and scopes workflow states to the exported case", () => {
    const snapshot = JSON.parse(JSON.stringify(researchCaseSnapshot(report, states, exportedAt)));
    expect(snapshot.schemaVersion).toBe(1);
    expect(snapshot.exportedAt).toBe(exportedAt);
    expect(snapshot.report).toEqual(report);
    expect(snapshot.actionStates).toEqual({
      "case-export:finding:0": "IN_PROGRESS",
      "case-export:request:0": "DONE",
      "case-export:meeting:0": "OPEN",
    });
    expect(snapshot.report.evidence[0].text).toContain('"9007199254740993.01",null');
    expect(states).not.toHaveProperty("case-export:meeting:0");
  });
});
