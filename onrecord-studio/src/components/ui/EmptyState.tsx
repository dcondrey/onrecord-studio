import type { ReactNode } from 'react';
import Icon, { type IconName } from './icons';

/** An empty surface names what is missing and carries the action that fills it. */
export default function EmptyState({ icon, title, body, actions }: { icon: IconName; title: string; body: ReactNode; actions?: ReactNode }) {
  return (
    <div role="status" className="motion-reveal flex flex-col items-center rounded-xl border border-dashed border-line bg-surface/40 px-6 py-10 text-center">
      <span className="grid h-10 w-10 place-items-center rounded-xl bg-accent/10 text-accent"><Icon name={icon} className="h-5 w-5" /></span>
      <h3 className="mt-3 text-sm font-semibold text-fg">{title}</h3>
      <p className="mt-1 max-w-md text-label leading-relaxed text-muted">{body}</p>
      {actions && <div className="mt-4 flex flex-wrap justify-center gap-2">{actions}</div>}
    </div>
  );
}
