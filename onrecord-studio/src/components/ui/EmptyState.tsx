import type { ReactNode } from 'react';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from './icons';

/**
 * An empty surface names what is missing and carries the action that fills it. It is static, so it
 * is not a live region. `compact` drops the dashed container for use inside a bordered panel.
 */
export default function EmptyState({ icon, title, body, actions, compact = false }: {
  icon: IconName;
  title: string;
  body: ReactNode;
  actions?: ReactNode;
  compact?: boolean;
}) {
  return (
    <div className={cn('motion-reveal flex flex-col items-center text-center', compact ? 'px-4 py-6' : 'rounded-xl border border-dashed border-line bg-surface/40 px-6 py-10')}>
      <span className="grid h-10 w-10 place-items-center rounded-xl bg-accent/10 text-accent"><Icon name={icon} size="lg" /></span>
      <h3 className="mt-3 text-heading text-fg">{title}</h3>
      <p className="mt-1 max-w-prose text-body text-muted">{body}</p>
      {actions && <div className="mt-4 flex flex-wrap justify-center gap-2">{actions}</div>}
    </div>
  );
}
