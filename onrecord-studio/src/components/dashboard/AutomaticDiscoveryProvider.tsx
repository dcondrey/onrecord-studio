import { createContext, useContext, useEffect, useState, useSyncExternalStore, type ReactNode } from 'react';
import { invokeCommand } from '../../ipc';
import { useAppStore } from '../../store/useAppStore';
import { DiscoveryController } from '../../utils/discoveryController';
import { guidanceForSubject } from '../../utils/sourceGuidance';

const Context = createContext<DiscoveryController | null>(null);
export function AutomaticDiscoveryProvider({ children }: { children: ReactNode }) {
  const refreshRequests = useAppStore((state) => state.evidenceRefreshRequests);
  const [controller] = useState(() => new DiscoveryController({
    schema: () => invokeCommand('get_analytical_schema', {}),
    candidates: () => invokeCommand('list_analytical_import_candidates', {}),
    patterns: () => invokeCommand('query_corpus_patterns', { query: { minimumEvidenceCount: 2, resultLimit: 8,
      patternKinds: ['EXACT_DUPLICATE', 'NEAR_DUPLICATE', 'SHARED_ENTITY', 'SHARED_STATUTE'], requiredTerms: [] } }),
    maintain: () => invokeCommand('run_data_maintenance', {}),
    query: (sql) => invokeCommand('run_analytical_sql', { sql, limit: 100 }),
    fetch: (request) => invokeCommand('fetch_city_dataset', request),
    collect: ({ url, subject }) => invokeCommand('start_data_source', { url, guidance: guidanceForSubject(subject) }),
    record: (leads, complete) => invokeCommand('record_service_leads', { leads, complete }).then((outcome) => {
      if (outcome.findingsRecorded + outcome.staleCleared > 0) useAppStore.getState().requestFindingsRefresh();
    }),
  }, () => document.visibilityState !== 'hidden'));
  useEffect(() => {
    controller.start();
    const resume = () => controller.refresh();
    window.addEventListener('focus', resume);
    document.addEventListener('visibilitychange', resume);
    return () => { controller.stop(); window.removeEventListener('focus', resume); document.removeEventListener('visibilitychange', resume); };
  }, [controller]);
  useEffect(() => { controller.refresh(); }, [controller, refreshRequests]);
  return <Context.Provider value={controller}>{children}</Context.Provider>;
}
export const useDiscoveryController = () => useContext(Context);
const noop = () => () => {};
const empty = () => null;
export function useAutomaticDiscovery() {
  const controller = useDiscoveryController();
  const state = useSyncExternalStore(controller?.subscribe ?? noop, controller?.snapshot ?? empty);
  return { controller, state };
}
