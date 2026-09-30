export type StarterEntryKind = 'SOURCE' | 'RECORD_REQUEST_TARGET';

export interface StarterEntry {
  /** Equals the id of the built-in San Diego source preset that carries this entry's crawl policy. */
  id: string;
  kind: StarterEntryKind;
  title: string;
  url: string;
  what: string;
  publisher: string;
  /** `null` means no statement was found, which is not permission to reuse. */
  termsNote: string | null;
  /** YYYY-MM-DD on which the URL was fetched and confirmed. Not a promise it still loads. */
  verifiedAsOf: string;
}

export interface StarterSet {
  id: string;
  title: string;
  summary: string;
  entries: StarterEntry[];
}

export interface StarterSetsReport {
  registryVersion: number;
  sets: StarterSet[];
}
