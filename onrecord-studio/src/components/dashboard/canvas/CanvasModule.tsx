import { useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from 'react';
import { cn } from '../../../utils/cn';
import Button from '../../ui/Button';
import Icon, { type IconName } from '../../ui/icons';

/**
 * One module of the overview canvas: a title, the one-sentence headline it leads with, a compact
 * visual, and a detail region it expands into in place (spanning the whole grid row) rather than
 * navigating away. The detail mounts on first expansion and stays mounted, so a collapsed module
 * keeps its drafts, filters and scroll position.
 */
export default function CanvasModule({ id, icon, title, badges, headline, expanded, onToggle, expandLabel, collapseLabel, detail, footer, span, hero = false, className, children }: {
  id: string;
  icon: IconName;
  title: string;
  badges?: ReactNode;
  /** The deterministic sentence the module leads with; null while there is nothing to state. */
  headline: ReactNode;
  expanded: boolean;
  onToggle(): void;
  /** The toggle's accessible names. */
  expandLabel: string;
  collapseLabel: string;
  /** What the module expands into. Without it the module has no toggle. */
  detail?: ReactNode;
  footer?: ReactNode;
  /** Grid placement while collapsed; an expanded module spans the whole row. */
  span?: string;
  /** The canvas's headline module: its sentence is set at the headline size. */
  hero?: boolean;
  className?: string;
  children?: ReactNode;
}) {
  const [everExpanded, setEverExpanded] = useState(expanded);
  if (expanded && !everExpanded) setEverExpanded(true);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const showingDetail = expanded && Boolean(detail);
  const titleId = `${id}-title`;
  const detailId = `${id}-detail`;
  const collapseOnEscape = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key !== 'Escape' || !expanded || event.defaultPrevented) return;
    event.preventDefault();
    onToggle();
    toggleRef.current?.focus();
  };
  return (
    <section
      aria-labelledby={titleId}
      data-canvas-module={id}
      data-expanded={expanded || undefined}
      onKeyDown={collapseOnEscape}
      className={cn('canvas-module flex min-w-0 flex-col', expanded ? 'col-span-full' : span, className)}
      style={{ viewTransitionName: id } as CSSProperties}
    >
      <header className="flex items-center gap-2.5 px-5 pt-4">
        <span className="canvas-module-icon grid h-7 w-7 shrink-0 place-items-center rounded-lg"><Icon name={icon} size="sm" /></span>
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1">
          <h3 id={titleId} className="text-heading text-fg">{title}</h3>
          {badges}
        </div>
        {detail && (
          <Button
            ref={toggleRef}
            variant="ghost"
            size="sm"
            iconOnly
            icon={expanded ? 'shrink' : 'expand'}
            aria-label={expanded ? collapseLabel : expandLabel}
            title={expanded ? collapseLabel : expandLabel}
            aria-expanded={expanded}
            aria-controls={detailId}
            onClick={onToggle}
            className="-mr-1.5 shrink-0 text-muted hover:text-fg"
          />
        )}
      </header>
      {headline && <p className={cn('px-5 pt-2.5 text-fg', hero ? 'text-headline' : 'text-lede')}>{headline}</p>}
      {/* Expanded, the detail replaces the compact face, which restates part of it. The face stays
          mounted, so collapsing returns to it without reloading. */}
      {children && <div className={cn('min-w-0 flex-1 flex-col px-5 pb-5 pt-4', showingDetail ? 'hidden' : 'flex')}>{children}</div>}
      {(!children || showingDetail) && <div className="pb-5" />}
      {footer && <div className={cn('mt-auto border-t border-line px-5 py-3 text-caption text-muted', showingDetail && 'hidden')}>{footer}</div>}
      {detail && everExpanded && (
        <div id={detailId} hidden={!expanded} className="canvas-module-detail border-t border-line px-5 pb-6 pt-5">
          {detail}
        </div>
      )}
    </section>
  );
}
