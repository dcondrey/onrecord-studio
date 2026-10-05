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
import watchlists from './topics-watchlists.json';
import scale from './topics-scale.json';
import resolution from './topics-resolution.json';
import annotations from './topics-annotations.json';
import annotationExport from './topics-annotation-export.json';
import identifiers from './topics-identifiers.json';
import pageView from './topics-page-view.json';
import navigation from './topics-navigation.json';
import dispositions from './topics-dispositions.json';
import rollingProduction from './topics-rolling-production.json';
import serviceReportLeads from './topics-service-report-leads.json';
import policeStopLeads from './topics-police-stop-leads.json';
import abatementLogLeads from './topics-abatement-log-leads.json';
import chargeLogLeads from './topics-charge-log-leads.json';
import capacityComparisonLeads from './topics-capacity-comparison-leads.json';
import releaseTiming from './topics-release-timing.json';
import overdueFollowUp from './topics-overdue-follow-up.json';

export type HelpGroup =
  | 'Getting started'
  | 'Views'
  | 'Findings'
  | 'Finding kinds'
  | 'Concepts'
  | 'Accounts and updates'
  | 'Data handling';

/** Content that is true of only one release. Untagged content is shown in every build. */
export type HelpChannel = 'app-store' | 'direct';

export interface HelpSection {
  channel?: HelpChannel;
  heading?: string;
  paragraphs?: string[];
  items?: string[];
}

export interface HelpTopic {
  id: string;
  channel?: HelpChannel;
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

const ALL_HELP_TOPICS = [
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
  ...watchlists,
  ...scale,
  ...resolution,
  ...annotations,
  ...annotationExport,
  ...identifiers,
  ...pageView,
  ...navigation,
  ...dispositions,
  ...rollingProduction,
  ...serviceReportLeads,
  ...policeStopLeads,
  ...abatementLogLeads,
  ...chargeLogLeads,
  ...capacityComparisonLeads,
  ...releaseTiming,
  ...overdueFollowUp,
] as HelpTopic[];

/**
 * The topics a build may show. The App Store build must not describe the direct build's accounts,
 * trials, Stripe checkout or updater (App Review 3.1.1), and the direct build has no App Store
 * purchase. `null` (development) keeps everything.
 */
export function helpTopicsFor(distribution: HelpChannel | null): HelpTopic[] {
  const shown = (channel?: HelpChannel) => !channel || !distribution || channel === distribution;
  const topics = ALL_HELP_TOPICS.filter((topic) => shown(topic.channel));
  const ids = new Set(topics.map((topic) => topic.id));
  return topics.map((topic) => ({
    ...topic,
    sections: topic.sections.filter((section) => shown(section.channel)),
    related: topic.related.filter((id) => ids.has(id)),
  }));
}

export const HELP_TOPICS = helpTopicsFor(__DISTRIBUTION__);

export function findHelpTopic(id: string): HelpTopic | undefined {
  return HELP_TOPICS.find((topic) => topic.id === id);
}

/** The topic a "?" control on a view opens. Falls back to the first topic when none documents it. */
export function helpTopicForView(view: WorkspaceView): HelpTopic {
  return HELP_TOPICS.find((topic) => topic.views.includes(view)) ?? HELP_TOPICS[0];
}
