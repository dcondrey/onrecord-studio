import type { RequestSearchResult, TopicFindingBasis, TopicFindingMatch, TopicPackView } from '../types';

export const BRIEFING_STORAGE_KEY = 'onrecord:briefing-project:v1';

export type BriefingEvidenceKind = 'REQUEST' | 'DOCUMENT_EXCERPT';
export type BriefingEvidenceReadiness = 'LEAD' | 'REVIEWED' | 'READY';

export interface BriefingEvidence {
  id: string;
  kind: BriefingEvidenceKind;
  portalKey: string;
  requestId: number;
  documentId: number | null;
  fileName: string | null;
  characterOffset: number | null;
  sourceUrl: string | null;
  agencyName: string;
  label: string;
  recordedAt: string | null;
  excerpt: string;
  rawFileSha256: string | null;
  claim: string;
  note: string;
  readiness: BriefingEvidenceReadiness;
  addedAt: string;
}

export interface BriefingProject {
  version: 1;
  title: string;
  audience: string;
  meetingDate: string;
  durationMinutes: number;
  objective: string;
  opening: string;
  requestedAction: string;
  evidence: BriefingEvidence[];
  /**
   * True once the auto-draft-from-findings effect has reached a definitive outcome for this
   * project: the project already had evidence, or the findings fetch completed. A transient fetch
   * failure leaves this false so the next mount gets another attempt. Persisted (not
   * component-local) so that deleting every drafted item and remounting the workspace does not
   * read an empty `evidence` array as "never drafted" and resurrect them.
   */
  autoDrafted: boolean;
  /** True if `addBriefingEvidence` has ever evicted an item to stay under its 200-item cap. */
  evidenceTruncated: boolean;
}

export function createDefaultBriefingProject(now = new Date()): BriefingProject {
  return {
    version: 1,
    title: 'San Diego public accountability briefing',
    audience: 'San Diego City Council',
    meetingDate: localDateInput(now),
    durationMinutes: 3,
    objective: 'Present a documented concern about public spending or services, explain its community impact, and request a specific measurable action.',
    opening: '',
    requestedAction: '',
    evidence: [],
    autoDrafted: false,
    evidenceTruncated: false,
  };
}

export function loadBriefingProject(
  storage?: Pick<Storage, 'getItem'>,
  now = new Date(),
): BriefingProject {
  const fallback = createDefaultBriefingProject(now);
  if (!storage) return fallback;
  try {
    const parsed: unknown = JSON.parse(storage.getItem(BRIEFING_STORAGE_KEY) ?? 'null');
    if (!isRecord(parsed) || parsed.version !== 1) return fallback;
    return {
      version: 1,
      title: boundedText(parsed.title, fallback.title, 240),
      audience: boundedText(parsed.audience, fallback.audience, 240),
      meetingDate: dateInput(parsed.meetingDate, fallback.meetingDate),
      durationMinutes: boundedNumber(parsed.durationMinutes, fallback.durationMinutes, 1, 120),
      objective: boundedText(parsed.objective, fallback.objective, 4_000),
      opening: boundedText(parsed.opening, '', 8_000),
      requestedAction: boundedText(parsed.requestedAction, '', 8_000),
      evidence: Array.isArray(parsed.evidence)
        // Keep the newest 200, the same end `addBriefingEvidence` keeps, so a reload shows the
        // evidence that was on screen before it.
        ? parsed.evidence.filter(isBriefingEvidence).slice(-200)
        : [],
      autoDrafted: typeof parsed.autoDrafted === 'boolean' ? parsed.autoDrafted : false,
      evidenceTruncated:
        (typeof parsed.evidenceTruncated === 'boolean' ? parsed.evidenceTruncated : false) ||
        (Array.isArray(parsed.evidence) && parsed.evidence.length > 200),
    };
  } catch {
    return fallback;
  }
}

export function persistBriefingProject(
  project: BriefingProject,
  storage?: Pick<Storage, 'setItem'>,
): void {
  storage?.setItem(BRIEFING_STORAGE_KEY, JSON.stringify(project));
}

export function requestBriefingEvidence(
  record: RequestSearchResult,
  sourceUrl: string | null,
  now = new Date(),
): BriefingEvidence {
  return {
    id: `request:${record.portalKey}:${record.requestId}`,
    kind: 'REQUEST',
    portalKey: record.portalKey,
    requestId: record.requestId,
    documentId: null,
    fileName: null,
    characterOffset: null,
    sourceUrl,
    agencyName: record.agencyName,
    label: record.title || `Request ${record.requestId}`,
    recordedAt: record.createdAt,
    excerpt: record.descriptionPreview,
    rawFileSha256: null,
    claim: '',
    note: '',
    readiness: 'LEAD',
    addedAt: now.toISOString(),
  };
}

