// Filename, size, content, organization, and date rules an operator can hand to
// `start_data_source` alongside a guided-repair seed URL. Mirrors the new fields on
// `SourceGuidance` (src-tauri/src/types/source.rs) and the exclusion rules they drive
// (`FrontierExclusion` in src-tauri/src/ingestion/queue.rs).

import type { SourceGuidance } from '../types';

/** Held as raw strings while the dialog is open; each list field is comma- or newline-separated. */
export interface GuidanceFilterDraft {
  includeFilenameGlobs: string;
  excludeFilenameGlobs: string;
  includeFilenamePatterns: string;
  excludeFilenamePatterns: string;
  includeContentKeywords: string;
  excludeContentKeywords: string;
  includeContentPatterns: string;
  excludeContentPatterns: string;
  includeOrganizations: string;
  excludeOrganizations: string;
  minimumFileSizeBytes: string;
  maximumFileSizeBytes: string;
  documentDateFrom: string;
  documentDateTo: string;
}

export const DEFAULT_GUIDANCE_FILTER_DRAFT: GuidanceFilterDraft = {
  includeFilenameGlobs: '',
  excludeFilenameGlobs: '',
  includeFilenamePatterns: '',
  excludeFilenamePatterns: '',
  includeContentKeywords: '',
  excludeContentKeywords: '',
  includeContentPatterns: '',
  excludeContentPatterns: '',
  includeOrganizations: '',
  excludeOrganizations: '',
  minimumFileSizeBytes: '',
  maximumFileSizeBytes: '',
  documentDateFrom: '',
  documentDateTo: '',
};

type GuidanceFilterFields = Pick<
  SourceGuidance,
  | 'includeFilenameGlobs'
  | 'excludeFilenameGlobs'
  | 'includeFilenamePatterns'
  | 'excludeFilenamePatterns'
  | 'includeContentKeywords'
  | 'excludeContentKeywords'
  | 'includeContentPatterns'
  | 'excludeContentPatterns'
  | 'includeOrganizations'
  | 'excludeOrganizations'
  | 'minimumFileSizeBytes'
  | 'maximumFileSizeBytes'
  | 'documentDateFrom'
  | 'documentDateTo'
>;

export type GuidanceFilterResult =
  | { ok: true; filters: GuidanceFilterFields }
  | { ok: false; problem: string };

function splitList(value: string): string[] {
  return value
    .split(/[\n,]+/)
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0);
}

// Regexes commonly contain commas (for example `\d{1,3}`), so unlike simple keyword/glob lists
// they use newlines only as separators. Splitting every comma silently changes the expression.
function splitPatternLines(value: string): string[] {
  return value
    .split(/\n+/)
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0);
}

type ListResult = { ok: true; value: string[] } | { ok: false; problem: string };

function parsePatternList(value: string, label: string): ListResult {
  const patterns = splitPatternLines(value);
  for (const pattern of patterns) {
    try {
      new RegExp(pattern);
    } catch {
      return { ok: false, problem: `${label} contains an invalid regular expression: ${pattern}` };
    }
  }
  return { ok: true, value: patterns };
}

const ISO_DATE = /^\d{4}-\d{2}-\d{2}$/;

type DateResult = { ok: true; value: string | null } | { ok: false; problem: string };

function parseDate(value: string, label: string): DateResult {
  const trimmed = value.trim();
  if (trimmed === '') return { ok: true, value: null };
  if (!ISO_DATE.test(trimmed)) return { ok: false, problem: `${label} must be a date in YYYY-MM-DD form.` };
  const [year, month, day] = trimmed.split('-').map(Number);
  const parsed = new Date(Date.UTC(year, month - 1, day));
  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() + 1 !== month ||
    parsed.getUTCDate() !== day
  ) {
    return { ok: false, problem: `${label} must be a valid calendar date.` };
  }
  return { ok: true, value: trimmed };
}

