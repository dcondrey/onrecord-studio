/** Mirrors src-tauri/src/types/standing_search.rs. */

export interface StandingQueryFilters {
  searchTerm: string;
  portalKey: string | null;
  agencyId: number | null;
  status: string | null;
  policyDeviation: string | null;
}

export interface StandingQueryInput {
  label: string;
  /** The `QueryAst` from `src/query`, serialized verbatim. */
  astJson: string;
  filters: StandingQueryFilters;
}

export interface StandingQuery {
  id: string;
  label: string;
  astJson: string;
  filters: StandingQueryFilters;
  createdAt: string;
}

/** Measured over every match, or scaled from a sample. Never rendered alike. */
export type CountBasis = { basis: 'EXACT' } | { basis: 'ESTIMATED_FROM_SAMPLE'; sampleSize: number };

export interface FacetValue {
  value: string;
  count: number;
}

export interface Facet {
  dimension: string;
  values: FacetValue[];
  truncated: boolean;
}

export interface DocumentCluster {
  extractedTextSha256: string;
  requestCount: number;
  documentCount: number;
  exampleFileName: string;
}

export interface StandingQueryResult {
  /** Always exact. */
  matchCount: number;
  matchesDigest: string;
  countBasis: CountBasis;
  facets: Facet[];
  clusters: DocumentCluster[];
}

export interface StandingQueryRun {
  runId: string;
  queryId: string;
  ranAt: string;
  corpusDigest: string;
  corpusDocumentCount: number;
  result: StandingQueryResult;
  /** `null` on the first run. */
  changedSincePrevious: boolean | null;
  previousMatchCount: number | null;
}

export interface StandingQuerySummary {
  query: StandingQuery;
  lastRun: StandingQueryRun | null;
  currentCorpusDigest: string;
  due: boolean;
}

export type StandingQueryOutcome =
  | { outcome: 'RAN'; run: StandingQueryRun }
  | { outcome: 'NOT_DUE'; queryId: string }
  | { outcome: 'DEFERRED'; queryId: string; reason: string };

export interface SeriesMember {
  entityId: string;
  displayValue: string;
  documentCount: number;
}

export interface SeriesGap {
  label: string;
  prefix: string;
  fromNumber: number;
  toNumber: number;
  width: number;
  before: SeriesMember;
  after: SeriesMember;
  seriesMemberCount: number;
}

export interface SeriesGapReport {
  gaps: SeriesGap[];
  seriesCount: number;
  truncated: boolean;
}