export function documentBriefingEvidence(input: {
  portalKey: string;
  requestId: number;
  documentId: number;
  fileName: string;
  characterOffset: number | null;
  sourceUrl: string | null;
  agencyName: string;
  requestTitle: string;
  excerpt: string;
  rawFileSha256: string | null;
  /**
   * Required when `characterOffset` is null: disambiguates two document-level findings (no
   * character-level location) for the same document, which would otherwise collide on the same
   * id and silently deduplicate in `addBriefingEvidence`. Ignored when `characterOffset` is set,
   * since the offset itself is already a distinguishing locator.
   */
  evidenceId?: string;
  now?: Date;
}): BriefingEvidence {
  const now = input.now ?? new Date();
  return {
    id: input.characterOffset === null
      ? `document:${input.portalKey}:${input.documentId}:whole:${input.evidenceId ?? ''}`
      : `document:${input.portalKey}:${input.documentId}:${input.characterOffset}`,
    kind: 'DOCUMENT_EXCERPT',
    portalKey: input.portalKey,
    requestId: input.requestId,
    documentId: input.documentId,
    fileName: input.fileName,
    characterOffset: input.characterOffset,
    sourceUrl: input.sourceUrl,
    agencyName: input.agencyName,
    label: input.requestTitle || input.fileName,
    recordedAt: null,
    excerpt: input.excerpt.trim().slice(0, 1_200),
    rawFileSha256: input.rawFileSha256,
    claim: '',
    note: '',
    readiness: 'LEAD',
    addedAt: now.toISOString(),
  };
}

export function buildBriefingMarkdown(project: BriefingProject, generatedAt = new Date()): string {
  const ready = project.evidence.filter((item) => item.readiness === 'READY');
  const evidence = ready.length > 0 ? ready : project.evidence;
  const minutes = Math.max(1, Math.round(project.durationMinutes));
  const targetWords = minutes * 130;
  const sections = evidence.map((item, index) => {
    // A document-level finding (e.g. a metadata-vs-content contradiction) can be cited
    // without any character-level location, so characterOffset can be null even when
    // documentId is not. `?? 0` would fabricate a precise offset the evidence never had;
    // omit the segment instead of asserting a location nothing measured.
    const locator = item.documentId === null
      ? `${item.portalKey} / request ${item.requestId}`
      : `${item.portalKey} / request ${item.requestId} / document ${item.documentId}` +
        (item.characterOffset === null ? '' : ` / character ${item.characterOffset}`);
    const source = item.sourceUrl ? `\n- Source: ${item.sourceUrl}` : '';
    const digest = item.rawFileSha256 ? `\n- Preserved file SHA-256: ${item.rawFileSha256}` : '';
    const excerpt = item.excerpt ? `\n\n> ${item.excerpt.replace(/\s+/g, ' ').trim()}` : '';
    return `### ${index + 1}. ${item.claim.trim() || item.label}\n\n- Evidence status: ${readinessLabel(item.readiness)} (operator assessment)\n- Local locator: ${locator}\n- Agency: ${item.agencyName}${source}${digest}${excerpt}${item.note.trim() ? `\n\nContext / limitation: ${item.note.trim()}` : ''}`;
  });

  return [
    `# ${project.title.trim() || 'Presentation briefing'}`,
    '',
    `Generated: ${generatedAt.toISOString()}`,
    `Audience: ${project.audience.trim() || 'Not specified'}`,
    `Meeting date: ${project.meetingDate || 'Not specified'}`,
    `Time limit: ${minutes} minute${minutes === 1 ? '' : 's'} (draft toward approximately ${targetWords} spoken words)`,
    '',
    '## Objective',
    '',
    project.objective.trim() || 'Not yet written.',
    '',
    '## Opening',
    '',
    project.opening.trim() || '[Write a one-sentence opening that names the agenda item and why it matters.]',
    '',
    '## Evidence-backed points',
    '',
    sections.length > 0 ? sections.join('\n\n') : '[No evidence has been saved from the Evidence workspace.]',
    '',
    '## Requested action',
    '',
    project.requestedAction.trim() || '[Ask for one specific decision, safeguard, disclosure, or follow-up date.]',
    '',
    '## Verification note',
    '',
    'Evidence-status labels are the operator’s assessments. Request records and excerpts remain traceable to the local OnRecord locator above; a saved lead is not independently corroborated merely because it appears in this outline.',
    '',
  ].join('\n');
}

export const TOPIC_BASIS_LABELS: Record<TopicFindingBasis, string> = {
  COUNTED: 'Counted violation',
  STATED: 'Stated in the record',
  PATTERN: 'Pattern signal, not a legal claim',
};

