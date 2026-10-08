import { useEffect } from 'react';
import { useAppStore, type WorkspaceView } from '../../store/useAppStore';
import { workspaceCommandRegistry } from '../../commands/registry';
import Button from '../ui/Button';

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
    <div role="group" aria-label="History" className="flex shrink-0 items-center gap-0.5">
      <Button
        variant="ghost"
        iconOnly
        icon="back"
        onClick={navigateBack}
        disabled={!previous}
        title={previous ? `Back to ${viewLabel(previous.view)} (${modifier}[)` : 'Nothing to go back to'}
        aria-label={previous ? `Back to ${viewLabel(previous.view)}` : 'Back'}
      />
      <Button
        variant="ghost"
        iconOnly
        icon="forward"
        onClick={navigateForward}
        disabled={!next}
        title={next ? `Forward to ${viewLabel(next.view)} (${modifier}])` : 'Nothing to go forward to'}
        aria-label={next ? `Forward to ${viewLabel(next.view)}` : 'Forward'}
      />
    </div>
  );
}

export function isApplePlatform() {
  const platform = navigator.platform.toLocaleLowerCase();
  return platform.includes('mac') || navigator.userAgent.toLocaleLowerCase().includes('mac os');
}
