import { useAppStore } from '../../store/useAppStore';
import type { useCaseScope } from '../../hooks/useCaseScope';
import Icon from '../ui/icons';

/**
 * Says when a view is narrowed to the active case and turns it off or on. Shown only with a case
 * active; a scope that has not loaded says so instead of presenting an empty list as the case.
 */
/** `listOnly`: the view lists what the case covers beside its full contents instead of filtering them. */
export default function CaseScopeBar({ state, noun, listOnly = false }: { state: ReturnType<typeof useCaseScope>; noun: string; listOnly?: boolean }) {
  const activeCaseId = useAppStore((s) => s.activeCaseId);
  const scopeToCase = useAppStore((s) => s.scopeToCase);
  const setScopeToCase = useAppStore((s) => s.setScopeToCase);
  const inspect = useAppStore((s) => s.inspect);
  if (activeCaseId === null) return null;
  if (!scopeToCase) {
    return (
      <div className="flex items-center gap-2 text-label text-muted">
        <Icon name="briefcase" className="h-3.5 w-3.5" />
        Showing all {noun}.
        <button type="button" onClick={() => setScopeToCase(true)} className="focus-ring rounded font-semibold text-accent hover:underline">Only the active case</button>
      </div>
    );
  }
  return (
    <div role="status" className="motion-reveal flex flex-wrap items-center gap-2 rounded-lg border border-highlight/30 bg-highlight/10 px-3 py-1.5 text-label text-highlight">
      <Icon name="briefcase" className="h-3.5 w-3.5" />
      {state.status === 'pending' && <span>Narrowing to the active case…</span>}
      {state.status === 'failed' && <span className="text-danger">The case's scope did not load, so nothing is hidden: {state.message}</span>}
      {state.status === 'ready' && (
        <span>
          {listOnly ? 'The ' : `Only ${noun} in `}
          <button type="button" onClick={() => inspect({ kind: 'case', caseId: state.value.caseId })} className="focus-ring rounded font-semibold underline decoration-highlight/40 underline-offset-2">the active case</button>
          {listOnly ? ` covers these ${noun}` : ''}{' '}({state.value.scope.documents.length} source document{state.value.scope.documents.length === 1 ? '' : 's'}).
        </span>
      )}
      <button type="button" onClick={() => setScopeToCase(false)} className="focus-ring ml-auto rounded font-semibold hover:underline">{listOnly ? 'Hide' : 'Show everything'}</button>
    </div>
  );
}
