// @vitest-environment jsdom
import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { useLatestRequestGate } from '../hooks/useLatestRequestGate';

describe('useLatestRequestGate', () => {
  it('accepts only the newest request token', () => {
    const { result } = renderHook(() => useLatestRequestGate());
    let first = 0;
    let second = 0;
    act(() => {
      first = result.current.begin();
      second = result.current.begin();
    });
    expect(result.current.isCurrent(first)).toBe(false);
    expect(result.current.isCurrent(second)).toBe(true);
  });

  it('can invalidate an in-flight request without starting another one', () => {
    const { result } = renderHook(() => useLatestRequestGate());
    let token = 0;
    act(() => {
      token = result.current.begin();
      result.current.invalidate();
    });
    expect(result.current.isCurrent(token)).toBe(false);
  });

  it('invalidates the current token during unmount cleanup', () => {
    const { result, unmount } = renderHook(() => useLatestRequestGate());
    let token = 0;
    act(() => {
      token = result.current.begin();
    });
    const isCurrent = result.current.isCurrent;
    expect(isCurrent(token)).toBe(true);
    unmount();
    expect(isCurrent(token)).toBe(false);
  });
});
