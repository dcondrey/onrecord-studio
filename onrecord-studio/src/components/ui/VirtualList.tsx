import { useCallback, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { cn } from '../../utils/cn';

/** Rows drawn when the container has no measurable height (first paint, or a test DOM). */
const UNMEASURED_ROWS = 30;
const OVERSCAN = 6;

interface VirtualListProps<T> {
  items: readonly T[];
  /** Fixed, so the window is computed from the scroll offset without measuring any row. */
  rowHeight: number;
  getKey: (item: T) => string;
  renderRow: (item: T, row: { index: number; tabIndex: 0 | -1 }) => ReactNode;
  ariaLabel: string;
  /** Called when the window reaches the last rows. The caller owns de-duplication and failure. */
  onNearEnd?: () => void;
  /** Row the roving focus rests on after `resetKey` changes, for example the selected row. */
  initialActiveIndex?: number;
  /** A change returns the scroll offset and focus to the first row, because the rows are a new set. */
  resetKey?: string;
  /** Always laid out after the last row, so a Load more button stays reachable at any scroll depth. */
  footer?: ReactNode;
  className?: string;
}

const NEAR_END_ROWS = 12;

/**
 * Draws only the rows in the scroll window. The focused row stays mounted wherever it is scrolled,
 * and arrow, Home, End and Page keys move focus between rows, because tabbing through tens of
 * thousands of rows is not a way to reach one.
 */
export default function VirtualList<T>({
  items,
  rowHeight,
  getKey,
  renderRow,
  ariaLabel,
  onNearEnd,
  initialActiveIndex = 0,
  resetKey,
  footer,
  className,
}: VirtualListProps<T>) {
  const container = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(0);
  const lastIndex = Math.max(0, items.length - 1);
  const [activeIndex, setActiveIndex] = useState(initialActiveIndex);
  const focusAfterRender = useRef<number | null>(null);
  const nearEnd = useRef(onNearEnd);
  nearEnd.current = onNearEnd;
  const active = Math.min(activeIndex, lastIndex);

  useLayoutEffect(() => {
    const element = container.current;
    if (!element) return;
    setHeight(element.clientHeight);
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => setHeight(element.clientHeight));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (container.current) container.current.scrollTop = 0;
    setScrollTop(0);
    setActiveIndex(initialActiveIndex);
    // `initialActiveIndex` is read only when the row set is replaced.
  }, [resetKey]);

  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - OVERSCAN);
  const last = Math.min(
    items.length,
    height > 0 ? Math.ceil((scrollTop + height) / rowHeight) + OVERSCAN : first + UNMEASURED_ROWS,
  );

  useEffect(() => {
    if (items.length > 0 && last >= items.length - NEAR_END_ROWS) nearEnd.current?.();
  }, [last, items.length]);

  useEffect(() => {
    const target = focusAfterRender.current;
    if (target === null) return;
    focusAfterRender.current = null;
    container.current
      ?.querySelector<HTMLElement>(`[data-row-index="${target}"] button, [data-row-index="${target}"] [tabindex]`)
      ?.focus();
  });

  const moveTo = useCallback(
    (index: number) => {
      const element = container.current;
      if (!element) return;
      const top = index * rowHeight;
      const viewport = element.clientHeight || rowHeight * UNMEASURED_ROWS;
      if (top < element.scrollTop) element.scrollTop = top;
      else if (top + rowHeight > element.scrollTop + viewport) element.scrollTop = top + rowHeight - viewport;
      setScrollTop(element.scrollTop);
      setActiveIndex(index);
      focusAfterRender.current = index;
    },
    [rowHeight],
  );

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const row = (event.target as HTMLElement).closest<HTMLElement>('[data-row-index]');
    if (!row || items.length === 0) return;
    const page = Math.max(1, Math.floor((container.current?.clientHeight || rowHeight * 10) / rowHeight) - 1);
    const current = Number(row.dataset.rowIndex);
    const next =
      event.key === 'ArrowDown' ? current + 1
      : event.key === 'ArrowUp' ? current - 1
      : event.key === 'PageDown' ? current + page
      : event.key === 'PageUp' ? current - page
      : event.key === 'Home' ? 0
      : event.key === 'End' ? lastIndex
      : null;
    if (next === null) return;
    event.preventDefault();
    moveTo(Math.min(lastIndex, Math.max(0, next)));
  };

  const indexes: number[] = [];
  for (let index = first; index < last; index += 1) indexes.push(index);
  if (items.length > 0 && (active < first || active >= last)) indexes.push(active);

  return (
    <div
      ref={container}
      role="list"
      aria-label={ariaLabel}
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
      onKeyDown={onKeyDown}
      className={cn('relative overflow-y-auto', className)}
    >
      <div style={{ height: items.length * rowHeight, position: 'relative' }}>
        {indexes.map((index) => {
          const item = items[index];
          return (
            <div
              key={getKey(item)}
              role="listitem"
              aria-setsize={items.length}
              aria-posinset={index + 1}
              data-row-index={index}
              onFocusCapture={() => setActiveIndex(index)}
              style={{ position: 'absolute', top: index * rowHeight, left: 0, right: 0, height: rowHeight }}
            >
              {renderRow(item, { index, tabIndex: index === active ? 0 : -1 })}
            </div>
          );
        })}
      </div>
      {footer}
    </div>
  );
}
