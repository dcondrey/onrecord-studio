import { useEffect, useRef, type KeyboardEvent, type MouseEvent } from 'react';

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

// Every body child a dialog made inert, with how many open dialogs hold it so and the attributes
// it had before the first one. A count, not a flag, so stacked dialogs closing in any order
// release the page only when the last one goes.
const inertHolds = new Map<Element, { count: number; inert: string | null; ariaHidden: string | null }>();
const openPanels: HTMLElement[] = [];

function holdBackgroundInert(panel: HTMLElement): Element[] {
  const held: Element[] = [];
  for (const child of Array.from(document.body.children)) {
    if (child.contains(panel) || child.tagName === 'SCRIPT') continue;
    const hold = inertHolds.get(child);
    if (hold) hold.count += 1;
    else {
      inertHolds.set(child, { count: 1, inert: child.getAttribute('inert'), ariaHidden: child.getAttribute('aria-hidden') });
      child.setAttribute('inert', '');
      child.setAttribute('aria-hidden', 'true');
    }
    held.push(child);
  }
  return held;
}

function releaseBackgroundInert(held: Element[]) {
  for (const child of held) {
    const hold = inertHolds.get(child);
    if (!hold) continue;
    hold.count -= 1;
    if (hold.count > 0) continue;
    inertHolds.delete(child);
    if (hold.inert === null) child.removeAttribute('inert');
    else child.setAttribute('inert', hold.inert);
    if (hold.ariaHidden === null) child.removeAttribute('aria-hidden');
    else child.setAttribute('aria-hidden', hold.ariaHidden);
  }
}

/**
 * Focus moves into the dialog on open, Tab wraps inside it, Escape closes it, and focus returns
 * to the opener. `aria-modal` promises the page behind is inert, so everything outside the dialog
 * is made `inert` and hidden from assistive technology while it is open, and focus that still
 * lands there is sent back. The dialog must be portaled to `document.body` for that to leave it
 * live. Attach `ref` (with `tabIndex={-1}`) to the dialog panel, and both `onKeyDown` and
 * `onOverlayMouseDown` to the overlay element the panel sits inside.
 */
export function useDialogFocus(onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const panel = ref.current;
    const held = panel ? holdBackgroundInert(panel) : [];
    if (panel) openPanels.push(panel);
    // A child that claimed focus itself (autoFocus) already knows the right first control.
    if (!panel?.contains(document.activeElement)) panel?.focus();
    // `inert` keeps focus out where it is supported; this catches the rest (and engines without it).
    const reclaimFocus = (event: FocusEvent) => {
      if (!panel || openPanels[openPanels.length - 1] !== panel) return;
      const target = event.target;
      if (target instanceof Node && held.some((child) => child.contains(target))) panel.focus();
    };
    document.addEventListener('focusin', reclaimFocus);
    return () => {
      document.removeEventListener('focusin', reclaimFocus);
      if (panel) openPanels.splice(openPanels.lastIndexOf(panel), 1);
      // Inert elements cannot take focus, so the page is released before focus returns to it.
      releaseBackgroundInert(held);
      opener?.focus();
    };
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
