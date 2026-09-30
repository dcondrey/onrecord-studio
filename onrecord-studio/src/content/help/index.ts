import type { FindingKind } from '../../types/findings';
import type { WorkspaceView } from '../../store/useAppStore';
import start from './topics-start.json';
import views from './topics-views.json';
import features from './topics-features.json';
import findings from './topics-findings.json';
import spending from './topics-spending.json';
import concepts from './topics-concepts.json';
import account from './topics-account.json';
import topicPacks from './topics-topic-packs.json';
import plainLanguage from './topics-plain-language.json';

import starterSets from './topics-starter-sets.json';

import evidencePackets from './topics-evidence-packets.json';

import bundle from './topics-bundle.json';

export type HelpGroup =
  | 'Getting started'
  | 'Views'
  | 'Findings'
  | 'Finding kinds'
  | 'Concepts'
  | 'Accounts and updates'
  | 'Data handling';

export interface HelpSection {
  heading?: string;
  paragraphs?: string[];
  items?: string[];
}

export interface HelpTopic {
  id: string;
  title: string;
  group: HelpGroup;
  summary: string;
  keywords: string[];
  sections: HelpSection[];
  /** Workspace views this topic documents. The first topic listing a view is its contextual help. */
  views: WorkspaceView[];
  findingKinds: FindingKind[];
  /** Tauri command modules (the `commands::<group>::` names in lib.rs) this topic documents. */
  commandGroups: string[];
  related: string[];
  /** Key into the bundled screenshots, when the topic has one. */
  screenshot?: string;
  /** Key into the inline diagrams, when the topic has one. */
  figure?: string;
}

export const HELP_GROUP_ORDER: readonly HelpGroup[] = [
  'Getting started',
  'Views',
  'Findings',
  'Finding kinds',
  'Concepts',
  'Accounts and updates',
  'Data handling',
];

export const HELP_TOPICS = [
  ...start,
  ...views,
  ...features,
  ...findings,
  ...spending,
  ...concepts,
  ...account,
  ...topicPacks,
  ...plainLanguage,

  ...starterSets,

  ...evidencePackets,

  ...bundle,
] as HelpTopic[];

export function findHelpTopic(id: string): HelpTopic | undefined {
  return HELP_TOPICS.find((topic) => topic.id === id);
}

/** The topic a "?" control on a view opens. Falls back to the first topic when none documents it. */
export function helpTopicForView(view: WorkspaceView): HelpTopic {
  return HELP_TOPICS.find((topic) => topic.views.includes(view)) ?? HELP_TOPICS[0];
}
