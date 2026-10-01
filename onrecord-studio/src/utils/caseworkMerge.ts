import type { CaseworkCase, CaseworkCitation } from '../types/casework';

type ItemSection = 'questions' | 'requests' | 'decisions' | 'commitments' | 'evidence';
const ITEM_SECTIONS: ItemSection[] = ['questions', 'requests', 'decisions', 'commitments', 'evidence'];
const SECTION_LABELS: Record<ItemSection, string> = {
  questions: 'Research question',
  requests: 'Records request',
  decisions: 'Decision',
  commitments: 'Commitment',
  evidence: 'Evidence',
};
const FIELDS = ['title', 'owner', 'disposition', 'notes'] as const;
const FIELD_LABELS: Record<(typeof FIELDS)[number], string> = { title: 'Case title', owner: 'Case owner', disposition: 'Case disposition', notes: 'Case notes' };

export type MergeChange = 'added' | 'changed' | 'removed';

export interface MergeCandidate {
  key: string;
  label: string;
  change: MergeChange;
}

type Identified = { id: string } & Record<string, unknown>;

function items(record: CaseworkCase, section: ItemSection): Identified[] {
  return record[section] as unknown as Identified[];
}

function itemName(section: ItemSection, item: Identified): string {
  const name = item.question ?? item.title ?? item.promise ?? item.agendaItem ?? item.label ?? item.id;
  return `${SECTION_LABELS[section]}: ${String(name)}`;
}

/** Item-level differences between the open case and a shared one, for choosing what to take. */
export function mergeCandidates(local: CaseworkCase, incoming: CaseworkCase): MergeCandidate[] {
  const out: MergeCandidate[] = [];
  for (const field of FIELDS) {
    if (local[field] !== incoming[field]) out.push({ key: `field:${field}`, label: FIELD_LABELS[field], change: 'changed' });
  }
  for (const section of ITEM_SECTIONS) {
    const mine = items(local, section);
    const theirs = items(incoming, section);
    for (const item of theirs) {
      const held = mine.find((candidate) => candidate.id === item.id);
      if (!held) out.push({ key: `${section}:${item.id}`, label: itemName(section, item), change: 'added' });
      else if (JSON.stringify(held) !== JSON.stringify(item)) out.push({ key: `${section}:${item.id}`, label: itemName(section, item), change: 'changed' });
    }
    for (const item of mine) {
      if (!theirs.some((candidate) => candidate.id === item.id)) out.push({ key: `${section}:${item.id}`, label: itemName(section, item), change: 'removed' });
    }
  }
  return out;
}

function citedIds(item: Identified): string[] {
  const direct = ['citationIds', 'counterCitationIds', 'outcomeCitationIds', 'completionCitationIds'].flatMap((field) => (item[field] as string[] | undefined) ?? []);
  const productions = ((item.productions as { citationIds?: string[] }[] | undefined) ?? []).flatMap((production) => production.citationIds ?? []);
  return [...direct, ...productions];
}

/**
 * The open case with the chosen differences taken from the shared one. Evidence an adopted item
 * cites is copied across when missing, and a chosen evidence removal is held back while any
 * remaining item still cites it, so no citation id is left pointing at nothing. Nothing is saved:
 * the result is a draft the reader reviews and saves as a new revision.
 */
export function applyMerge(local: CaseworkCase, incoming: CaseworkCase, selected: ReadonlySet<string>): CaseworkCase {
  const next: CaseworkCase = { ...local };
  for (const field of FIELDS) {
    if (selected.has(`field:${field}`)) next[field] = incoming[field];
  }
  const adopted: Identified[] = [];
  for (const section of ITEM_SECTIONS) {
    let merged = [...items(local, section)];
    for (const item of items(incoming, section)) {
      if (!selected.has(`${section}:${item.id}`)) continue;
      adopted.push(item);
      const at = merged.findIndex((candidate) => candidate.id === item.id);
      if (at >= 0) merged[at] = item;
      else merged.push(item);
    }
    merged = merged.filter((item) => !(selected.has(`${section}:${item.id}`) && !items(incoming, section).some((candidate) => candidate.id === item.id)));
    (next as unknown as Record<string, unknown>)[section] = merged;
  }
  const stillCited = new Set(ITEM_SECTIONS.filter((section) => section !== 'evidence').flatMap((section) => items(next, section).flatMap(citedIds)));
  const retained = local.evidence.filter((citation) => stillCited.has(citation.id) && !next.evidence.some((held) => held.id === citation.id));
  const have = new Set([...next.evidence, ...retained].map((citation) => citation.id));
  const missing: CaseworkCitation[] = [];
  for (const id of new Set(adopted.flatMap(citedIds))) {
    const cite = incoming.evidence.find((citation) => citation.id === id);
    if (cite && !have.has(id)) missing.push(cite);
  }
  next.evidence = [...next.evidence, ...retained, ...missing];
  return next;
}
