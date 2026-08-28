import { describe, expect, it } from 'vitest';

import {
  DEFAULT_GUIDANCE_FILTER_DRAFT,
  guidanceFiltersAreCustom,
  parseGuidanceFilters,
} from '../utils/sourceGuidance';

describe('source guidance parsing', () => {
  it('turns an untouched draft into an explicit empty filter set', () => {
    const result = parseGuidanceFilters(DEFAULT_GUIDANCE_FILTER_DRAFT);
    expect(result).toEqual({
      ok: true,
      filters: {
        includeFilenameGlobs: [],
        excludeFilenameGlobs: [],
        includeFilenamePatterns: [],
        excludeFilenamePatterns: [],
        includeContentKeywords: [],
        excludeContentKeywords: [],
        includeContentPatterns: [],
        excludeContentPatterns: [],
        includeOrganizations: [],
        excludeOrganizations: [],
        minimumFileSizeBytes: null,
        maximumFileSizeBytes: null,
        documentDateFrom: null,
        documentDateTo: null,
      },
    });
    expect(guidanceFiltersAreCustom(DEFAULT_GUIDANCE_FILTER_DRAFT)).toBe(false);
  });

  it('parses lists, byte bounds, and an ordered date range', () => {
    const result = parseGuidanceFilters({
      ...DEFAULT_GUIDANCE_FILTER_DRAFT,
      includeFilenameGlobs: '*.pdf, *.docx\n*.xlsx',
      excludeContentPatterns: `${String.raw`\bSSN\b`}\n${String.raw`\d{1,3}`}`,
      includeOrganizations: 'Parks, Public Works',
      minimumFileSizeBytes: '1024',
      maximumFileSizeBytes: '4096',
      documentDateFrom: '2024-02-29',
      documentDateTo: '2026-08-27',
    });
    expect(result).toMatchObject({
      ok: true,
      filters: {
        includeFilenameGlobs: ['*.pdf', '*.docx', '*.xlsx'],
        excludeContentPatterns: [String.raw`\bSSN\b`, String.raw`\d{1,3}`],
        includeOrganizations: ['Parks', 'Public Works'],
        minimumFileSizeBytes: 1024,
        maximumFileSizeBytes: 4096,
        documentDateFrom: '2024-02-29',
        documentDateTo: '2026-08-27',
      },
    });
  });

  it('rejects malformed patterns, impossible dates, reversed ranges, and imprecise byte counts', () => {
    expect(
      parseGuidanceFilters({
        ...DEFAULT_GUIDANCE_FILTER_DRAFT,
        includeContentPatterns: '[unterminated',
      }),
    ).toMatchObject({ ok: false, problem: expect.stringContaining('invalid regular expression') });
    expect(
      parseGuidanceFilters({ ...DEFAULT_GUIDANCE_FILTER_DRAFT, documentDateFrom: '2026-02-30' }),
    ).toMatchObject({ ok: false, problem: expect.stringContaining('valid calendar date') });
    expect(
      parseGuidanceFilters({
        ...DEFAULT_GUIDANCE_FILTER_DRAFT,
        documentDateFrom: '2026-08-28',
        documentDateTo: '2026-08-27',
      }),
    ).toMatchObject({ ok: false, problem: expect.stringContaining('must not be after') });
    expect(
      parseGuidanceFilters({
        ...DEFAULT_GUIDANCE_FILTER_DRAFT,
        maximumFileSizeBytes: '9007199254740992',
      }),
    ).toMatchObject({ ok: false, problem: expect.stringContaining('too large') });
  });
});
