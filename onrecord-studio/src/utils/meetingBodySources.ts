import type { CrawlPolicy } from '../types';
import registry from '../content/meetingBodies.json';
import { regionalSource } from './regionalSources';
import type { SanDiegoSource } from './sanDiego';

export type PublishingPattern = 'CITY_PAGE_PDF' | 'CITY_FIXED_URL' | 'ONBASE' | 'AGENCY_PAGE';

export interface MeetingBody {
  id: string;
  /** The body's name as its publisher writes it. */
  name: string;
  url: string;
  pattern: PublishingPattern;
  publisher: string;
  /** `null` means the index address came from the City's roster and was not opened. */
  verifiedAsOf: string | null;
  note: string | null;
  headingNames: string[];
}

// The same file the Rust starter set is built from (`src-tauri/src/ingestion/meeting_bodies.rs`).
export const MEETING_BODIES = registry.bodies as readonly MeetingBody[];
export const MEETING_BODY_CATEGORY = 'Boards and commissions';

const singlePage: CrawlPolicy = { maximumPages: 1, maximumDepth: 0, maximumResources: 300, followSupportedExternalPlatforms: false };

const PATTERN_COVERAGE: Record<PublishingPattern, string> = {
  CITY_PAGE_PDF: 'The body’s page and the agenda and minutes files it links. Earlier years on other pages are separate.',
  CITY_FIXED_URL: 'The body’s page and its agenda. The agenda has one address that the City overwrites for each meeting, so collecting again replaces the earlier agenda.',
  ONBASE: 'Meeting pages, agendas and item attachments within the collection limits. A revised docket keeps its address, so collecting again replaces the earlier copy.',
  AGENCY_PAGE: 'The meeting page and the agendas, minutes and reports it links.',
};

function sourceForBody(body: MeetingBody): SanDiegoSource {
  const unverified = body.verifiedAsOf ? '' : ' This address is from the City’s roster of boards and has not been opened to confirm it links agendas.';
  const description = `Agendas and minutes of the ${body.name}.${body.note ? ` ${body.note}` : ''}`;
  const analysisNotes = 'An agenda is not a record of a vote. Keep the body, meeting date and agenda version with each item.';
  const coverage = `${PATTERN_COVERAGE[body.pattern]}${unverified}`;
  if (body.pattern === 'CITY_PAGE_PDF' || body.pattern === 'CITY_FIXED_URL') {
    // No guidance override: `sanDiegoGuidance` limits these to the page and www.sandiego.gov/sites/default/files/.
    return {
      id: body.id, label: body.name, url: body.url, description, coverage, analysisNotes,
      category: MEETING_BODY_CATEGORY, publisher: body.publisher, crawlPolicy: singlePage, pagePrefixes: [body.url],
    };
  }
  if (body.pattern === 'ONBASE') {
    return regionalSource(body.id, body.name, body.url, MEETING_BODY_CATEGORY, body.publisher, description, analysisNotes, { coverage });
  }
  return regionalSource(body.id, body.name, body.url, MEETING_BODY_CATEGORY, body.publisher, description, analysisNotes, {
    coverage, crawlPolicy: singlePage, pagePrefixes: [body.url],
    guidance: { allowedHostnames: [new URL(body.url).hostname], restrictDownloadsToSourceHost: true },
  });
}

export const MEETING_BODY_SOURCES: readonly SanDiegoSource[] = MEETING_BODIES.map(sourceForBody);
