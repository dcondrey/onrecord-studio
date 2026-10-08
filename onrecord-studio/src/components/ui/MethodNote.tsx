import { useId, useState, type ReactNode } from 'react';
import { cn } from '../../utils/cn';
import Icon from './icons';

/**
 * Keeps a panel's basis and caveats one click away instead of above its content. The trigger is
 * inline, so it can follow a sentence and wrap flush left with it.
 */
export default function MethodNote({ children, label = 'How this works' }: { children: ReactNode; label?: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((current) => !current)}
        className="focus-ring inline-flex items-center gap-1 whitespace-nowrap rounded align-baseline text-body text-accent hover:underline"
      >
        {label}
        <Icon name="chevron" size="xs" className={cn('transition-transform', open && 'rotate-90')} />
      </button>
      {open && (
        <div id={id} className="motion-reveal mt-2 rounded-lg border border-line bg-surface/60 px-3 py-2 text-body text-fg-secondary">
          {children}
        </div>
      )}
    </>
  );
}
