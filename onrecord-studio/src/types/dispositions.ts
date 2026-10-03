/** Mirrors `Disposition` in `src-tauri/src/analysis/dispositions.rs`. */
export type Disposition =
  | 'RELEASED'
  | 'RELEASED_IN_PART'
  | 'WITHHELD'
  | 'NO_RECORDS'
  | 'WITHDRAWN'
  | 'NOT_CLASSIFIED'
  | 'NOT_RECORDED';

export interface DispositionRequest {
  requestId: number;
  rpid: string | null;
}

export interface DispositionBucket {
  disposition: Disposition;
  requests: DispositionRequest[];
}

export interface DepartmentDispositions {
  portalSubdomain: string;
  /** `null` when the requests name no department. */
  agencyId: number | null;
  agencyName: string | null;
  /** Every request of the department in scope: the denominator of every share. */
  totalRequests: number;
  buckets: DispositionBucket[];
}
