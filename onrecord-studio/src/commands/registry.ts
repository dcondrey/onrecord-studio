import type { AppTheme, WorkspaceView } from '../store/useAppStore';
import type { IconName } from '../components/ui/icons';

export interface WorkspaceCommandDefinition {
  id: `workspace-${WorkspaceView}`;
  view: WorkspaceView;
  label: string;
  navigationLabel: string;
  detail: string;
  icon: IconName;
  keywords: string;
  shortcutIndex: number;
}

export interface ThemeCommandDefinition {
  id: `theme-${AppTheme}`;
  theme: AppTheme;
  label: string;
  detail: string;
  icon: IconName;
  keywords: string;
}

export const workspaceCommandRegistry: readonly WorkspaceCommandDefinition[] = [
  { id: 'workspace-overview', view: 'overview', label: 'Open overview', navigationLabel: 'Overview', detail: 'Signals and agency performance', icon: 'grid', keywords: 'overview workspace screen signals performance', shortcutIndex: 1 },
  { id: 'workspace-patterns', view: 'patterns', label: 'Explore patterns', navigationLabel: 'Patterns', detail: 'Cross-file connections and recurring language', icon: 'sparkles', keywords: 'patterns workspace connections recurring language', shortcutIndex: 2 },
  { id: 'workspace-evidence', view: 'evidence', label: 'Review evidence', navigationLabel: 'Evidence', detail: 'Search requests and inspect source records', icon: 'evidence', keywords: 'evidence workspace search case review', shortcutIndex: 3 },
  { id: 'workspace-activity', view: 'activity', label: 'View activity', navigationLabel: 'Activity', detail: 'Ingestion queues, throughput, and exceptions', icon: 'activity', keywords: 'activity workspace ingestion operations queues', shortcutIndex: 4 },
] as const;

export const themeCommandRegistry: readonly ThemeCommandDefinition[] = [
  { id: 'theme-auto', theme: 'auto', label: 'Use system appearance', detail: 'Follow the operating system setting', icon: 'system', keywords: 'auto system theme color mode appearance' },
  { id: 'theme-light', theme: 'light', label: 'Use light appearance', detail: 'Bright native workspace', icon: 'sun', keywords: 'light theme color mode appearance' },
  { id: 'theme-dark', theme: 'dark', label: 'Use dark appearance', detail: 'Low-light investigative workspace', icon: 'moon', keywords: 'dark theme color mode appearance' },
] as const;

export function scoreCommand(query: string, label: string, detail: string, keywords: string) {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return 0;
  const normalizedLabel = label.toLocaleLowerCase();
  const haystack = `${normalizedLabel} ${detail} ${keywords}`.toLocaleLowerCase();
  let score = 0;
  for (const term of terms) {
    const position = haystack.indexOf(term);
    if (position < 0) return null;
    score += normalizedLabel === term ? 100 : normalizedLabel.startsWith(term) ? 60 : position < normalizedLabel.length ? 30 : 10;
  }
  return score;
}