/**
 * A briefing of one topic's findings, for `buildBriefingMarkdown`. Citations are written as
 * locators only: a detector's quote has not been masked or checked against the stored text, so it
 * stays out of an exported file.
 */
export function topicBriefingProject(topic: TopicPackView, now = new Date()): BriefingProject {
  // Hypotheses about who a redaction hides are never exported, as in the verifiable report.
  const exported = topic.findings.filter((match) => match.finding.kind !== 'REDACTION_CANDIDATE_INFERRED');
  const withheld = topic.findings.length - exported.length;
  return {
    ...createDefaultBriefingProject(now),
    title: `${topic.title}: findings briefing`,
    objective: `Findings whose subject is a request or document tagged "${topic.title}" by its own wording (${topic.cues.join(', ')}).` +
      (withheld > 0 ? ` ${withheld} inferred-redaction finding${withheld === 1 ? ' is' : 's are'} left out: they are hypotheses about who a redaction hides, not facts.` : ''),
    evidence: exported.map((match) => topicFindingEvidence(match, now)),
    autoDrafted: true,
  };
}

function topicFindingEvidence({ basis, anchorRequestId, finding }: TopicFindingMatch, now: Date): BriefingEvidence {
  const confidence = finding.confidence.kind === 'MEASURED'
    ? [`Measured confidence: ${Math.round(finding.confidence.basisPoints / 100)}%`]
    : [];
  const citations = finding.citations.length === 0
    ? ['Citations: none recorded.']
    : ['Citations:', ...finding.citations.map((citation) => {
      const range = citation.characterStart === null
        ? ''
        : ` / characters ${citation.characterStart}${citation.characterEnd === null ? '' : `–${citation.characterEnd}`}`;
      return `- ${citation.portalKey} / document ${citation.documentId}${range}${citation.label ? ` (${citation.label})` : ''}`;
    })];
  return {
    id: `topic-finding:${finding.id}`,
    kind: finding.subject.documentId === null ? 'REQUEST' : 'DOCUMENT_EXCERPT',
    portalKey: finding.subject.portalKey ?? '',
    requestId: anchorRequestId,
    documentId: finding.subject.documentId,
    fileName: null,
    characterOffset: null,
    sourceUrl: null,
    agencyName: 'Not recorded on the finding',
    label: finding.headline,
    recordedAt: finding.lastSeenAt,
    excerpt: '',
    rawFileSha256: null,
    claim: `${TOPIC_BASIS_LABELS[basis]}: ${finding.headline}`,
    note: [`Basis: ${TOPIC_BASIS_LABELS[basis]}.`, ...confidence, finding.explanation, ...citations].join('\n'),
    readiness: 'LEAD',
    addedAt: now.toISOString(),
  };
}

export function localDateInput(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}

function readinessLabel(value: BriefingEvidenceReadiness): string {
  if (value === 'READY') return 'Presentation ready';
  if (value === 'REVIEWED') return 'Source reviewed';
  return 'Saved lead';
}

function isBriefingEvidence(value: unknown): value is BriefingEvidence {
  if (!isRecord(value)) return false;
  return typeof value.id === 'string'
    && (value.kind === 'REQUEST' || value.kind === 'DOCUMENT_EXCERPT')
    && typeof value.portalKey === 'string'
    && Number.isSafeInteger(value.requestId)
    && (value.documentId === null || Number.isSafeInteger(value.documentId))
    && (value.fileName === null || typeof value.fileName === 'string')
    && (value.characterOffset === null || Number.isSafeInteger(value.characterOffset))
    && (value.sourceUrl === null || typeof value.sourceUrl === 'string')
    && typeof value.agencyName === 'string'
    && typeof value.label === 'string'
    && (value.recordedAt === null || typeof value.recordedAt === 'string')
    && typeof value.excerpt === 'string'
    && (value.rawFileSha256 === null || typeof value.rawFileSha256 === 'string')
    && typeof value.claim === 'string'
    && typeof value.note === 'string'
    && (value.readiness === 'LEAD' || value.readiness === 'REVIEWED' || value.readiness === 'READY')
    && typeof value.addedAt === 'string';
}

function boundedText(value: unknown, fallback: string, maximum: number): string {
  return typeof value === 'string' ? value.slice(0, maximum) : fallback;
}

function boundedNumber(value: unknown, fallback: number, minimum: number, maximum: number): number {
  return typeof value === 'number' && Number.isFinite(value)
    ? Math.min(maximum, Math.max(minimum, value))
    : fallback;
}

function dateInput(value: unknown, fallback: string): string {
  return typeof value === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(value) ? value : fallback;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}
