/**
 * Scrolls `element` into view inside `container` only. `element.scrollIntoView` scrolls every
 * scrolling ancestor up to the document, which moves the app shell itself; this moves the one
 * container named. Smooth only when motion is on in full (the resolved preference on <html>).
 */
export function scrollWithin(container: HTMLElement, element: HTMLElement, block: 'start' | 'nearest') {
  const box = container.getBoundingClientRect();
  const target = element.getBoundingClientRect();
  let delta: number;
  if (block === 'start' || target.top < box.top) delta = target.top - box.top;
  else if (target.bottom > box.bottom) delta = Math.min(target.bottom - box.bottom, target.top - box.top);
  else return;
  container.scrollTo({
    top: Math.max(0, container.scrollTop + delta),
    behavior: document.documentElement.dataset.motion === 'full' ? 'smooth' : 'auto',
  });
}

/** The nearest ancestor of `element` that scrolls vertically and has more content than room. */
export function scrollingAncestor(element: HTMLElement): HTMLElement | null {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY;
    if ((overflow === 'auto' || overflow === 'scroll') && node.scrollHeight > node.clientHeight) return node;
  }
  return null;
}
