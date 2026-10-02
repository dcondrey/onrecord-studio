import { beforeEach, describe, expect, it } from 'vitest';
import { INSPECTOR_DEPTH_LIMIT, useAppStore, type InspectorTarget } from '../store/useAppStore';

const finding = (id: string): InspectorTarget => ({ kind: 'finding', id });

describe('inspector stack', () => {
  beforeEach(() => useAppStore.setState({ inspectorStack: [] }));

  it('starts a new stack from a view, stacks from a layer, and returns to a layer already open', () => {
    const { inspect } = useAppStore.getState();
    inspect(finding('a'));
    inspect(finding('b'), 'layer');
    inspect(finding('c'), 'layer');
    inspect(finding('b'), 'layer');
    expect(useAppStore.getState().inspectorStack).toEqual([finding('a'), finding('b')]);
    inspect(finding('z'));
    expect(useAppStore.getState().inspectorStack).toEqual([finding('z')]);
  });

  it('keeps the most recent layers when the depth limit is reached', () => {
    for (let index = 0; index <= INSPECTOR_DEPTH_LIMIT; index++) useAppStore.getState().inspect(finding(String(index)), 'layer');
    const stack = useAppStore.getState().inspectorStack;
    expect(stack).toHaveLength(INSPECTOR_DEPTH_LIMIT);
    expect(stack.at(-1)).toEqual(finding(String(INSPECTOR_DEPTH_LIMIT)));
  });
});
