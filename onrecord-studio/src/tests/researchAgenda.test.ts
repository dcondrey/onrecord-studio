import { describe, expect, it } from "vitest";
import { researchAgenda } from "../utils/researchAgenda";
import type { ResearchReport } from "../types";
const dossier = (id: string, createdAt: string): ResearchReport => ({
  id,
  createdAt,
  topic: "homelessness",
  model: "fixture",
  evidence: [{ id: "e1", label: "Council agenda", sourceUrl: "https://www.sandiego.gov/agenda.pdf", portalKey: null, requestId: null, documentId: null, characterOffset: null, sourceSha256: "a", text: "September 28, 2026. Item 605: grant amendment.", sql: null, snapshotProvenance: [], sourceReferences: [], limitations: "Fixture" }],
  inputTokens: 0,
  outputTokens: 0,
  investigationLog: [],
  synthesis: {
    title: "A case",
    summary: "",
    findings: [],
    questions: [],
    limitations: [],
    requests: [
      {
        title: "Performance records",
        custodian: "City",
        gap: "Outcomes",
        resolves: "Was the funded service delivered?",
        draft: id,
        evidenceIds: ["e1"],
      },
    ],
    meetings: [
      {
        body: "Council",
        date: "2026-09-28",
        agendaItem: "Item 605",
        relevance: "Grant amendment",
        action: "Compare changed milestones",
        citations: [{ evidenceId: "e1", quote: "September 28, 2026. Item 605: grant amendment." }],
      },
    ],
  },
});
describe("cross-case action queue", () => {
  it("surfaces dated decisions and active older work while retiring obsolete suggestions", () => {
    const older = dossier("older", "2026-09-20");
    const newer = dossier("newer", "2026-09-26");
    const actions = researchAgenda(
      [older, newer],
      { "older:request:0": "IN_PROGRESS" },
      "2026-09-26",
    );
    expect(actions.map((a) => a.key)).toEqual([
      "newer:meeting:0",
      "older:request:0",
      "newer:request:0",
    ]);
    expect(
      researchAgenda([older, newer], {}, "2026-09-26").some(
        (a) => a.reportId === "older",
      ),
    ).toBe(false);
  });
  it("never treats historical meetings as upcoming or reopens completed actions", () => {
    const report = dossier("case", "2026-09-26");
    expect(
      researchAgenda([report], {}, "2026-10-01").map((a) => a.section),
    ).toEqual(["requests", "meetings"]);
    expect(
      researchAgenda(
        [report],
        { "case:request:0": "DONE", "case:meeting:0": "DISMISSED" },
        "2026-09-26",
      ),
    ).toEqual([]);
  });
});
