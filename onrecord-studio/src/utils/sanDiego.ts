import { openSourceConnector } from '../components/dashboard/sourceEvents';
import type { CrawlPolicy, SourceGuidance } from '../types';
import { REGIONAL_SOURCES } from './regionalSources';
import { HOUSING_SOURCES } from './housingSources';
import { INVESTIGATION_SOURCES } from './investigationSources';

export interface SanDiegoSource {
  id: string;
  label: string;
  url: string;
  description: string;
  coverage: string;
  crawlPolicy: CrawlPolicy;
  pagePrefixes: string[];
  category?: string;
  publisher?: string;
  analysisNotes?: string;
  evidenceType?: string;
  period?: string;
  access?: 'public' | 'export';
  aliases?: string[];
  guidance?: Partial<SourceGuidance>;
}

const city = 'https://www.sandiego.gov';
const singlePage: CrawlPolicy = {
  maximumPages: 1,
  maximumDepth: 0,
  maximumResources: 300,
  followSupportedExternalPlatforms: false,
};

// These are maintained entry points, not a promise that everything held by the City is online.
// Current calendars keep the app useful after a particular meeting packet becomes historical.
export const SAN_DIEGO_SOURCES: readonly SanDiegoSource[] = [
  {
    id: 'sd-cpp-meetings',
    label: 'CPP agendas and minutes',
    url: `${city}/cpp/meetingsagendas`,
    description: 'Prepare for Commission on Police Practices meetings using the posted agendas, packets, and minutes.',
    coverage: 'The current calendar and its linked documents. Earlier years and video recordings are separate research.',
    crawlPolicy: singlePage,
    pagePrefixes: [`${city}/cpp/meetingsagendas`],
  },
  {
    id: 'sd-police-policies',
    label: 'SDPD policies and procedures',
    url: `${city}/police/data-transparency/policies-procedures`,
    description: 'Find the written policies, procedures, department orders, and training documents behind a police practice.',
    coverage: 'The published policy catalog and its paginated categories. Check each document’s revision date.',
    crawlPolicy: { maximumPages: 100, maximumDepth: 4, maximumResources: 1000, followSupportedExternalPlatforms: false },
    pagePrefixes: [`${city}/police/data-transparency/policies-procedures`],
  },
  {
    id: 'sd-cpp-resources',
    label: 'CPP resources',
    url: `${city}/cpp/reports`,
    description: 'Collect the Commission’s published resources and linked reference documents.',
    coverage: 'This resource page and its linked City documents. External sites and videos are not included.',
    crawlPolicy: singlePage,
    pagePrefixes: [`${city}/cpp/reports`],
  },
  {
    id: 'sd-council-agendas',
    label: 'City Council agendas',
    url: `${city}/city-clerk/officialdocs/legisdocs/brown-act`,
    description: 'Review the City Clerk’s currently posted Council agendas before preparing public comment.',
    coverage: 'Currently posted agendas. Historical Council packets are available through the City Clerk’s separate archive.',
    crawlPolicy: { ...singlePage, maximumResources: 100 },
    pagePrefixes: [`${city}/city-clerk/officialdocs/legisdocs/brown-act`],
  },
  ...REGIONAL_SOURCES,
  ...HOUSING_SOURCES,
  ...INVESTIGATION_SOURCES,
];

export const SAN_DIEGO_RECORDS_URL = 'https://sandiego.nextrequest.com/';

export function sanDiegoSourceForUrl(value: string): SanDiegoSource | undefined {
  try {
    const normalize = (input: string) => {
      const url = new URL(input.includes('://') ? input : `https://${input}`);
      if (!['http:', 'https:', 'ftp:', 'sftp:'].includes(url.protocol) || url.username || url.password) return '';
      if (url.protocol === 'http:') url.protocol = 'https:';
      if (url.hostname === 'budget.sandiego.gov' && url.hash.replace(/^#\/?/, '')) return '';
      url.hash = '';
      return url.href.replace(/\/$/, '');
    };
    const normalized = normalize(value.trim());
    if (!normalized) return undefined;
    return SAN_DIEGO_SOURCES.find((source) => [source.url, ...(source.aliases ?? [])].some((url) => normalize(url) === normalized));
  } catch {
    return undefined;
  }
}

export function sanDiegoGuidance(source: SanDiegoSource): SourceGuidance {
  return {
    reuseForPublisherPattern: false,
    additionalSeedUrls: [],
    allowedHostnames: ['www.sandiego.gov'],
    includeUrlPrefixes: [...source.pagePrefixes, `${city}/sites/default/files/`],
    excludeUrlPrefixes: [],
    linkSelector: 'main a[href]',
    downloadSelector: null,
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
    restrictDownloadsToSourceHost: true,
    ...source.guidance,
  };
}

export function focusSourceComposer() {
  openSourceConnector();
}

export function openFileImport() {
  const desktop = document.getElementById('portal-url-desktop-import-files');
  if (desktop?.getClientRects().length) {
    desktop.click();
    return;
  }
  openSourceConnector();
  window.setTimeout(() => document.getElementById('portal-url-mobile-import-files')?.click(), 0);
}
