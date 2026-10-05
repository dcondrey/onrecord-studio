import { describe, expect, it } from 'vitest';
import { REGIONAL_SOURCES } from '../utils/regionalSources';
import { HOUSING_SOURCES } from '../utils/housingSources';
import { MEETING_BODIES, MEETING_BODY_SOURCES } from '../utils/meetingBodySources';
import { SAN_DIEGO_SOURCES, sanDiegoGuidance, sanDiegoSourceForUrl } from '../utils/sanDiego';

describe('regional source routing', () => {
  it('covers every requested source with a bounded route and counting guidance', () => {
    const requested = [
      'data.sandiego.gov', 'opendata.sandag.org', 'sandiego.gov', 'sandiegocounty.budget.socrata.com', 'sandiegocounty.gov', 'sandag.org',
      'sandiego.nextrequest.com', 'sandiegohousingcommission.nextrequest.com', 'pra.sandiegocounty.gov', 'sdgis-sandag.opendata.arcgis.com',
      'library.ucsd.edu/dc/collection/bb7998853b', 'gis.sandiego.org/FreightViewer', 'sandiegocounty.gov/content/sdc/hhsa/programs/phs/community_health_statistics.html',
      'arjis.org', 'data.sdcoe.net', 'data.ca.gov', 'rtfhsd.org', '211sandiego.org', 'livewellsd.org/regional-data-hub', 'innovate78.com',
      'opendatasandiego.com', 'escondido.org', 'humandynamics.sdsu.edu', 'sdsheriff.gov/resources/open-data', 'downtownsandiego.org', 'homelessdata.com',
      'sandiego.hylandcloud.com/211agendaonlinecouncil', 'sandiego.granicus.com',
      'sdapcd.org/content/sdapcd/apcd-air-data.html', 'sdirwmp.org/watergis', 'projectcleanwater.org', 'thinkblue.org/by-the-numbers',
      'waterboards.ca.gov/sandiego', 'ssl.netfile.com/static/agency/csd/', 'sdvote.com', 'sandiego.gov/iba/reports', 'sdcers.gov', 'sdcera.org',
      'sdarcc.gov', 'sandiegounified.org/about/budget', 'https://efile.sandiego.gov', 'ftp://massive-ftp.ucsd.edu', 'ftp.sangis.org', 'ftp.sandiego.gov', 'sftp.sandiego.gov',
      'homeleessnesshub.ucsd.edu', 'data.sandiegodata.org', 'voiceofsandiego.org', 'insidesandiego.org', 'elcajoncollaborative.org',
      'transparenthomelessdata.com', 'kpbs.org', 'san-diego-homelessness-analytics.vercel.app', 'ppic.org', 'fox5sandiego.com',
      'calmatters.org', 'sdcda.org', 'hhubsandiego-ucsdonline.hub.arcgis.com', 'timesofsandiego.com', 'cbs8.com', 'hcd.ca.gov',
      'luckyduckfoundation.org', 'inewssoure.org', 'sdrescue.org', 'thecoastnews.com', 'sdvoice.info', 'axios.com', 'powayusd.com',
      'hud.gov', 'sdhc.org', 'sdmts.com', 'datanexus.ucsd.edu', 'catalog.data.gov', 'opendatasandiego.com', 'data.transportation.gov', 'asir.sdsu.edu',
    ];
    expect(REGIONAL_SOURCES).toHaveLength(76);
    expect(new Set(SAN_DIEGO_SOURCES.map((source) => source.id)).size).toBe(SAN_DIEGO_SOURCES.length);
    expect(new Set(SAN_DIEGO_SOURCES.map((source) => source.url)).size).toBe(SAN_DIEGO_SOURCES.length);
    for (const url of requested) {
      const source = sanDiegoSourceForUrl(url);
      expect(source, url).toBeDefined();
      expect(source?.analysisNotes, url).toBeTruthy();
      expect(source?.crawlPolicy.maximumResources).toBeLessThanOrEqual(500);
    }
  });

  it('keeps dataset selection and publisher boundaries instead of inheriting root recipes', () => {
    expect(sanDiegoSourceForUrl('https://opendata.sandag.org/d/krtq-effn')).toBeUndefined();
    expect(sanDiegoSourceForUrl('https://sandiego.gov.evil.test')).toBeUndefined();
    expect(sanDiegoSourceForUrl('https://user:password@sandiego.gov')).toBeUndefined();
    expect(sanDiegoSourceForUrl('javascript:alert(1)')).toBeUndefined();
    expect(sanDiegoSourceForUrl('ftp://unlisted.example')).toBeUndefined();
    expect(sanDiegoSourceForUrl('http://escondido.org/')?.url).toBe('https://www.escondido.gov');
    expect(sanDiegoSourceForUrl('https://sandiego.granicus.com/')?.url).toContain('ViewPublisher.php?view_id=3');
  });

  it('leaves catalog adapters enabled and file transfer sources explicitly on export routes', () => {
    for (const id of ['sandag-open-data', 'ca-open-data', 'sandag-gis', 'ucsd-homelessness-gis', 'federal-transportation-data']) {
      const source = REGIONAL_SOURCES.find((source) => source.id === id)!;
      const guidance = sanDiegoGuidance(source);
      expect(guidance.linkSelector).toBeNull();
      expect(guidance.additionalSeedUrls).toEqual([]);
      // The crawler bounds page following; preset allowlists must not suppress linked CDN files.
      expect(guidance.allowedHostnames).toEqual([]);
      expect(guidance.restrictDownloadsToSourceHost).toBe(false);
      expect(guidance.includeUrlPrefixes).toEqual([]);
    }
    for (const source of REGIONAL_SOURCES.filter((source) => /^(ftp|sftp):/.test(source.url))) {
      expect(source.access).toBe('export');
    }
  });

  it('repairs only registered spelling variants and keeps journalism distinct from City communications', () => {
    expect(sanDiegoSourceForUrl('homeleessnesshub.ucsd.edu')?.url).toBe('https://homelessnesshub.ucsd.edu');
    expect(sanDiegoSourceForUrl('inewssoure.org')?.url).toBe('https://inewsource.org');
    expect(sanDiegoSourceForUrl('inewssoure.org')?.category).toBe('News and reporting');
    expect(sanDiegoSourceForUrl('insidesandiego.org')?.publisher).toBe('City of San Diego');
    expect(sanDiegoSourceForUrl('axios.com')?.url).toBe('https://www.axios.com/local/san-diego');
    expect(sanDiegoSourceForUrl('https://www.axios.com/local/san-diego/2026/example')).toBeUndefined();
    expect(sanDiegoSourceForUrl('https://sdhc.org/unlisted-lowercase.pdf')).toBeUndefined();
    expect(REGIONAL_SOURCES.filter((source) => source.id === 'opendatasandiego')).toHaveLength(1);
    const budget = sanDiegoSourceForUrl('budget.sandiego.gov')!;
    expect(budget.url).toBe('https://budget.sandiego.gov/transparency');
    expect(sanDiegoGuidance(budget).additionalSeedUrls).toContain('https://data.sandiego.gov/departments/department-of-finance/');
    expect(sanDiegoSourceForUrl('https://budget.sandiego.gov/transparency#/sandiegoca/reports/7714')).toBeUndefined();
  });

  it('offers every registered meeting body as a catalogue source at the registry address, scoped to its publisher', () => {
    expect(MEETING_BODIES.length).toBeGreaterThanOrEqual(16);
    expect(MEETING_BODY_SOURCES).toHaveLength(MEETING_BODIES.length);
    for (const body of MEETING_BODIES) {
      const source = sanDiegoSourceForUrl(body.url);
      expect(source?.id, body.name).toBe(body.id);
      expect(source?.label).toBe(body.name);
      expect(source?.access ?? 'public').toBe('public');
      const guidance = sanDiegoGuidance(source!);
      if (body.pattern === 'CITY_PAGE_PDF' || body.pattern === 'CITY_FIXED_URL') {
        expect(source?.crawlPolicy.maximumDepth).toBe(0);
        expect(guidance.allowedHostnames).toEqual(['www.sandiego.gov']);
        expect(guidance.includeUrlPrefixes).toEqual([body.url, 'https://www.sandiego.gov/sites/default/files/']);
      } else if (body.pattern === 'AGENCY_PAGE') {
        expect(guidance.allowedHostnames).toEqual([new URL(body.url).hostname]);
        expect(guidance.restrictDownloadsToSourceHost).toBe(true);
      }
      // An unopened address says so where the operator chooses it.
      expect(/has not been opened/.test(source!.coverage)).toBe(body.verifiedAsOf === null);
    }
  });

  it('retains selected document scope, case-sensitive PDF aliases, fiscal context, and unavailable routes', () => {
    expect(HOUSING_SOURCES).toHaveLength(42);
    for (const source of HOUSING_SOURCES) {
      expect(sanDiegoSourceForUrl(source.url)?.id).toBe(source.id);
      for (const alias of source.aliases ?? []) expect(sanDiegoSourceForUrl(alias)?.id).toBe(source.id);
      if (new URL(source.url).pathname.endsWith('.pdf')) {
        expect(source.crawlPolicy).toMatchObject({ maximumPages: 1, maximumDepth: 0, maximumResources: 1 });
        expect(source.period).toBeTruthy();
      }
    }
    expect(sanDiegoSourceForUrl('sdhc.org/wp-content/uploads/2025/01/fy26-budget-city-format.pdf')).toMatchObject({
      url: 'https://sdhc.org/wp-content/uploads/2025/01/FY26-Budget-City-Format.pdf', evidenceType: 'Budget proposal',
    });
    expect(HOUSING_SOURCES.find((source) => source.id === 'fy26-hssd')?.evidenceType).toBe('Adopted capital budget');
    expect(HOUSING_SOURCES.find((source) => source.id === 'fy25-hssd')?.evidenceType).toBe('Adopted budget');
    expect(HOUSING_SOURCES.find((source) => source.id === 'escondido-meeting-housing-attachment')?.access).toBe('export');
  });
});
