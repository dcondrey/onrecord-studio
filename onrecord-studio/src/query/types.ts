/**
 * Canonical query types shared by every search surface.
 *
 * The supported release grammar maps exactly onto the bounded flat native query.
 * `compile.ts` additionally exposes filters that cannot be represented because
 * they are negated, duplicated, or ambiguous, so the UI can label them honestly.
 */

export type QuerySource = 'qualifier' | 'natural-language';

export type QueryField = 'agency' | 'portal' | 'status' | 'flag';

export type QueryOperator = 'equals';

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
  portalKey: string | null;
  agencyId: number | null;
  status: string | null;
  policyDeviation: string | null;
  /** Filters that cannot be represented by the current IPC command. */
  deferredFilters: QueryFilter[];
}

export interface QueryAgencyOption {
  agencyId: number;
  agencyName: string;
  portalKey: string;
}
