import { describe, expect, it } from 'vitest';
import { HELP_GROUP_ORDER, HELP_TOPICS, findHelpTopic, helpTopicForView, helpTopicsFor } from '../content/help';
import { workspaceCommandRegistry } from '../commands/registry';
import { HELP_SHOTS } from '../components/help/shots';
import { LEGAL_CLAIM_KINDS } from '../types/findings';

const text = (id: string) => {
  const topic = findHelpTopic(id);
  if (!topic) throw new Error(`missing topic ${id}`);
  return [topic.title, topic.summary, ...topic.sections.flatMap((s) => [s.heading ?? '', ...(s.paragraphs ?? []), ...(s.items ?? [])])].join('\n');
};

describe('help content', () => {
  it('has unique ids, known groups, related links that resolve, and screenshots that exist', () => {
    const ids = HELP_TOPICS.map((entry) => entry.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const entry of HELP_TOPICS) {
      expect(HELP_GROUP_ORDER).toContain(entry.group);
      expect(entry.sections.length).toBeGreaterThan(0);
      for (const related of entry.related) expect(ids).toContain(related);
      if (entry.screenshot) expect(HELP_SHOTS[entry.screenshot]).toBeDefined();
    }
  });

  it('keeps the direct build\'s accounts, payment and updater out of App Store help', () => {
    for (const distribution of ['app-store', 'direct'] as const) {
      const topics = helpTopicsFor(distribution);
      const ids = new Set(topics.map((topic) => topic.id));
      for (const topic of topics) {
        expect(topic.sections.length).toBeGreaterThan(0);
        for (const related of topic.related) expect(ids).toContain(related);
      }
    }
    const appStoreText = helpTopicsFor('app-store')
      .flatMap((topic) => [topic.title, topic.summary, ...topic.sections.flatMap((s) => [s.heading ?? '', ...(s.paragraphs ?? []), ...(s.items ?? [])])])
      .join('\n');
    expect(appStoreText).not.toMatch(/stripe|checkout|trial|subscription|per seat|sign in with|writerslogic account|updates\.writerslogic|~\/\.env|\bbeta\b/i);
  });

  it('gives every workspace view a contextual topic that lists it', () => {
    for (const command of workspaceCommandRegistry) {
      expect(helpTopicForView(command.view).views).toContain(command.view);
    }
  });

  it('keeps the disclosure that some features do contact other servers', () => {
    const handling = text('data-handling');
    for (const required of ['OpenAI', 'OpenRouter', 'Ollama', 'auth.writerslogic.com', 'updates.writerslogic.com', 'email address', 'Stripe']) {
      expect(handling).toContain(required);
    }
    expect(text('autonomous-research')).toContain('~/.env');
    expect(text('work-email-verification')).toContain('does not prove you work there');
  });

  it('labels exactly the kinds the app treats as legal claims', () => {
    const legal = HELP_TOPICS.filter((entry) => text(entry.id).includes('Basis: Legal claim: counted violation'));
    expect(legal.flatMap((entry) => entry.findingKinds).sort()).toEqual([...LEGAL_CLAIM_KINDS].sort());
  });
});
