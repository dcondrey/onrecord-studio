import type { AppContrast, AppMotion, AppTheme, WorkspaceView } from '../store/useAppStore';
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

export interface MotionCommandDefinition {
  id: `motion-${AppMotion}`;
  motion: AppMotion;
  label: string;
  detail: string;
  icon: IconName;
  keywords: string;
}

export interface ContrastCommandDefinition {
  id: `contrast-${AppContrast}`;
  contrast: AppContrast;
  label: string;
  detail: string;
  icon: IconName;
  keywords: string;
}

export const workspaceCommandRegistry: readonly WorkspaceCommandDefinition[] = [
  { id: 'workspace-overview', view: 'overview', label: 'Open San Diego research desk', navigationLabel: 'San Diego', detail: 'Local sources, records, and meeting preparation', icon: 'grid', keywords: 'overview workspace screen san diego sources meetings police cpp city', shortcutIndex: 1 },
  { id: 'workspace-patterns', view: 'patterns', label: 'Explore patterns', navigationLabel: 'Patterns', detail: 'Cross-file connections and recurring language', icon: 'sparkles', keywords: 'patterns workspace connections recurring language', shortcutIndex: 2 },
  { id: 'workspace-analysis', view: 'analysis', label: 'Analyze San Diego data', navigationLabel: 'Analysis', detail: 'Semantic retrieval and local SQL datasets', icon: 'database', keywords: 'analysis semantic vector embeddings sql ripa calls nibrs budget san diego', shortcutIndex: 3 },
  { id: 'workspace-evidence', view: 'evidence', label: 'Review evidence', navigationLabel: 'Evidence', detail: 'Search requests and inspect source records', icon: 'evidence', keywords: 'evidence workspace search case review', shortcutIndex: 4 },
  { id: 'workspace-activity', view: 'activity', label: 'View activity', navigationLabel: 'Activity', detail: 'Ingestion queues, throughput, and exceptions', icon: 'activity', keywords: 'activity workspace ingestion operations queues', shortcutIndex: 5 },
  { id: 'workspace-briefing', view: 'briefing', label: 'Build a briefing', navigationLabel: 'Briefing', detail: 'Turn cited evidence into a timed presentation', icon: 'clipboard', keywords: 'briefing presentation public comment outline claims citations', shortcutIndex: 6 },
] as const;

export const themeCommandRegistry: readonly ThemeCommandDefinition[] = [
  { id: 'theme-auto', theme: 'auto', label: 'Use system appearance', detail: 'Follow the operating system setting', icon: 'system', keywords: 'auto system theme color mode appearance' },
  { id: 'theme-light', theme: 'light', label: 'Use light appearance', detail: 'Bright native workspace', icon: 'sun', keywords: 'light theme color mode appearance' },
  { id: 'theme-dark', theme: 'dark', label: 'Use dark appearance', detail: 'Low-light investigative workspace', icon: 'moon', keywords: 'dark theme color mode appearance' },
] as const;

/// Both preferences default to the operating system and can be overridden in either direction.
/// "Keep full motion" and "Use standard contrast" are not redundant with the system default: they
/// are how a reader whose OS asks for reduced motion or higher contrast globally opts this
/// application out, which a one-way toggle cannot express.
export const motionCommandRegistry: readonly MotionCommandDefinition[] = [
  { id: 'motion-system', motion: 'system', label: 'Use system motion setting', detail: 'Follow the operating system reduced-motion preference', icon: 'system', keywords: 'motion animation system accessibility reduced default' },
  { id: 'motion-reduce', motion: 'reduce', label: 'Reduce motion', detail: 'Suppress animation and transitions in this workspace', icon: 'pause', keywords: 'motion animation reduce accessibility vestibular still' },
  { id: 'motion-full', motion: 'full', label: 'Keep full motion', detail: 'Animate regardless of the operating system preference', icon: 'play', keywords: 'motion animation full allow accessibility' },
] as const;

export const contrastCommandRegistry: readonly ContrastCommandDefinition[] = [
  { id: 'contrast-system', contrast: 'system', label: 'Use system contrast setting', detail: 'Follow the operating system increased-contrast preference', icon: 'system', keywords: 'contrast system accessibility default' },
  { id: 'contrast-more', contrast: 'more', label: 'Increase contrast', detail: 'Stronger hairlines, borders, and secondary text', icon: 'sliders', keywords: 'contrast more high accessibility legibility vision' },
  { id: 'contrast-standard', contrast: 'standard', label: 'Use standard contrast', detail: 'Standard palette regardless of the operating system preference', icon: 'sliders', keywords: 'contrast standard normal accessibility' },
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
