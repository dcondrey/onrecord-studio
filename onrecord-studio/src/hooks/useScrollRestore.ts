import { useEffect } from 'react';
import { findViewScroller, useAppStore } from '../store/useAppStore';

const RESTORE_WINDOW_MS = 800;

/**
 * After back/forward, returns the restored view to its scroll offset. Content often arrives a few
 * frames after mount, so the offset is retried until it fits or the window closes; a newer view
 * change cancels the attempt.
 */
export function useScrollRestore() {
  const workspaceView = useAppStore((state) => state.workspaceView);
  const consumeScrollRestore = useAppStore((state) => state.consumeScrollRestore);

  useEffect(() => {
    const offset = consumeScrollRestore();
    if (offset === null || offset <= 0) return;
    const started = performance.now();
    let frame = 0;
    const attempt = () => {
      const scroller = findViewScroller();
      if (scroller && scroller.scrollHeight - scroller.clientHeight >= offset) {
        scroller.scrollTop = offset;
        return;
      }
      if (performance.now() - started < RESTORE_WINDOW_MS) frame = requestAnimationFrame(attempt);
      else if (scroller) scroller.scrollTop = offset;
    };
    frame = requestAnimationFrame(attempt);
    return () => cancelAnimationFrame(frame);
  }, [consumeScrollRestore, workspaceView]);
}
