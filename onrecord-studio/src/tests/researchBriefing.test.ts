import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { briefingFindings, citedMeetingDate, researchRunIssues, retainedCitation } from "../utils/researchBriefing";
import type { ResearchReport } from "../types";
const load = (name: string): ResearchReport => JSON.parse(readFileSync(new URL(`../../docs/validation/${name}.json`, import.meta.url), "utf8"));
const homelessness = load("live-research-dossier");
const finance = load("live-finance-dossier");

describe("findings-first briefing built from the saved live dossiers", () => {
  it("exposes the archived dossier's failed deeper reads and missing challenge instead of treating editorial review as corroboration", () => {
    const issues = researchRunIssues(homelessness);
    expect(issues.some((issue) => issue.summary.startsWith("6 deeper document reads did not complete"))).toBe(true);
    expect(issues.some((issue) => issue.summary.startsWith("8 follow-up searches returned no passages"))).toBe(true);
    expect(issues.some((issue) => issue.summary.startsWith("No separate evidence-challenge step"))).toBe(true);
    const challenged = { ...finance, investigationLog: ["Independent evidence challenge: 0 new excerpts/results retained; 0 successful reads, 0 successful queries, 0 failed tools."] };
    expect(researchRunIssues(challenged)[0].summary).toContain("no additional source evidence");
  });
  it("features concrete source-backed discoveries across investigations and keeps their exact numbers", () => {
    const result = briefingFindings([homelessness, finance], {});
    expect(result.omitted).toBe(0);
    expect(result.findings).toHaveLength(homelessness.synthesis.findings.length + finance.synthesis.findings.length);
    expect(new Set(result.findings.slice(0, 2).map((item) => item.report.topic)).size).toBe(2);
    expect(result.findings.some((item) => item.finding.title.includes("Kettner/Vine"))).toBe(true);
    expect(result.findings.flatMap((item) => item.citations).some((citation) => citation.quote.includes("50818708"))).toBe(true);
    expect(result.findings[0].requests.length).toBeGreaterThan(0);
    expect(result.findings[0].finding.nextTest).toBeTruthy();
    expect(result.findings.flatMap((item) => item.counterEvidence).length).toBeGreaterThan(0);
  });
  it("does not multiply the same original snapshots into independent sources", () => {
    const result = briefingFindings([finance], {});
    const repeated = result.findings.find((item) => item.citations.length > 2)!;
    expect(repeated.sourceCount).toBe(2);
  });
  it("matches the actual requested subject instead of picking the first request that cites the same broad report", () => {
    const result = briefingFindings([homelessness], {});
    expect(result.findings.find((item) => item.index === 0)?.requests[0].index).toBe(4);
    expect(result.findings.find((item) => item.index === 2)?.requests[0].index).toBe(3);
    expect(result.findings.find((item) => item.index === 3)?.requests[0].index).toBe(1);
    expect(result.findings.find((item) => item.index === 4)?.requests[0].index).toBe(5);
  });
  it("hides retired and obsolete suggestions while retaining unfinished older work", () => {
    const old = { ...homelessness, id: "old", createdAt: "2020-01-01", synthesis: { ...homelessness.synthesis, findings: [{ ...homelessness.synthesis.findings[0], title: "Older unresolved allocation question" }] } };
    const states = { [`${homelessness.id}:finding:0`]: "DONE", "old:finding:0": "IN_PROGRESS" };
    const result = briefingFindings([old, homelessness], states);
    expect(result.findings.some((item) => item.report.id === "old")).toBe(true);
    expect(result.findings.some((item) => item.report.id === homelessness.id && item.index === 0)).toBe(false);
    expect(briefingFindings([old, homelessness], {}).findings.some((item) => item.report.id === "old")).toBe(false);
  });
  it("will not feature a claim whose quoted amount or opposing evidence has changed", () => {
    const report = structuredClone(finance);
    report.synthesis.findings[0].citations[0].quote = '[2023,"Police","584706166","588906166",null]';
    expect(retainedCitation(report, report.synthesis.findings[0].citations[0])).toBe(false);
    expect(briefingFindings([report], {}).omitted).toBe(1);
    const opposed = structuredClone(homelessness);
    opposed.synthesis.findings[0].counterEvidence[0].quote = "This opposing passage was never retrieved.";
    expect(briefingFindings([opposed], {}).findings.some((item) => item.index === 0)).toBe(false);
  });
  it("requires the dated agenda passage and refuses invalid calendar dates or uncited dates", () => {
    const meeting = homelessness.synthesis.meetings[0];
    expect(citedMeetingDate(homelessness, meeting)).toBe(true);
    expect(citedMeetingDate(homelessness, { ...meeting, date: "2026-02-30" })).toBe(false);
    expect(citedMeetingDate(homelessness, { ...meeting, date: "2027-09-28" })).toBe(false);
    expect(citedMeetingDate(homelessness, { ...meeting, agendaItem: "Item 999" })).toBe(false);
    expect(citedMeetingDate(homelessness, { ...meeting, citations: [] })).toBe(false);
  });
});
