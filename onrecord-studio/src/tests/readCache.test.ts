import { describe, expect, it, vi } from 'vitest';
import { createReadCache } from '../utils/readCache';

describe('read cache', () => {
  it('shares concurrent reads and never caches a failed read', async () => {
    const cache = createReadCache<string>(2);
    const load = vi.fn().mockRejectedValueOnce(new Error('busy')).mockResolvedValue('evidence');
    const first = cache.read('a', load);
    const second = cache.read('a', load);
    expect(first).toBe(second);
    await expect(first).rejects.toThrow('busy');
    expect(cache.peek('a')).toBeUndefined();
    await expect(cache.read('a', load)).resolves.toBe('evidence');
    await cache.read('a', load);
    expect(load).toHaveBeenCalledTimes(2);
  });

  it('bounds retained evidence and expires it after five minutes', async () => {
    const now = vi.spyOn(Date, 'now').mockReturnValue(0);
    try {
      const cache = createReadCache<number>(2);
      await cache.read('a', async () => 1);
      await cache.read('b', async () => 2);
      expect(cache.peek('a')).toBe(1);
      await cache.read('c', async () => 3);
      expect(cache.peek('a')).toBeUndefined();
      expect(cache.peek('b')).toBe(2);
      now.mockReturnValue(299_999);
      expect(cache.peek('c')).toBe(3);
      now.mockReturnValue(300_000);
      expect(cache.peek('c')).toBeUndefined();
      now.mockReturnValue(300_001);
      expect(cache.peek('b')).toBeUndefined();
    } finally { now.mockRestore(); }
  });
});
