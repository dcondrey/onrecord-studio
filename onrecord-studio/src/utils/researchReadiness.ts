import type { ResearchPolicy } from "../types";

/** Keep this aligned with ResearchPolicy::minimum_run_calls in the native scheduler. */
export function minimumResearchCalls(policy: ResearchPolicy): number {
  const discoversSources = policy.autoCollect && policy.discoverSources
    && (policy.provider === "openai" || policy.provider === "openrouter");
  return policy.researchPasses + 3 + Number(discoversSources);
}

export function researchBudgetMessage(policy: ResearchPolicy, callsToday: number): string | null {
  if (!policy.enabled) return null;
  const required = minimumResearchCalls(policy);
  if (policy.dailyCallLimit < required) {
    return `The daily limit of ${policy.dailyCallLimit} model calls cannot cover one investigation (${required} calls). Raise the limit or reduce research depth in Research settings.`;
  }
  const remaining = Math.max(0, policy.dailyCallLimit - callsToday);
  if (remaining < required) {
    return `${remaining} model calls remain today; starting a complete investigation requires ${required}. The budget resets at 00:00 UTC. Increase the daily limit in Research settings to start another investigation sooner.`;
  }
  return null;
}
