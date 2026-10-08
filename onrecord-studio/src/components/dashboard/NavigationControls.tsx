import { useEffect } from 'react';
import { useAppStore, type WorkspaceView } from '../../store/useAppStore';
import { workspaceCommandRegistry } from '../../commands/registry';
import { cn } from '../../utils/cn';
import Icon from '../ui/icons';

const VIEW_LABELS = new Map<WorkspaceView, string>(workspaceCommandRegistry.map((command) => [command.view, command.navigationLabel]));

export function viewLabel(view: WorkspaceView) {
  return VIEW_LABELS.get(view) ?? view;
}

/** Back names where it goes, so a jump from one view to another always shows the way home. */
export default function NavigationControls() {
  const previous = useAppStore((state) => state.navigationBack.at(-1) ?? null);
  const next = useAppStore((state) => state.navigationForward[0] ?? null);
  const navigateBack = useAppStore((state) => state.navigateBack);
  const navigateForward = useAppStore((state) => state.navigateForward);
  const modifier = isApplePlatform() ? '⌘' : 'Ctrl';

  // Mouse buttons 3 and 4 are the hardware back and forward buttons.
  useEffect(() => {
    const onMouseUp = (event: MouseEvent) => {
      if (event.button === 3) { event.preventDefault(); navigateBack(); }
      if (event.button === 4) { event.preventDefault(); navigateForward(); }
    };
    window.addEventListener('mouseup', onMouseUp);
    return () => window.removeEventListener('mouseup', onMouseUp);
  }, [navigateBack, navigateForward]);

  return (
    <div role="group" aria-label="History" className="flex shrink-0 items-center gap-1">
      <button
        type="button"
        onClick={navigateBack}
        disabled={!previous}
        title={previous ? `Back to ${viewLabel(previous.view)} (${modifier}[)` : 'Nothing to go back to'}
        aria-label={previous ? `Back to ${viewLabel(previous.view)}` : 'Back'}
        className={cn(
          'motion-press focus-ring flex h-8 items-center gap-1 rounded-lg border px-2 text-label font-medium transition',
          previous ? 'border-control bg-surface/70 text-fg-secondary hover:text-fg' : 'border-transparent text-muted/60',
        )}
      >
        <Icon name="back" className="h-4 w-4" />
        {previous && <span key={previous.view} className="motion-fade hidden max-w-[9rem] truncate xl:inline">{viewLabel(previous.view)}</span>}
      </button>
      <button
        type="button"
        onClick={navigateForward}
        disabled={!next}
        title={next ? `Forward to ${viewLabel(next.view)} (${modifier}])` : 'Nothing to go forward to'}
        aria-label={next ? `Forward to ${viewLabel(next.view)}` : 'Forward'}
        className={cn(
          'motion-press focus-ring grid h-8 w-8 place-items-center rounded-lg border transition',
          next ? 'border-control bg-surface/70 text-fg-secondary hover:text-fg' : 'border-transparent text-muted/60',
        )}
      >
        <Icon name="forward" className="h-4 w-4" />
      </button>
    </div>
  );
}

export function isApplePlatform() {
  const platform = navigator.platform.toLocaleLowerCase();
  return platform.includes('mac') || navigator.userAgent.toLocaleLowerCase().includes('mac os');
}
