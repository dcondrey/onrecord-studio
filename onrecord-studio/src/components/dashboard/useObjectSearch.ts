import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore, type InspectorTarget } from '../../store/useAppStore';
import type { CorpusEntityNode, Finding } from '../../types';
import { matchesTokens, wordTokens } from '../../utils/tokenMatch';
import { enumLabel, severityLabel } from '../objects/labels';

export type ObjectGroup = 'Findings' | 'Agencies' | 'Entities' | 'Records' | 'Passages';

export interface ObjectResult {
  key: string;
  group: ObjectGroup;
  label: string;
  detail: string;
  target: InspectorTarget;
}

export interface ObjectSearch {
  results: ObjectResult[];
  /** A search for the current text is still running: no result yet is not "no match". */
  pending: boolean;
}

const DEBOUNCE_MS = 180;
const PER_GROUP = 4;
const MIN_QUERY = 2;

/**
 * Searches every kind of object at once for the palette. Findings, agencies and entities are
 * matched locally against one snapshot per palette session, word by word (every typed word begins a
 * word of the name); records and passages come from the native searches as they answer. Results for an older query are dropped, so fast typing never
 * shows matches for text no longer in the box, and `pending` stays true until this text's answer.
 */
export function useObjectSearch(query: string): ObjectSearch {
  const [settled, setSettled] = useState<{ query: string; results: ObjectResult[] }>({ query: '', results: [] });
  const gate = useLatestRequestGate();
  const snapshot = useRef<{ findings: Finding[]; entities: CorpusEntityNode[] } | null>(null);
  const profiles = useAppStore((state) => state.agencyProfiles);
  const trimmed = query.trim();
  const searching = trimmed.length >= MIN_QUERY;

  useEffect(() => {
    if (!searching) {
      gate.invalidate();
      setSettled({ query: trimmed, results: [] });
      if (!trimmed) snapshot.current = null;
      return;
    }
    const token = gate.begin();
    const timer = window.setTimeout(() => {
      const needle = wordTokens(trimmed);
      const local = snapshot.current
        ? Promise.resolve(snapshot.current)
        : Promise.all([
          invokeCommand('list_findings', { filter: null, limit: 500 }).catch(() => null),
          invokeCommand('list_top_corpus_entities', { limit: 500 }).catch(() => null),
        ]).then(([findings, entities]) => {
          const read = { findings: findings ?? [], entities: entities ?? [] };
          // A failed read is retried on the next query rather than held as an empty corpus.
          if (findings && entities) snapshot.current = read;
          return read;
        });
      const records = invokeCommand('query_audit_records_page', { searchTerm: trimmed, pageSize: PER_GROUP }).then((page) => page.records).catch(() => []);
      const passages = invokeCommand('hybrid_search', { query: trimmed, limit: PER_GROUP }).catch(() => []);

      void Promise.all([local, records, passages]).then(([{ findings, entities }, recordRows, passageRows]) => {
        if (!gate.isCurrent(token)) return;
        const next: ObjectResult[] = [];
        for (const finding of findings.filter((row) => !row.dismissedAt && matchesTokens(needle, row.headline)).slice(0, PER_GROUP)) {
          next.push({ key: `finding:${finding.id}`, group: 'Findings', label: finding.headline, detail: `${severityLabel(finding.severity)} · ${enumLabel(finding.kind)}`, target: { kind: 'finding', id: finding.id, label: finding.headline } });
        }
        for (const profile of profiles.filter((row) => matchesTokens(needle, row.agencyName)).slice(0, PER_GROUP)) {
          next.push({ key: `agency:${profile.portalSubdomain}:${profile.agencyId}`, group: 'Agencies', label: profile.agencyName, detail: `${profile.portalSubdomain} · ${profile.totalRequestsProcessed.toLocaleString()} requests`, target: { kind: 'agency', portalKey: profile.portalSubdomain, agencyId: profile.agencyId, label: profile.agencyName } });
        }
        for (const entity of entities.filter((row) => matchesTokens(needle, row.displayValue)).slice(0, PER_GROUP)) {
          next.push({ key: `entity:${entity.entityId}`, group: 'Entities', label: entity.displayValue, detail: `${enumLabel(entity.kind)} · ${entity.documentCount.toLocaleString()} documents`, target: { kind: 'entity', entityId: entity.entityId, label: entity.displayValue } });
        }
        // Records come from the native search, which already matches whole index tokens across the
        // request and its documents' text; a preview-only filter here would drop real matches.
        for (const record of recordRows) {
          next.push({ key: `request:${record.portalKey}:${record.requestId}`, group: 'Records', label: record.title || `Request ${record.requestId}`, detail: `${record.agencyName} · ${record.status}`, target: { kind: 'request', portalKey: record.portalKey, requestId: record.requestId, label: record.title || `Request ${record.requestId}` } });
        }
        for (const passage of passageRows) {
          next.push({ key: `passage:${passage.stableId}`, group: 'Passages', label: passage.excerpt, detail: `${passage.fileName} · ${passage.agencyName}`, target: { kind: 'document', portalKey: passage.portalKey, documentId: passage.documentId, requestId: passage.requestId, characterStart: passage.characterStart, characterEnd: passage.characterEnd, label: passage.fileName } });
        }
        setSettled({ query: trimmed, results: next });
      });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [gate, profiles, searching, trimmed]);

  // Decided in render from the text the answer belongs to, so the first frame after a keystroke
  // already reads as searching rather than as an empty result.
  const current = settled.query === trimmed;
  return { results: current ? settled.results : [], pending: searching && !current };
}
