import { describe, expect, it } from 'vitest';
import { HELP_TOPICS, type HelpTopic } from '../content/help';
import { buildSearchIndex, excerptFor, searchTopics, tokenize } from '../components/help/search';

function topic(id: string, title: string, body: string, keywords: string[] = []): HelpTopic {
  return {
    id,
    title,
    group: 'Concepts',
    summary: '',
    keywords,
    sections: [{ paragraphs: [body] }],
    views: [],
    findingKinds: [],
    commandGroups: [],
    related: [],
  };
}

const index = buildSearchIndex(HELP_TOPICS);
const ids = (query: string, source = index) => searchTopics(source, query).map((hit) => hit.topic.id);

describe('tokenize', () => {
  it('splits on anything that is not a letter or digit and lowercases', () => {
    expect(tokenize('Cmd+Shift+/ opens 7922.535, \u00c9T\u00c9')).toEqual(['cmd', 'shift', 'opens', '7922', '535', '\u00e9t\u00e9']);
  });
});

describe('searchTopics on the shipped topics', () => {
  it('returns every topic in authored order for an empty query', () => {
    expect(ids('')).toEqual(HELP_TOPICS.map((entry) => entry.id));
    expect(ids('   ')).toHaveLength(HELP_TOPICS.length);
  });

  it('ranks a title hit above a body-only hit', () => {
    expect(ids('deadlines')[0]).toBe('statutory-deadlines');
  });

  it('is case insensitive and matches inside words', () => {
    expect(ids('C2PA')).toContain('provenance-c2pa');
    expect(ids('c2p')).toContain('provenance-c2pa');
  });

  it('requires every word of the query to match', () => {
    expect(ids('provenance digest')).toContain('provenance-c2pa');
    expect(ids('provenance zzzznotaword')).toEqual([]);
  });

  it('finds the disclosure topics by the terms a worried reader would type', () => {
    expect(ids('openai')).toContain('data-handling');
    expect(ids('openai')).toContain('autonomous-research');
    expect(ids('privacy')[0]).toBe('data-handling');
    expect(ids('unassessed')).toContain('measured-vs-unassessed');
  });

  it('finds a finding kind by its wire name and by plain words', () => {
    expect(ids('POLICY_VIOLATION_DETECTED')).toContain('finding-policy-violation-detected');
    expect(ids('own stated commitment')).toContain('finding-policy-violation-detected');
  });
});

describe('searchTopics treats the query as text, never as a pattern', () => {
  const synthetic = buildSearchIndex([
    topic('a', 'Plain topic', 'nothing special here'),
    topic('b', 'Symbols', 'call f(x) then [wait] and a.b*c ? done'),
  ]);

  it.each(['(', '[', '*', '?', '.', '\\', '+'])('matches %s only where it literally occurs', (symbol) => {
    expect(() => searchTopics(synthetic, symbol)).not.toThrow();
    expect(ids(symbol, synthetic)).toEqual(symbol === '\\' || symbol === '+' ? [] : ['b']);
  });

  it('does not expand a pattern that would match everything', () => {
    expect(ids('.*', synthetic)).toEqual([]);
    expect(ids('^', synthetic)).toEqual([]);
  });

  it('builds an excerpt around the first literal occurrence', () => {
    const target = topic('c', 'Excerpt', `${'x '.repeat(80)}needle ${'y '.repeat(80)}`);
    const excerpt = excerptFor(target, 'needle', 20);
    expect(excerpt).toContain('needle');
    expect(excerpt?.startsWith('…')).toBe(true);
    expect(excerptFor(target, 'absent')).toBeNull();
    expect(excerptFor(target, '')).toBeNull();
  });
});
