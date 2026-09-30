import type { HelpTopic } from '../../content/help';

/**
 * Plain matching for Help. A query is split into word tokens (digits, and letters that have an upper and lower case);
 * every token must occur as a substring of the topic, and whole-word and prefix hits rank higher.
 * A query with no word characters at all, such as "(" or "c++", is matched as literal text. No
 * pattern language is interpreted anywhere in this file.
 */

function isWordCharacter(character: string) {
  if (character >= '0' && character <= '9') return true;
  return character.toLocaleLowerCase() !== character.toLocaleUpperCase();
}

export function tokenize(text: string): string[] {
  const tokens: string[] = [];
  let current = '';
  for (const character of text.toLocaleLowerCase()) {
    if (isWordCharacter(character)) {
      current += character;
    } else if (current) {
      tokens.push(current);
      current = '';
    }
  }
  if (current) tokens.push(current);
  return tokens;
}

export interface IndexedTopic {
  topic: HelpTopic;
  title: string;
  titleWords: string[];
  keywords: string;
  keywordWords: string[];
  summary: string;
  body: string;
  bodyWords: Set<string>;
}

function bodyText(topic: HelpTopic) {
  return topic.sections
    .flatMap((section) => [section.heading ?? '', ...(section.paragraphs ?? []), ...(section.items ?? [])])
    .join('\n');
}

export function buildSearchIndex(topics: readonly HelpTopic[]): IndexedTopic[] {
  return topics.map((topic) => {
    const body = bodyText(topic).toLocaleLowerCase();
    const keywords = topic.keywords.join('\n').toLocaleLowerCase();
    return {
      topic,
      title: topic.title.toLocaleLowerCase(),
      titleWords: tokenize(topic.title),
      keywords,
      keywordWords: tokenize(keywords),
      summary: topic.summary.toLocaleLowerCase(),
      body,
      bodyWords: new Set(tokenize(body)),
    };
  });
}

function hasWordWithPrefix(words: Iterable<string>, prefix: string) {
  for (const word of words) if (word.startsWith(prefix)) return true;
  return false;
}

/** Score for one needle against one topic, or null when it does not occur anywhere. */
function scoreNeedle(entry: IndexedTopic, needle: string, wholeWord: boolean): number | null {
  let score = 0;
  if (entry.titleWords.includes(needle)) score += 100;
  else if (wholeWord && hasWordWithPrefix(entry.titleWords, needle)) score += 60;
  else if (entry.title.includes(needle)) score += 40;

  if (entry.keywordWords.includes(needle)) score += 50;
  else if (wholeWord && hasWordWithPrefix(entry.keywordWords, needle)) score += 30;
  else if (entry.keywords.includes(needle)) score += 20;

  if (entry.summary.includes(needle)) score += 15;

  if (wholeWord && entry.bodyWords.has(needle)) score += 8;
  else if (wholeWord && hasWordWithPrefix(entry.bodyWords, needle)) score += 6;
  else if (entry.body.includes(needle)) score += 4;

  return score === 0 ? null : score;
}

export interface SearchHit {
  topic: HelpTopic;
  score: number;
}

/** Empty or blank queries return every topic in its authored order. */
export function searchTopics(index: readonly IndexedTopic[], query: string): SearchHit[] {
  const trimmed = query.trim();
  if (!trimmed) return index.map((entry) => ({ topic: entry.topic, score: 0 }));

  const tokens = tokenize(trimmed);
  const needles = tokens.length > 0 ? tokens : [trimmed.toLocaleLowerCase()];
  const wholeWord = tokens.length > 0;

  const hits: SearchHit[] = [];
  index.forEach((entry) => {
    let total = 0;
    for (const needle of needles) {
      const score = scoreNeedle(entry, needle, wholeWord);
      if (score === null) return;
      total += score;
    }
    hits.push({ topic: entry.topic, score: total });
  });
  // Array.prototype.sort is stable, so ties keep the authored order.
  return hits.sort((left, right) => right.score - left.score);
}

/** A short passage around the first place a query term occurs in the body, for the result list. */
export function excerptFor(topic: HelpTopic, query: string, radius = 70): string | null {
  const tokens = tokenize(query);
  const needles = tokens.length > 0 ? tokens : [query.trim().toLocaleLowerCase()];
  if (needles.every((needle) => needle === '')) return null;
  const text = bodyText(topic).split('\n').join(' ');
  const lower = text.toLocaleLowerCase();
  for (const needle of needles) {
    const at = lower.indexOf(needle);
    if (at < 0) continue;
    const start = Math.max(0, at - radius);
    const end = Math.min(text.length, at + needle.length + radius);
    return `${start > 0 ? '…' : ''}${text.slice(start, end).trim()}${end < text.length ? '…' : ''}`;
  }
  return null;
}
