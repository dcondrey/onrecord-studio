import { useEffect, useRef, type KeyboardEvent, type MouseEvent } from 'react';

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/**
 * Focus moves into the dialog on open, Tab wraps inside it, Escape closes it, and focus returns
 * to the opener. `aria-modal` promises the page behind is inert, so the wrap is not optional.
 * Attach `ref` (with `tabIndex={-1}`) to the dialog panel, and both `onKeyDown` and
 * `onOverlayMouseDown` to the overlay element the panel sits inside.
 */
export function useDialogFocus(onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // A child that claimed focus itself (autoFocus) already knows the right first control.
    if (!ref.current?.contains(document.activeElement)) ref.current?.focus();
    return () => opener?.focus();
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === 'Escape') {
      onCloseRef.current();
      return;
    }
    if (event.key !== 'Tab' || !ref.current) return;
    const focusable = Array.from(ref.current.querySelectorAll<HTMLElement>(FOCUSABLE));
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = document.activeElement;
    if (!first || !last) {
      event.preventDefault();
    } else if (event.shiftKey && (active === first || active === ref.current)) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  };

  // Only when the mousedown itself targets the overlay, not a click that started inside the
  // panel and was dragged out (a text selection must not also dismiss the dialog).
  const onOverlayMouseDown = (event: MouseEvent<HTMLElement>) => {
    if (event.target === event.currentTarget) onCloseRef.current();
  };

  return { ref, onKeyDown, onOverlayMouseDown };
}
