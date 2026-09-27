import { describe, expect, it } from "vitest";
import type { ResearchPolicy } from "../types";
import { minimumResearchCalls, researchBudgetMessage } from "../utils/researchReadiness";

const policy: ResearchPolicy = {
  enabled: true, autoCollect: true, discoverSources: true, provider: "openai",
  model: "gpt-5.4", intervalHours: 6, dailyCallLimit: 24, researchPasses: 4,
};

describe("research readiness", () => {
  it("reserves discovery only when collection, discovery, and a cloud provider are enabled", () => {
    expect(minimumResearchCalls(policy)).toBe(8);
    expect(minimumResearchCalls({ ...policy, provider: "openrouter" })).toBe(8);
    expect(minimumResearchCalls({ ...policy, provider: "ollama" })).toBe(7);
    expect(minimumResearchCalls({ ...policy, autoCollect: false })).toBe(7);
    expect(minimumResearchCalls({ ...policy, discoverSources: false })).toBe(7);
  });
  it("distinguishes an impossible saved cap from a budget that resets tomorrow", () => {
    expect(researchBudgetMessage({ ...policy, dailyCallLimit: 7 }, 0)).toContain("cannot cover one investigation (8 calls)");
    expect(researchBudgetMessage(policy, 17)).toContain("7 model calls remain today");
    expect(researchBudgetMessage(policy, 17)).toContain("00:00 UTC");
    expect(researchBudgetMessage(policy, 16)).toBeNull();
    expect(researchBudgetMessage({ ...policy, enabled: false, dailyCallLimit: 7 }, 24)).toBeNull();
  });
});
