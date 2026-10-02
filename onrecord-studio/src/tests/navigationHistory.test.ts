import { beforeEach, describe, expect, it } from 'vitest';
import { NAVIGATION_HISTORY_LIMIT, useAppStore, type WorkspaceView } from '../store/useAppStore';

const reset = () => useAppStore.setState({
  workspaceView: 'overview', navigationBack: [], navigationForward: [], pendingScrollRestore: null,
  searchTerm: '', selectedAgencyId: null, selectedRequestId: null, selectedRequestPortalKey: null, activePortalKey: null,
});

describe('navigation history', () => {
  beforeEach(reset);

  it('returns each view with the filters it was left with, and a new jump discards forward history', () => {
    const store = useAppStore.getState();
    store.setSearchFilters('shelter', 4, null);
    store.setWorkspaceView('evidence');
    useAppStore.getState().setSelectedRequest(101, 'sdpd');
    useAppStore.getState().setWorkspaceView('story');
    useAppStore.getState().setSearchFilters('', null, null);

    useAppStore.getState().navigateBack();
    let state = useAppStore.getState();
    expect(state.workspaceView).toBe('evidence');
    expect([state.searchTerm, state.selectedAgencyId, state.selectedRequestId]).toEqual(['shelter', 4, 101]);

    useAppStore.getState().navigateBack();
    expect(useAppStore.getState().workspaceView).toBe('overview');
    useAppStore.getState().navigateForward();
    expect(useAppStore.getState().workspaceView).toBe('evidence');

    useAppStore.getState().setWorkspaceView('compare');
    state = useAppStore.getState();
    expect(state.navigationForward).toEqual([]);
    expect(state.navigationBack.map((entry) => entry.view)).toEqual(['overview', 'evidence']);
  });

  it('records nothing for a jump to the current view and keeps at most the limit', () => {
    useAppStore.getState().setWorkspaceView('overview');
    expect(useAppStore.getState().navigationBack).toEqual([]);
    const views: WorkspaceView[] = ['evidence', 'story'];
    for (let index = 0; index <= NAVIGATION_HISTORY_LIMIT + 1; index++) useAppStore.getState().setWorkspaceView(views[index % 2]);
    expect(useAppStore.getState().navigationBack).toHaveLength(NAVIGATION_HISTORY_LIMIT);
  });

  it('does nothing when there is no history', () => {
    useAppStore.getState().navigateBack();
    useAppStore.getState().navigateForward();
    expect(useAppStore.getState().workspaceView).toBe('overview');
  });
});
