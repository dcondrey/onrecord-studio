import { SAN_DIEGO_SOURCES } from '../utils/sanDiego';

export interface BuiltInSourceOption {
  id: string;
  label: string;
  description: string;
  placeholder: string;
  url?: string;
}

export const BUILT_IN_SOURCE_OPTIONS: BuiltInSourceOption[] = [
  {
    id: 'auto',
    label: 'Document or source URL',
    description: 'Add a San Diego public document or source page.',
    placeholder: 'Paste a URL, or pick a source',
  },
  ...SAN_DIEGO_SOURCES.map((source) => ({
    id: source.id,
    label: source.label,
    description: source.coverage,
    placeholder: source.url,
    url: source.url,
  })),
];
