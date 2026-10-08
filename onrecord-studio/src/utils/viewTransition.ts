import { flushSync } from 'react-dom';

type ViewTransitionDocument = Document & { startViewTransition?: (update: () => void) => unknown };

/**
 * Runs a state update as a shared-element view transition where WebKit supports it (18+) and the
 * resolved motion setting allows it. `data-motion` is what App.tsx writes from both the OS query and
 * the in-app Reduce motion command. Otherwise the update runs synchronously, so motion never
 * delays the change.
 */
export function withViewTransition(update: () => void): void {
  const doc = document as ViewTransitionDocument;
  if (typeof doc.startViewTransition !== 'function' || document.documentElement.dataset.motion === 'reduce') {
    update();
    return;
  }
  doc.startViewTransition(() => flushSync(update));
}
