import { describe, expect, it } from 'vitest';
import type { RequestSearchResult } from '../types';
import {
  BRIEFING_STORAGE_KEY,
  buildBriefingMarkdown,
  createDefaultBriefingProject,
  documentBriefingEvidence,
  loadBriefingProject,
  persistBriefingProject,
  requestBriefingEvidence,
} from '../utils/briefing';

const record: RequestSearchResult = {
  requestId: 42,
  portalKey: 'sandiego',
  agencyId: 7,
  agencyName: 'Police Department',
  title: 'Oral-fluid screening protocol',
  descriptionPreview: 'Policy, validation, and retention records.',
  status: 'closed',
  createdAt: '2026-08-30T12:00:00Z',
  closedAt: '2026-09-01T12:00:00Z',
  documentCount: 2,
  integrityFlags: [],
};

describe('presentation briefing', () => {
  it('falls back safely when persisted briefing data is malformed', () => {
    const storage = { getItem: () => '{not json' };
    const project = loadBriefingProject(storage, new Date('2026-09-02T12:00:00-07:00'));

    expect(project.meetingDate).toBe('2026-09-02');
    expect(project.durationMinutes).toBe(3);
    expect(project.evidence).toEqual([]);
  });

  it('flags evidenceTruncated when loading on-disk data that already exceeded the cap', () => {
    const oversized = Array.from({ length: 210 }, (_, index) =>
      requestBriefingEvidence({ ...record, requestId: index }, null));
    const storage = {
      getItem: () => JSON.stringify({ ...createDefaultBriefingProject(), evidence: oversized }),
    };

    const project = loadBriefingProject(storage);

    expect(project.evidence).toHaveLength(200);
    expect(project.evidenceTruncated).toBe(true);
  });

  it('persists and restores bounded evidence records', () => {
    let value: string | null = null;
    const storage = {
      getItem: (key: string) => key === BRIEFING_STORAGE_KEY ? value : null,
      setItem: (key: string, next: string) => {
        if (key === BRIEFING_STORAGE_KEY) value = next;
      },
    };
    const project = createDefaultBriefingProject(new Date('2026-09-02T12:00:00-07:00'));
    project.evidence.push(requestBriefingEvidence(record, 'https://www.sandiego.gov/cpp'));
    persistBriefingProject(project, storage);

    expect(loadBriefingProject(storage).evidence).toEqual(project.evidence);
  });

  it('exports presentation-ready evidence with an exact local locator and digest', () => {
    const project = createDefaultBriefingProject(new Date('2026-09-02T12:00:00-07:00'));
    project.requestedAction = 'Publish the operating protocol before deployment.';
    project.evidence.push(
      documentBriefingEvidence({
        portalKey: 'sandiego',
        requestId: 42,
        documentId: 9,
        fileName: 'protocol.pdf',
        characterOffset: 1_240,
        sourceUrl: 'https://www.sandiego.gov/police',
        agencyName: 'Police Department',
        requestTitle: 'Oral-fluid screening protocol',
        excerpt: 'A screening result does not establish impairment.',
        rawFileSha256: 'abc123',
        now: new Date('2026-09-02T20:00:00Z'),
      }),
    );
    project.evidence[0].readiness = 'READY';
    project.evidence[0].claim = 'The written protocol distinguishes screening from impairment.';
    project.evidence[0].note = 'Confirm the operative revision date.';

    const markdown = buildBriefingMarkdown(project, new Date('2026-09-02T21:00:00Z'));

    expect(markdown).toContain('sandiego / request 42 / document 9 / character 1240');
    expect(markdown).toContain('Preserved file SHA-256: abc123');
    expect(markdown).toContain('Presentation ready (operator assessment)');
    expect(markdown).toContain('Confirm the operative revision date.');
  });

  it('gives two document-level findings for the same document distinct ids, not a fabricated offset', () => {
    // A document-level finding (no character-level location) must not collide with another
    // document-level finding on the same document: both would derive an id from a fabricated
    // `0` offset without `evidenceId`, and the second would silently deduplicate against the
    // first in `addBriefingEvidence`.
    const base = {
      portalKey: 'sandiego',
      requestId: 42,
      documentId: 9,
      fileName: 'protocol.pdf',
      characterOffset: null as number | null,
      sourceUrl: null,
      agencyName: 'Police Department',
      requestTitle: 'Oral-fluid screening protocol',
      rawFileSha256: null,
    };
    const first = documentBriefingEvidence({ ...base, excerpt: 'first finding', evidenceId: 'ev-1' });
    const second = documentBriefingEvidence({ ...base, excerpt: 'second finding', evidenceId: 'ev-2' });

    expect(first.id).not.toBe(second.id);
    expect(first.characterOffset).toBeNull();

    const project = createDefaultBriefingProject(new Date('2026-09-02T12:00:00-07:00'));
    project.evidence.push(first, second);
    const markdown = buildBriefingMarkdown(project, new Date('2026-09-02T21:00:00Z'));
    expect(markdown).toContain('sandiego / request 42 / document 9');
    expect(markdown).not.toContain('character 0');
    expect(markdown).not.toMatch(/document 9 \/ character/);
  });

  it('does not fabricate a character offset for a document-level finding with no character-level citation', () => {
    // Mirrors what findingToBriefingEvidence (BriefingWorkspace.tsx) builds from a detector
    // like metadata_vs_content, which cites a document but not a character range: documentId
    // is set while characterOffset is genuinely null, not merely defaulted.
    const project = createDefaultBriefingProject();
    project.evidence.push({
      id: 'autodraft:finding-1',
      kind: 'DOCUMENT_EXCERPT',
      portalKey: 'sandiego',
      requestId: 42,
      documentId: 9,
      fileName: null,
      characterOffset: null,
      sourceUrl: null,
      agencyName: '',
      label: 'Metadata contradicts content',
      recordedAt: null,
      excerpt: 'Text-cited date: 2026-08-01',
      rawFileSha256: null,
      claim: 'The document text references a later date than its declared creation.',
      note: '',
      readiness: 'LEAD',
      addedAt: '2026-09-02T12:00:00Z',
    });

    const markdown = buildBriefingMarkdown(project);

    expect(markdown).toContain('sandiego / request 42 / document 9');
    expect(markdown).not.toContain('character 0');
    expect(markdown).not.toMatch(/document 9 \/ character/);
  });

  it('does not imply that a saved request lead was corroborated', () => {
    const project = createDefaultBriefingProject();
    project.evidence.push(requestBriefingEvidence(record, null));

    const markdown = buildBriefingMarkdown(project);

    expect(markdown).toContain('Saved lead (operator assessment)');
    expect(markdown).toContain('a saved lead is not independently corroborated');
  });
});
