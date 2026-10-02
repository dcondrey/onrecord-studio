import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore, type InspectorTarget } from '../../store/useAppStore';
import type { CorpusEntityNode, Finding } from '../../types';
import { enumLabel, severityLabel } from '../objects/labels';

export type ObjectGroup = 'Findings' | 'Agencies' | 'Entities' | 'Records' | 'Passages';

export interface ObjectResult {
  key: string;
  group: ObjectGroup;
  label: string;
  detail: string;
  target: InspectorTarget;
}

const DEBOUNCE_MS = 180;
const PER_GROUP = 4;
const MIN_QUERY = 2;

/**
 * Searches every kind of object at once for the palette. Findings and entities are matched locally
 * against one snapshot per palette session; records and passages are queried natively. Results for
 * an older query are dropped, so fast typing never shows matches for text no longer in the box.
 */
export function useObjectSearch(query: string): ObjectResult[] {
  const [results, setResults] = useState<ObjectResult[]>([]);
  const gate = useLatestRequestGate();
  const snapshot = useRef<{ findings: Finding[]; entities: CorpusEntityNode[] } | null>(null);
  const profiles = useAppStore((state) => state.agencyProfiles);
  const trimmed = query.trim();

  useEffect(() => {
    if (trimmed.length < MIN_QUERY) {
      gate.invalidate();
      setResults([]);
      if (!trimmed) snapshot.current = null;
      return;
    }
    const token = gate.begin();
    const timer = window.setTimeout(() => {
      const needle = trimmed.toLocaleLowerCase();
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
        for (const finding of findings.filter((row) => !row.dismissedAt && row.headline.toLocaleLowerCase().includes(needle)).slice(0, PER_GROUP)) {
          next.push({ key: `finding:${finding.id}`, group: 'Findings', label: finding.headline, detail: `${severityLabel(finding.severity)} · ${enumLabel(finding.kind)}`, target: { kind: 'finding', id: finding.id, label: finding.headline } });
        }
        for (const profile of profiles.filter((row) => row.agencyName.toLocaleLowerCase().includes(needle)).slice(0, PER_GROUP)) {
          next.push({ key: `agency:${profile.portalSubdomain}:${profile.agencyId}`, group: 'Agencies', label: profile.agencyName, detail: `${profile.portalSubdomain} · ${profile.totalRequestsProcessed.toLocaleString()} requests`, target: { kind: 'agency', portalKey: profile.portalSubdomain, agencyId: profile.agencyId, label: profile.agencyName } });
        }
        for (const entity of entities.filter((row) => row.displayValue.toLocaleLowerCase().includes(needle)).slice(0, PER_GROUP)) {
          next.push({ key: `entity:${entity.entityId}`, group: 'Entities', label: entity.displayValue, detail: `${enumLabel(entity.kind)} · ${entity.documentCount.toLocaleString()} documents`, target: { kind: 'entity', entityId: entity.entityId, label: entity.displayValue } });
        }
        for (const record of recordRows) {
          next.push({ key: `request:${record.portalKey}:${record.requestId}`, group: 'Records', label: record.title || `Request ${record.requestId}`, detail: `${record.agencyName} · ${record.status}`, target: { kind: 'request', portalKey: record.portalKey, requestId: record.requestId, label: record.title || `Request ${record.requestId}` } });
        }
        for (const passage of passageRows) {
          next.push({ key: `passage:${passage.stableId}`, group: 'Passages', label: passage.excerpt, detail: `${passage.fileName} · ${passage.agencyName}`, target: { kind: 'document', portalKey: passage.portalKey, documentId: passage.documentId, requestId: passage.requestId, characterStart: passage.characterStart, characterEnd: passage.characterEnd, label: passage.fileName } });
        }
        setResults(next);
      });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [gate, profiles, trimmed]);

  return results;
}
