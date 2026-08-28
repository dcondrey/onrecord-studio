import { describe, expect, it } from 'vitest';

import { shouldReportFailure } from '../utils/repeatedFailure';

describe('repeated failure reporting', () => {
  it('reports the first failure and a changed failure immediately', () => {
    expect(shouldReportFailure(null, 'offline', 1_000, 60_000)).toBe(true);
    expect(
      shouldReportFailure({ message: 'offline', reportedAt: 1_000 }, 'permission denied', 1_001, 60_000),
    ).toBe(true);
  });

  it('suppresses identical polling failures until the reminder interval elapses', () => {
    const previous = { message: 'offline', reportedAt: 1_000 };
    expect(shouldReportFailure(previous, 'offline', 60_999, 60_000)).toBe(false);
    expect(shouldReportFailure(previous, 'offline', 61_000, 60_000)).toBe(true);
  });
});
