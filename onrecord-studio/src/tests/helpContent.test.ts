import { describe, expect, it } from 'vitest';
import { HELP_GROUP_ORDER, HELP_TOPICS, findHelpTopic, helpTopicForView } from '../content/help';
import { workspaceCommandRegistry } from '../commands/registry';
import { HELP_SHOTS } from '../components/help/shots';

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
    expect(legal.flatMap((entry) => entry.findingKinds)).toEqual(['POLICY_VIOLATION_DETECTED']);
  });
});
