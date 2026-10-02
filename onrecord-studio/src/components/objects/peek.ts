import { invokeCommand } from '../../ipc';
import { inspectorTargetKey, useAppStore, type InspectorTarget } from '../../store/useAppStore';
import { confidenceLabel, enumLabel, findingKindLabel, formatShortDate, severityLabel } from './labels';

export interface PeekData {
  eyebrow: string;
  title: string;
  lines: string[];
  /** A passage with the cited span marked, for documents. */
  excerpt?: { before: string; marked: string; after: string };
}

const CACHE_LIMIT = 64;
const EXCERPT_CONTEXT = 140;
const cache = new Map<string, { generation: number; data: PeekData }>();

/** Any write that can change what a peek says bumps one of these counters, which retires the cache. */
function dataGeneration() {
  const state = useAppStore.getState();
  return state.findingsRefreshRequests * 1_000_000 + state.evidenceRefreshRequests * 1_000 + state.analyticsRefreshRequests;
}

export function cachedPeek(target: InspectorTarget): PeekData | null {
  const entry = cache.get(inspectorTargetKey(target));
  return entry && entry.generation === dataGeneration() ? entry.data : null;
}

export async function loadPeek(target: InspectorTarget): Promise<PeekData> {
  const key = inspectorTargetKey(target);
  const generation = dataGeneration();
  const hit = cache.get(key);
  if (hit && hit.generation === generation) return hit.data;
  const data = await fetchPeek(target);
  cache.delete(key);
  cache.set(key, { generation, data });
  while (cache.size > CACHE_LIMIT) cache.delete(cache.keys().next().value!);
  return data;
}

async function fetchPeek(target: InspectorTarget): Promise<PeekData> {
  switch (target.kind) {
    case 'finding': {
      const [finding] = await invokeCommand('list_findings', { filter: { id: target.id, includeDismissed: true }, limit: 1 });
      if (!finding) return { eyebrow: 'Finding', title: target.label ?? target.id, lines: ['This finding is no longer stored.'] };
      return {
        eyebrow: `${severityLabel(finding.severity)} · ${findingKindLabel(finding.kind)}`,
        title: finding.headline,
        lines: [
          confidenceLabel(finding.confidence),
          `${finding.citations.length} cited passage${finding.citations.length === 1 ? '' : 's'} · first seen ${formatShortDate(finding.discoveredAt)}`,
          ...(finding.dismissedAt ? [`Dismissed ${formatShortDate(finding.dismissedAt)}`] : []),
        ],
      };
    }
    case 'entity': {
      const neighborhood = await invokeCommand('get_corpus_entity_neighborhood', { entityId: target.entityId, limit: 3 });
      if (!neighborhood) return { eyebrow: 'Entity', title: target.label, lines: ['Not in the current corpus index.'] };
      const neighbors = neighborhood.edges.map((edge) => edge.neighbor.displayValue);
      return {
        eyebrow: enumLabel(neighborhood.center.kind),
        title: neighborhood.center.displayValue,
        lines: [
          `Mentioned in ${neighborhood.center.documentCount.toLocaleString()} document${neighborhood.center.documentCount === 1 ? '' : 's'}`,
          neighbors.length ? `Appears with ${neighbors.join(', ')}${neighborhood.truncated ? ', and more' : ''}` : 'No co-occurring entity recorded',
        ],
      };
    }
    case 'document': {
      const start = target.characterStart ?? 0;
      const offset = Math.max(0, start - EXCERPT_CONTEXT);
      const span = target.characterStart === null ? 0 : Math.max(0, (target.characterEnd ?? start) - start);
      const chunk = await invokeCommand('get_document_text_chunk', {
        documentId: target.documentId, portalKey: target.portalKey, characterOffset: offset, characterLimit: span + EXCERPT_CONTEXT * 2,
      });
      const text = chunk.maskedText;
      const markStart = start - chunk.characterOffset;
      const excerpt = target.characterStart === null || markStart < 0
        ? { before: text.slice(0, EXCERPT_CONTEXT * 2), marked: '', after: '' }
        : { before: text.slice(0, markStart), marked: text.slice(markStart, markStart + span), after: text.slice(markStart + span) };
      return {
        eyebrow: `Document · ${target.portalKey}`,
        title: target.label ?? `Document ${target.documentId}`,
        lines: [`${chunk.totalCharacterCount.toLocaleString()} characters of extracted text${chunk.maskedFindingCount ? ` · ${chunk.maskedFindingCount} masked` : ''}`],
        excerpt,
      };
    }
    case 'agency': {
      const profile = useAppStore.getState().agencyProfiles.find((entry) => entry.agencyId === target.agencyId && entry.portalSubdomain === target.portalKey);
      if (!profile) return { eyebrow: `Agency · ${target.portalKey}`, title: target.label, lines: ['No analytics profile computed for this agency.'] };
      return {
        eyebrow: `Agency · ${target.portalKey}`,
        title: profile.agencyName,
        lines: [
          `${profile.totalRequestsProcessed.toLocaleString()} requests · ${profile.totalClosed.toLocaleString()} closed`,
          profile.averageClosureDays === null ? 'No measurable closure' : `${profile.averageClosureDays.toFixed(1)} days average closure over ${profile.validLatencySampleSize.toLocaleString()} closures`,
          profile.statutoryViolationCount > 0 ? `${profile.statutoryViolationCount.toLocaleString()} counted statutory violations` : 'No counted statutory violation',
        ],
      };
    }
    case 'case': {
      const state = await invokeCommand('get_casework_state', {});
      const found = state.cases.find((entry) => entry.id === target.caseId);
      if (!found) return { eyebrow: 'Case', title: target.label ?? 'Case', lines: ['This case is no longer stored.'] };
      return {
        eyebrow: `Case · ${found.disposition.toLowerCase()}`,
        title: found.title,
        lines: [
          `${found.evidence.length} cited source${found.evidence.length === 1 ? '' : 's'} · ${found.requests.length} request${found.requests.length === 1 ? '' : 's'} · ${found.questions.length} open question${found.questions.length === 1 ? '' : 's'}`,
          `Updated ${formatShortDate(found.updatedAt)}`,
        ],
      };
    }
    case 'request': {
      const documents = await invokeCommand('list_request_documents', { requestId: target.requestId, portalKey: target.portalKey });
      return {
        eyebrow: `Request ${target.requestId} · ${target.portalKey}`,
        title: target.label ?? `Request ${target.requestId}`,
        lines: [`${documents.length.toLocaleString()} stored document${documents.length === 1 ? '' : 's'}`],
      };
    }
  }
}
