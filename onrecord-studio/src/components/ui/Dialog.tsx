import { createContext, useContext, useEffect, useId, useSyncExternalStore, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useDialogFocus } from '../licensing/useDialogFocus';
import { cn } from '../../utils/cn';
import Button from './Button';

export type DialogSize = 'sm' | 'md' | 'lg' | 'xl';

const WIDTH: Record<DialogSize, string> = {
  sm: 'max-w-[28rem]',
  md: 'max-w-[36rem]',
  lg: 'max-w-[48rem]',
  xl: 'max-w-[min(1080px,96vw)]',
};

const DialogContext = createContext<{ titleId: string; size: DialogSize; onClose: () => void } | null>(null);

// Open dialogs, oldest first. WKWebView composites every blurred layer separately, so only the
// topmost dialog is glass; the ones beneath it go opaque.
let openDialogs: string[] = [];
const stackListeners = new Set<() => void>();

function subscribeToStack(listener: () => void) {
  stackListeners.add(listener);
  return () => stackListeners.delete(listener);
}

function useIsTopDialog(id: string) {
  useEffect(() => {
    openDialogs = [...openDialogs, id];
    stackListeners.forEach((listener) => listener());
    return () => {
      openDialogs = openDialogs.filter((open) => open !== id);
      stackListeners.forEach((listener) => listener());
    };
  }, [id]);
  return useSyncExternalStore(subscribeToStack, () => !openDialogs.includes(id) || openDialogs[openDialogs.length - 1] === id);
}

function useDialogContext() {
  const context = useContext(DialogContext);
  if (!context) throw new Error('DialogHeader and DialogFooter must be rendered inside a Dialog');
  return context;
}

interface DialogProps {
  onClose: () => void;
  /** Accessible name when there is no DialogHeader. Otherwise the header's title names the dialog. */
  label?: string;
  size?: DialogSize;
  /** `sheet-right` is a full-height panel against the right edge, for long lists. */
  placement?: 'center' | 'sheet-right';
  className?: string;
  children: ReactNode;
}

/**
 * The one modal surface. Mount it only while open. It renders into `document.body`, so no
 * ancestor's transform, filter or backdrop-filter can become the containing block of its fixed
 * overlay. Focus enters on open, Tab stays inside, Escape and a press on the scrim close it, and
 * focus returns to the opener.
 */
export default function Dialog({ onClose, label, size = 'md', placement = 'center', className, children }: DialogProps) {
  const titleId = useId();
  const { ref, onKeyDown, onOverlayMouseDown } = useDialogFocus(onClose);
  const sheet = placement === 'sheet-right';
  const isTop = useIsTopDialog(titleId);
  return createPortal(
    // The scrim fades as a sibling of the panel, never its ancestor: an animated opacity on an
    // ancestor becomes the panel's backdrop root and leaves the glass nothing to blur.
    <div
      className={cn('fixed inset-0 grid', sheet ? 'z-sheet justify-items-end' : 'z-dialog place-items-center p-4')}
      onKeyDown={onKeyDown}
    >
      <div aria-hidden="true" className="motion-fade absolute inset-0 bg-scrim" onMouseDown={onOverlayMouseDown} />
      <div
        ref={ref}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        aria-labelledby={label ? undefined : titleId}
        data-stacked={isTop ? undefined : ''}
        className={cn(
          'glass relative flex w-full flex-col overflow-hidden text-fg shadow-dialog outline-none',
          sheet
            ? 'motion-slide-in h-full max-w-[28rem] rounded-l-2xl border-r-0'
            : cn('motion-pop max-h-[calc(100vh-4rem)] rounded-2xl', WIDTH[size]),
          className,
        )}
      >
        <DialogContext.Provider value={{ titleId, size, onClose }}>{children}</DialogContext.Provider>
      </div>
    </div>,
    document.body,
  );
}

/** Title, optional description, and the close button. Small dialogs use the heading size. */
export function DialogHeader({ title, description, actions }: { title: ReactNode; description?: ReactNode; actions?: ReactNode }) {
  const { titleId, size, onClose } = useDialogContext();
  return (
    <div className="flex shrink-0 items-start gap-3 px-5 py-4">
      <div className="min-w-0 flex-1">
        <h2 id={titleId} className={cn('text-fg', size === 'sm' ? 'text-heading' : 'text-title')}>{title}</h2>
        {description && <p className="mt-1 text-body text-muted">{description}</p>}
      </div>
      {actions && <div className="flex items-center gap-2">{actions}</div>}
      <Button variant="ghost" iconOnly icon="x" aria-label="Close" onClick={onClose} className="-mr-2 -mt-1" />
    </div>
  );
}

/** The scrolling region between header and footer. */
export function DialogBody({ className, children }: { className?: string; children: ReactNode }) {
  return <div className={cn('min-h-0 flex-1 overflow-y-auto px-5 pb-5', className)}>{children}</div>;
}

/** Actions, right-aligned with the primary last. */
export function DialogFooter({ children }: { children: ReactNode }) {
  useDialogContext();
  return <div className="flex shrink-0 items-center justify-end gap-2 border-t border-line px-5 py-4">{children}</div>;
}
