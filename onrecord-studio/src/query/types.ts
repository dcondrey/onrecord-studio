/**
 * Canonical query types shared by every search surface.
 *
 * NOTE(query): This is intentionally independent from the current IPC payload.
 * The backend still accepts the legacy flat fields today; the adapter in
 * `compile.ts` keeps that boundary explicit until the richer contract lands.
 */

export type QuerySource = 'visual' | 'qualifier' | 'natural-language' | 'imported';

export type QueryField =
  | 'agency'
  | 'portal'
  | 'status'
  | 'after'
  | 'before'
  | 'has'
  | 'flag'
  | 'type';

export type QueryOperator = 'equals' | 'contains' | 'after' | 'before' | 'exists';

export interface QueryFilter {
  field: QueryField;
  operator: QueryOperator;
  value: string;
  negated?: boolean;
  /** Original spelling is retained for editor highlighting and audit trails. */
  raw?: string;
}

export interface QueryIssue {
  message: string;
  token?: string;
  severity: 'error' | 'warning' | 'info';
}

export interface QueryAst {
  version: 1;
  source: QuerySource;
  text: string;
  filters: QueryFilter[];
  issues: QueryIssue[];
}

export interface LegacyQueryFilters {
  searchTerm: string;
  agencyId: number | null;
  status: string | null;
  policyDeviation: string | null;
  /** Filters that cannot be represented by the current IPC command. */
  deferredFilters: QueryFilter[];
}
