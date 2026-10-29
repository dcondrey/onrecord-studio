const caches = new Set<() => void>();

export function clearReadCaches() {
  for (const clear of caches) clear();
}

export function createReadCache<T>(capacity: number) {
  if (!Number.isSafeInteger(capacity) || capacity < 1) throw new Error('Cache capacity must be a positive integer');
  const entries = new Map<string, { promise: Promise<T>; value?: T; createdAt: number }>();
  const clear = () => entries.clear();
  caches.add(clear);
  const current = (key: string) => {
    const entry = entries.get(key);
    if (entry && Date.now() - entry.createdAt >= 300_000) { entries.delete(key); return undefined; }
    return entry;
  };
  return {
    peek(key: string): T | undefined { return current(key)?.value; },
    clear,
    read(key: string, load: () => Promise<T>): Promise<T> {
      const existing = current(key);
      if (existing) return existing.promise;
      let promise: Promise<T>;
      try { promise = load(); } catch (error) { return Promise.reject(error); }
      const entry: { promise: Promise<T>; value?: T; createdAt: number } = {
        createdAt: Date.now(),
        promise,
      };
      entries.set(key, entry);
      void promise.then((value) => { entry.value = value; }, () => {
        if (entries.get(key) === entry) entries.delete(key);
      });
      while (entries.size > capacity) entries.delete(entries.keys().next().value!);
      return entry.promise;
    },
  };
}
