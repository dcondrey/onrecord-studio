/**
 * Scrolls `element` into view inside `container` only. `element.scrollIntoView` scrolls every
 * scrolling ancestor up to the document, which moves the app shell itself; this moves the one
 * container named. Smooth only when motion is on in full (the resolved preference on <html>) and
 * the caller doesn't ask for an instant jump.
 */
export function scrollWithin(container: HTMLElement, element: HTMLElement, block: 'start' | 'nearest', instant = false) {
  const box = container.getBoundingClientRect();
  const target = element.getBoundingClientRect();
  let delta: number;
  if (block === 'start' || target.top < box.top) delta = target.top - box.top;
  else if (target.bottom > box.bottom) delta = Math.min(target.bottom - box.bottom, target.top - box.top);
  else return;
  container.scrollTo({
    top: Math.max(0, container.scrollTop + delta),
    behavior: !instant && document.documentElement.dataset.motion === 'full' ? 'smooth' : 'auto',
  });
}

/**
 * The nearest ancestor of `element` that scrolls vertically and has more content than room. With
 * `overflowing` false, the nearest that can scroll at all: the one that will hold `element` once
 * content still arriving makes it overflow.
 */
export function scrollingAncestor(element: HTMLElement, overflowing = true): HTMLElement | null {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if ((overflow === 'auto' || overflow === 'scroll') && (!overflowing || node.scrollHeight > node.clientHeight)) return node;
  }
  return null;
}

/**
 * Moves focus to `element` for the reader who asked to be taken there, and shows the focus ring
 * even when the request was a click: a browser draws `:focus-visible` after a script's focus only
 * when the previous focus was drawn, so index.css draws `[data-focus-landed]` until focus leaves.
 */
export function landFocus(element: HTMLElement) {
  element.dataset.focusLanded = '';
  element.addEventListener('blur', () => { delete element.dataset.focusLanded; }, { once: true });
  element.focus({ preventScroll: true });
}
