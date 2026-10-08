import type { ElementType, HTMLAttributes, ReactNode } from 'react';
import { cn } from '../../utils/cn';

/**
 * A page section. Nesting stops at two levels: a panel may hold `NestedCard`s, and a nested card
 * holds content, never another card.
 */
export default function Panel({ as: Tag = 'section', className, children, ...props }: { as?: ElementType; className?: string; children: ReactNode } & HTMLAttributes<HTMLElement>) {
  return <Tag className={cn('app-panel', className)} {...props}>{children}</Tag>;
}

export function PanelHeader({ eyebrow, title, description, actions, titleId }: {
  /** At most one per panel. */
  eyebrow?: ReactNode;
  title: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  titleId?: string;
}) {
  return (
    <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3 border-b border-line px-5 py-4">
      <div className="min-w-0 flex-1">
        {eyebrow && <p className="mb-1 text-overline uppercase text-muted">{eyebrow}</p>}
        <h3 id={titleId} className="text-heading text-fg">{title}</h3>
        {description && <p className="mt-1 max-w-prose text-caption text-muted">{description}</p>}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}

export function PanelBody({ dense = false, className, children }: { dense?: boolean; className?: string; children: ReactNode }) {
  return <div className={cn(dense ? 'p-4' : 'p-5', className)}>{children}</div>;
}

/** A card inside a panel: the panel's radius minus its padding, and no shadow of its own. */
export function NestedCard({ className, children, ...props }: { className?: string; children: ReactNode } & HTMLAttributes<HTMLDivElement>) {
  return <div className={cn('rounded-lg border border-line bg-surface/50 p-4', className)} {...props}>{children}</div>;
}