type ByteResult = { ok: true; value: number | null } | { ok: false; problem: string };

function parseByteCount(value: string, label: string): ByteResult {
  const trimmed = value.trim();
  if (trimmed === '') return { ok: true, value: null };
  if (!/^\d+$/.test(trimmed)) return { ok: false, problem: `${label} must be a whole number of bytes.` };
  const parsed = Number(trimmed);
  if (!Number.isSafeInteger(parsed)) {
    return { ok: false, problem: `${label} is too large to represent exactly.` };
  }
  return { ok: true, value: parsed };
}

export function parseGuidanceFilters(draft: GuidanceFilterDraft): GuidanceFilterResult {
  const includeFilenamePatterns = parsePatternList(draft.includeFilenamePatterns, 'Filename include pattern');
  if (!includeFilenamePatterns.ok) return includeFilenamePatterns;
  const excludeFilenamePatterns = parsePatternList(draft.excludeFilenamePatterns, 'Filename exclude pattern');
  if (!excludeFilenamePatterns.ok) return excludeFilenamePatterns;

  const includeContentPatterns = parsePatternList(draft.includeContentPatterns, 'Content include pattern');
  if (!includeContentPatterns.ok) return includeContentPatterns;
  const excludeContentPatterns = parsePatternList(draft.excludeContentPatterns, 'Content exclude pattern');
  if (!excludeContentPatterns.ok) return excludeContentPatterns;

  const minimumFileSizeBytes = parseByteCount(draft.minimumFileSizeBytes, 'Minimum file size');
  if (!minimumFileSizeBytes.ok) return minimumFileSizeBytes;
  const maximumFileSizeBytes = parseByteCount(draft.maximumFileSizeBytes, 'Maximum file size');
  if (!maximumFileSizeBytes.ok) return maximumFileSizeBytes;
  if (
    minimumFileSizeBytes.value !== null &&
    maximumFileSizeBytes.value !== null &&
    minimumFileSizeBytes.value > maximumFileSizeBytes.value
  ) {
    return { ok: false, problem: 'Minimum file size must not exceed maximum file size.' };
  }

  const documentDateFrom = parseDate(draft.documentDateFrom, 'Document date from');
  if (!documentDateFrom.ok) return documentDateFrom;
  const documentDateTo = parseDate(draft.documentDateTo, 'Document date to');
  if (!documentDateTo.ok) return documentDateTo;
  if (
    documentDateFrom.value !== null &&
    documentDateTo.value !== null &&
    documentDateFrom.value > documentDateTo.value
  ) {
    return { ok: false, problem: 'Document date from must not be after document date to.' };
  }

  return {
    ok: true,
    filters: {
      includeFilenameGlobs: splitList(draft.includeFilenameGlobs),
      excludeFilenameGlobs: splitList(draft.excludeFilenameGlobs),
      includeFilenamePatterns: includeFilenamePatterns.value,
      excludeFilenamePatterns: excludeFilenamePatterns.value,
      includeContentKeywords: splitList(draft.includeContentKeywords),
      excludeContentKeywords: splitList(draft.excludeContentKeywords),
      includeContentPatterns: includeContentPatterns.value,
      excludeContentPatterns: excludeContentPatterns.value,
      includeOrganizations: splitList(draft.includeOrganizations),
      excludeOrganizations: splitList(draft.excludeOrganizations),
      minimumFileSizeBytes: minimumFileSizeBytes.value,
      maximumFileSizeBytes: maximumFileSizeBytes.value,
      documentDateFrom: documentDateFrom.value,
      documentDateTo: documentDateTo.value,
    },
  };
}

export function guidanceFiltersAreCustom(draft: GuidanceFilterDraft): boolean {
  return (Object.keys(DEFAULT_GUIDANCE_FILTER_DRAFT) as (keyof GuidanceFilterDraft)[]).some(
    (field) => draft[field] !== DEFAULT_GUIDANCE_FILTER_DRAFT[field],
  );
}
