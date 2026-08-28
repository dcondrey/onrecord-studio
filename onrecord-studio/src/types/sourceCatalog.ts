/**
 * Public acquisition paths implemented by the current ingestion runtime.
 *
 * These choices are discovery hints, not provider endorsements or access bypasses. Every URL is
 * still subject to HTTPS/host safety checks, robots policy, crawl limits, and the source's public
 * accessibility. Credentialed and provider-specific systems do not appear here.
 */
export interface BuiltInSourceOption {
  id: string;
  label: string;
  description: string;
  placeholder: string;
}

export const BUILT_IN_SOURCE_OPTIONS: BuiltInSourceOption[] = [
  {
    id: 'auto',
    label: 'Public URL · auto-detect',
    description: 'Detect a public NextRequest portal, catalog, API, page, or file.',
    placeholder: 'Add a public URL, catalog, API, or document',
  },
  {
    id: 'foia',
    label: 'FOIA / reading room',
    description: 'Crawl a public agency reading room or document index.',
    placeholder: 'Paste the public agency reading-room URL',
  },
  {
    id: 'open-data',
    label: 'CKAN / Socrata catalog',
    description: 'Detect and page a public CKAN or Socrata dataset catalog.',
    placeholder: 'Paste the public CKAN or Socrata catalog URL',
  },
  {
    id: 'arcgis',
    label: 'ArcGIS Hub / REST',
    description: 'Discover public ArcGIS Hub links and page FeatureServer or MapServer layers.',
    placeholder: 'Paste the public ArcGIS Hub or numbered layer URL',
  },
  {
    id: 'tableau',
    label: 'Public Tableau view',
    description: 'Inspect a publicly readable Tableau page and its discoverable downloads.',
    placeholder: 'Paste the public Tableau view URL',
  },
  {
    id: 'google-drive',
    label: 'Public Google Drive',
    description: 'Inspect a publicly readable Drive page or embedded public folder feed.',
    placeholder: 'Paste the public Google Drive URL',
  },
  {
    id: 'direct-file',
    label: 'Direct file URL',
    description: 'Download one publicly reachable document or supported archive.',
    placeholder: 'Paste the direct public file URL',
  },
];
