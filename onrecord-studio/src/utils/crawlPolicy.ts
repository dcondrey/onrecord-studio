// Crawl and pacing settings the connector composer can hand to `start_data_source`.
//
// The bounds below mirror `validate_crawl_policy` (src-tauri/src/ingestion/queue.rs) and
// `validate_policy` (src-tauri/src/ingestion/limiter.rs). They exist so a typo is rejected in the
// dialog instead of after a round trip; the backend remains the authority.

import type { CrawlPolicy, RateLimitPolicy } from '../types';

/** Mirrors `CrawlPolicy::default()` in src-tauri/src/types/source.rs. */
export const DEFAULT_CRAWL_POLICY: CrawlPolicy = {
  maximumPages: 5_000,
  maximumDepth: 8,
  maximumResources: 100_000,
  followSupportedExternalPlatforms: true,
};

/** Mirrors `RateLimitPolicy::default()` in src-tauri/src/types/portal.rs. */
export const DEFAULT_RATE_LIMIT_POLICY: RateLimitPolicy = {
  burstCapacity: 5,
  refillTokensPerSecond: 1,
};

/**
 * Settings are held as raw strings while the dialog is open. A half-typed field is an empty
 * string, not `NaN`, so serde never sees a number the user did not finish entering.
 */
export interface CrawlSettingsDraft {
  maximumDepth: string;
  maximumPages: string;
  maximumResources: string;
  followSupportedExternalPlatforms: boolean;
  burstCapacity: string;
  refillTokensPerSecond: string;
}

export const DEFAULT_CRAWL_SETTINGS_DRAFT: CrawlSettingsDraft = {
  maximumDepth: String(DEFAULT_CRAWL_POLICY.maximumDepth),
  maximumPages: String(DEFAULT_CRAWL_POLICY.maximumPages),
  maximumResources: String(DEFAULT_CRAWL_POLICY.maximumResources),
  followSupportedExternalPlatforms: DEFAULT_CRAWL_POLICY.followSupportedExternalPlatforms,
  burstCapacity: String(DEFAULT_RATE_LIMIT_POLICY.burstCapacity),
  refillTokensPerSecond: String(DEFAULT_RATE_LIMIT_POLICY.refillTokensPerSecond),
};

export type CrawlSettingsResult =
  | { ok: true; crawlPolicy: CrawlPolicy; rateLimitPolicy: RateLimitPolicy }
  | { ok: false; problem: string };

type NumberField = { ok: true; value: number } | { ok: false; problem: string };

function parseWholeNumber(
  value: string,
  label: string,
  lowest: number,
  highest: number,
): NumberField {
  if (!/^\d+$/.test(value.trim())) {
    return { ok: false, problem: `${label} must be a whole number between ${lowest} and ${highest}.` };
  }
  const parsed = Number(value);
  if (parsed < lowest || parsed > highest) {
    return { ok: false, problem: `${label} must be between ${lowest} and ${highest}.` };
  }
  return { ok: true, value: parsed };
}

export function parseCrawlSettings(draft: CrawlSettingsDraft): CrawlSettingsResult {
  const depth = parseWholeNumber(draft.maximumDepth, 'Crawl depth', 0, 32);
  if (!depth.ok) return depth;
  const pages = parseWholeNumber(draft.maximumPages, 'Page limit', 1, 100_000);
  if (!pages.ok) return pages;
  const resources = parseWholeNumber(draft.maximumResources, 'Document limit', 1, 1_000_000);
  if (!resources.ok) return resources;
  const burst = parseWholeNumber(draft.burstCapacity, 'Burst capacity', 1, 100);
  if (!burst.ok) return burst;
  const refill = Number(draft.refillTokensPerSecond.trim());
  if (
    draft.refillTokensPerSecond.trim() === '' ||
    !Number.isFinite(refill) ||
    refill < 0.001 ||
    refill > 100
  ) {
    return { ok: false, problem: 'Requests per second must be between 0.001 and 100.' };
  }
  return {
    ok: true,
    crawlPolicy: {
      maximumDepth: depth.value,
      maximumPages: pages.value,
      maximumResources: resources.value,
      followSupportedExternalPlatforms: draft.followSupportedExternalPlatforms,
    },
    rateLimitPolicy: { burstCapacity: burst.value, refillTokensPerSecond: refill },
  };
}

/**
 * `start_data_source` treats any present crawl policy as an explicit recipe and stops loading the
 * saved connector recipe for that source (src-tauri/src/commands/ingest.rs). An untouched dialog
 * must therefore send `null`, not a default-valued object.
 */
export function crawlPolicyOverride(policy: CrawlPolicy): CrawlPolicy | null {
  const isDefault =
    policy.maximumDepth === DEFAULT_CRAWL_POLICY.maximumDepth &&
    policy.maximumPages === DEFAULT_CRAWL_POLICY.maximumPages &&
    policy.maximumResources === DEFAULT_CRAWL_POLICY.maximumResources &&
    policy.followSupportedExternalPlatforms ===
      DEFAULT_CRAWL_POLICY.followSupportedExternalPlatforms;
  return isDefault ? null : policy;
}
