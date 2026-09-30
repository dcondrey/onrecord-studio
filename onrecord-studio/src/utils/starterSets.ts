import type { PortalRegistration, SavedSource, SourceIngestionStart, StarterEntry, StarterSet } from '../types';
import { sanDiegoGuidance, sanDiegoSourceForUrl } from './sanDiego';

/** Comparable form of a URL: https, lower-case host, no fragment, no trailing slash. */
export function normalizeStarterUrl(value: string): string {
  try {
    const url = new URL(value);
    if (url.protocol === 'http:') url.protocol = 'https:';
    url.hash = '';
    return url.href.replace(/\/$/, '').toLowerCase();
  } catch {
    return '';
  }
}

/**
 * Whether the library already holds this entry. A source is identified by the address it was
 * registered under, so a registration made by hand from the same URL counts: re-adding a set
 * must not create a second copy of it.
 */
export function entryIsSaved(entry: StarterEntry, saved: readonly SavedSource[]): boolean {
  const wanted = normalizeStarterUrl(entry.url);
  if (!wanted) return false;
  return saved.some(({ registration }) =>
    [registration.canonicalUrl, registration.originUrl].some((url) => normalizeStarterUrl(url) === wanted),
  );
}

export function entriesToAdd(set: StarterSet, saved: readonly SavedSource[]): StarterEntry[] {
  return set.entries.filter((entry) => !entryIsSaved(entry, saved));
}

export function entryHost(entry: StarterEntry): string {
  try {
    return new URL(entry.url).hostname;
  } catch {
    return entry.url;
  }
}

export type StarterEntryOutcome =
  | { state: 'started'; source: PortalRegistration }
  /** The source did not load. It is reported as unavailable with the reason, never as empty. */
  | { state: 'unavailable'; reason: string };

export interface StarterAddDeps {
  start: (entry: StarterEntry) => Promise<SourceIngestionStart>;
  /** Called once per entry as soon as its result is known, including after the run was superseded. */
  onResult: (entry: StarterEntry, outcome: StarterEntryOutcome, start: SourceIngestionStart | null) => void;
  /** False once the run was cancelled or replaced; no further entries are started. */
  isCurrent: () => boolean;
}

/**
 * Adds entries one at a time through the ordinary add-source path. A failure marks that entry
 * unavailable and the run continues, so one broken publisher does not hide the others.
 */
export async function addStarterEntries(entries: readonly StarterEntry[], deps: StarterAddDeps): Promise<void> {
  for (const entry of entries) {
    if (!deps.isCurrent()) return;
    try {
      const started = await deps.start(entry);
      deps.onResult(entry, { state: 'started', source: started.source }, started);
    } catch (reason) {
      deps.onResult(entry, { state: 'unavailable', reason: reason instanceof Error ? reason.message : String(reason) }, null);
    }
  }
}

/** Arguments for `start_data_source`, identical to what choosing the same preset by hand sends. */
export function startArgsForEntry(entry: StarterEntry) {
  const preset = sanDiegoSourceForUrl(entry.url);
  if (!preset) throw new Error(`No built-in source preset matches ${entry.url}.`);
  if (preset.access === 'export') throw new Error(`${preset.coverage} Use Import files instead.`);
  return {
    url: preset.url,
    crawlPolicy: preset.crawlPolicy,
    guidance: sanDiegoGuidance(preset),
  };
}
