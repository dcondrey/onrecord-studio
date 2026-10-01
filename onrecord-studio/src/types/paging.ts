import type { CorpusEntityEdge, CorpusEntityKind, CorpusEntityNode, RelationshipBasis } from './corpus';
import type { RequestSearchResult } from './investigation';

/** A page of search matches. `nextCursor` is present exactly when more matches follow. */
export interface AuditRecordPage {
  records: RequestSearchResult[];
  /** Opaque; pass back unchanged. */
  nextCursor: string | null;
  /** Exact number of matches. Carried by the first page only; `null` means "not asked for", never zero. */
  total: number | null;
}

export interface AuditRecordCount {
  total: number;
}

/** The graph's basis and as-of rules, plus an optional kind, evaluated over every neighbour. */
export interface EntityNeighborFilter {
  bases: RelationshipBasis[];
  /** `YYYY-MM-DD`. */
  asOf: string | null;
  includeUndated: boolean;
  kind: CorpusEntityKind | null;
}

export interface EntityNeighborPage {
  center: CorpusEntityNode;
  edges: CorpusEntityEdge[];
  /** Present exactly when more neighbours follow. */
  nextCursor: string | null;
}

export interface EntityNeighborKindCount {
  kind: CorpusEntityKind;
  count: number;
}

export interface EntityNeighborCounts {
  /** Neighbours matching the filter. */
  total: number;
  /** Neighbours before the filter. */
  unfilteredTotal: number;
  byKind: EntityNeighborKindCount[];
}

/** The query a result list was produced by; later pages are requested with exactly these. */
export interface SearchPageArgs {
  searchTerm: string;
  portalKey: string | null;
  agencyId: number | null;
  status: string | null;
  policyDeviation: string | null;
}

/** Where a paged result list stands. `null` paging on the store means the list is complete as given. */
export interface SearchPaging {
  /** Identifies the query that produced the list; a page for any other generation is discarded. */
  generation: number;
  args: SearchPageArgs;
  nextCursor: string | null;
  loadingMore: boolean;
  moreError: string | null;
}
