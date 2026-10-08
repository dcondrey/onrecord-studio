import { useCallback, useRef, useState, type KeyboardEvent } from 'react';

export interface RovingFocusOptions {
  /** `cols` makes it a 2-D grid; without it the items are one list. */
  shape: { rows: number; cols?: number };
  onActivate(i: number, j?: number): void;
  onEscape?(): void;
}

export interface RovingItemProps {
  tabIndex: 0 | -1;
  onKeyDown(event: KeyboardEvent<Element>): void;
  onFocus(): void;
  onBlur(): void;
  ref(element: Element | null): void;
  'data-focused'?: true;
}

const PAGE = 10;

/**
 * Makes a figure one tab stop whose marks are reached with the arrow keys (2-D in a grid), Home and
 * End (with Ctrl/Cmd in a grid: the first or last cell), PageUp/PageDown by 10, Enter or Space to
 * activate, Escape to clear. Movement stops at the edges; it never wraps.
 */
export function useRovingFocus({ shape, onActivate, onEscape }: RovingFocusOptions) {
  const { rows } = shape;
  const grid = shape.cols !== undefined;
  const cols = shape.cols ?? 1;
  const [active, setActive] = useState<[number, number]>([0, 0]);
  const [focused, setFocused] = useState(false);
  const elements = useRef(new Map<string, Element>());

  const clamp = (value: number, size: number) => Math.max(0, Math.min(size - 1, value));
  const ai = clamp(active[0], rows);
  const aj = clamp(active[1], cols);

  const moveTo = useCallback((i: number, j: number) => {
    setActive([i, j]);
    const element = elements.current.get(`${i}:${j}`);
    if (element instanceof HTMLElement || element instanceof SVGElement) element.focus();
  }, []);

  const getItemProps = (i: number, j = 0): RovingItemProps => {
    const isActive = i === ai && j === aj;
    return {
      tabIndex: isActive ? 0 : -1,
      ref: (element) => {
        const key = `${i}:${j}`;
        if (element) elements.current.set(key, element);
        else elements.current.delete(key);
      },
      onFocus: () => {
        setFocused(true);
        if (!isActive) setActive([i, j]);
      },
      onBlur: () => setFocused(false),
      onKeyDown: (event) => {
        const meta = event.ctrlKey || event.metaKey;
        let next: [number, number];
        switch (event.key) {
          case 'ArrowRight':
            next = grid ? [i, j + 1] : [i + 1, j];
            break;
          case 'ArrowLeft':
            next = grid ? [i, j - 1] : [i - 1, j];
            break;
          case 'ArrowDown':
            next = [i + 1, j];
            break;
          case 'ArrowUp':
            next = [i - 1, j];
            break;
          case 'Home':
            next = grid ? [meta ? 0 : i, 0] : [0, j];
            break;
          case 'End':
            next = grid ? [meta ? rows - 1 : i, cols - 1] : [rows - 1, j];
            break;
          case 'PageDown':
            next = [i + PAGE, j];
            break;
          case 'PageUp':
            next = [i - PAGE, j];
            break;
          case 'Enter':
          case ' ':
            event.preventDefault();
            onActivate(i, grid ? j : undefined);
            return;
          case 'Escape':
            if (onEscape) {
              event.preventDefault();
              onEscape();
            }
            return;
          default:
            return;
        }
        event.preventDefault();
        moveTo(clamp(next[0], rows), clamp(next[1], cols));
      },
      ...(isActive && focused ? { 'data-focused': true as const } : {}),
    };
  };

  return { getItemProps, active: grid ? ([ai, aj] as const) : ([ai] as const) };
}
